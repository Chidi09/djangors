use crate::provider::{
    InitiateChargeRequest, InitiateChargeResponse, PaymentError, PaymentProvider, RefundResult,
    TransactionStatus, VerifiedTransaction,
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha512;

type HmacSha512 = Hmac<Sha512>;

const PAYSTACK_BASE_URL: &str = "https://api.paystack.co";

/// Paystack payment provider implementation.
pub struct PaystackProvider {
    client: reqwest::Client,
    secret_key: String,
    base_url: String,
}

impl PaystackProvider {
    /// Creates a new `PaystackProvider` pointing at the real Paystack API.
    pub fn new(secret_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            secret_key: secret_key.into(),
            base_url: PAYSTACK_BASE_URL.to_string(),
        }
    }

    /// Creates a new `PaystackProvider` with a custom base URL (for testing with a mock server).
    pub fn with_base_url(secret_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            secret_key: secret_key.into(),
            base_url: base_url.into(),
        }
    }

    async fn send_and_check(
        &self,
        request_builder: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, PaymentError> {
        let response = request_builder.send().await?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<PaystackErrorBody>(&body)
                .map(|e| e.message)
                .unwrap_or(body);
            return Err(PaymentError::Api { status, message });
        }
        Ok(response)
    }
}

#[derive(Deserialize)]
struct PaystackErrorBody {
    message: String,
}

#[derive(Serialize)]
struct InitializeChargeBody {
    email: String,
    amount: i64,
    reference: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    callback_url: Option<String>,
}

#[derive(Deserialize)]
struct PaystackResponse<T> {
    status: bool,
    message: String,
    data: Option<T>,
}

#[derive(Deserialize)]
struct InitializeChargeData {
    authorization_url: String,
    access_code: String,
    reference: String,
}

#[derive(Deserialize)]
struct VerifyResponseData {
    status: String,
    #[serde(deserialize_with = "deserialize_amount")]
    amount: i64,
    currency: Option<String>,
    reference: String,
    gateway_response: Option<String>,
}

#[derive(Deserialize)]
struct RefundResponseData {
    transaction: RefundTransactionRef,
    status: String,
}

#[derive(Deserialize)]
struct RefundTransactionRef {
    reference: String,
}

fn deserialize_amount<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(n) => n
            .as_i64()
            .ok_or_else(|| Error::custom("amount number out of range for i64")),
        serde_json::Value::String(s) => s
            .parse::<i64>()
            .map_err(|_| Error::custom(format!("amount string `{}` is not a valid integer", s))),
        _ => Err(Error::custom("amount must be a number or a string")),
    }
}

#[async_trait::async_trait]
impl PaymentProvider for PaystackProvider {
    async fn initiate(
        &self,
        req: &InitiateChargeRequest,
    ) -> Result<InitiateChargeResponse, PaymentError> {
        let body = InitializeChargeBody {
            email: req.email.clone(),
            amount: req.amount_minor,
            reference: req.reference.clone(),
            callback_url: req.callback_url.clone(),
        };

        let response = self
            .send_and_check(
                self.client
                    .post(format!("{}/transaction/initialize", self.base_url))
                    .header("Authorization", format!("Bearer {}", self.secret_key))
                    .json(&body),
            )
            .await?;

        let payload: PaystackResponse<InitializeChargeData> = response.json().await?;

        if !payload.status {
            return Err(PaymentError::Api {
                status: 200,
                message: payload.message,
            });
        }

        let data = payload.data.ok_or_else(|| {
            PaymentError::UnexpectedResponse(
                "missing data field in Paystack initialize response".to_string(),
            )
        })?;

        Ok(InitiateChargeResponse {
            authorization_url: data.authorization_url,
            access_code: data.access_code,
            reference: data.reference,
        })
    }

    async fn verify(&self, reference: &str) -> Result<VerifiedTransaction, PaymentError> {
        let url = {
            let base = reqwest::Url::parse(&format!("{}/transaction/verify/", self.base_url))
                .map_err(|e| PaymentError::UnexpectedResponse(e.to_string()))?;
            base.join(reference)
                .map_err(|e| PaymentError::UnexpectedResponse(e.to_string()))?
        };

        let response = self
            .send_and_check(
                self.client
                    .get(url)
                    .header("Authorization", format!("Bearer {}", self.secret_key)),
            )
            .await?;

        let payload: PaystackResponse<VerifyResponseData> = response.json().await?;

        if !payload.status {
            return Err(PaymentError::Api {
                status: 200,
                message: payload.message,
            });
        }

        let data = payload.data.ok_or_else(|| {
            PaymentError::UnexpectedResponse(
                "missing data field in Paystack verify response".to_string(),
            )
        })?;

        let status = match data.status.as_str() {
            "success" => TransactionStatus::Success,
            "failed" | "abandoned" => TransactionStatus::Failed,
            _ => TransactionStatus::Pending,
        };

        Ok(VerifiedTransaction {
            reference: data.reference,
            status,
            amount_minor: data.amount,
            currency: data.currency.unwrap_or_default(),
            gateway_response: data.gateway_response,
        })
    }

    fn verify_webhook_signature(&self, raw_body: &[u8], signature: &str) -> bool {
        let sig_bytes = match hex::decode(signature) {
            Ok(b) => b,
            Err(_) => return false,
        };
        let mut mac = match HmacSha512::new_from_slice(self.secret_key.as_bytes()) {
            Ok(m) => m,
            Err(_) => return false,
        };
        mac.update(raw_body);
        mac.verify_slice(&sig_bytes).is_ok()
    }

    /// Written from Paystack's public API reference, not ported from a working reference
    /// implementation like initiate/verify/webhook were - keep error handling maximally
    /// defensive (never panic or unwrap on unexpected shapes).
    async fn refund(
        &self,
        reference: &str,
        amount_minor: Option<i64>,
    ) -> Result<RefundResult, PaymentError> {
        #[derive(Serialize)]
        struct RefundBody<'a> {
            transaction: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            amount: Option<i64>,
        }

        let body = RefundBody {
            transaction: reference,
            amount: amount_minor,
        };

        let response = self
            .send_and_check(
                self.client
                    .post(format!("{}/refund", self.base_url))
                    .header("Authorization", format!("Bearer {}", self.secret_key))
                    .json(&body),
            )
            .await?;

        match response
            .json::<PaystackResponse<RefundResponseData>>()
            .await
        {
            Ok(payload) => {
                if !payload.status {
                    return Err(PaymentError::Api {
                        status: 200,
                        message: payload.message,
                    });
                }
                match payload.data {
                    Some(data) => Ok(RefundResult {
                        reference: data.transaction.reference,
                        status: data.status,
                    }),
                    None => Err(PaymentError::UnexpectedResponse(
                        "missing data field in Paystack refund response".to_string(),
                    )),
                }
            }
            Err(e) => Err(PaymentError::UnexpectedResponse(format!(
                "failed to parse refund response: {e}"
            ))),
        }
    }
}

#[cfg(test)]
#[path = "paystack_tests.rs"]
mod tests;
