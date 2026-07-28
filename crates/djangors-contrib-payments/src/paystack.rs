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
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn test_verify_webhook_signature_known_good() {
        let provider = PaystackProvider::new("test_secret_key");
        let body = b"{\"event\":\"charge.success\",\"data\":{\"reference\":\"ref_123\"}}";

        let mut mac = HmacSha512::new_from_slice(b"test_secret_key").unwrap();
        mac.update(body);
        let expected = hex::encode(mac.finalize().into_bytes());

        assert!(provider.verify_webhook_signature(body, &expected));

        let tampered = b"{\"event\":\"charge.failed\",\"data\":{\"reference\":\"ref_123\"}}";
        assert!(!provider.verify_webhook_signature(tampered, &expected));

        assert!(!provider.verify_webhook_signature(body, "not-valid-hex"));
        assert!(!provider.verify_webhook_signature(body, "gg"));
    }

    #[test]
    fn test_deserialize_amount_flexible() {
        #[derive(Deserialize)]
        struct TestAmount {
            #[serde(deserialize_with = "deserialize_amount")]
            amount: i64,
        }

        let json_num = r#"{"amount": 50000}"#;
        let parsed: TestAmount = serde_json::from_str(json_num).unwrap();
        assert_eq!(parsed.amount, 50000);

        let json_str = r#"{"amount": "50000"}"#;
        let parsed: TestAmount = serde_json::from_str(json_str).unwrap();
        assert_eq!(parsed.amount, 50000);
    }

    async fn setup_initiate_mock(
        mock_server: &MockServer,
        status_code: u16,
        response_body: serde_json::Value,
    ) {
        Mock::given(method("POST"))
            .and(path("/transaction/initialize"))
            .respond_with(ResponseTemplate::new(status_code).set_body_json(response_body))
            .mount(mock_server)
            .await;
    }

    async fn setup_verify_mock(
        mock_server: &MockServer,
        reference: &str,
        status_code: u16,
        response_body: serde_json::Value,
    ) {
        Mock::given(method("GET"))
            .and(path(format!("/transaction/verify/{reference}")))
            .respond_with(ResponseTemplate::new(status_code).set_body_json(response_body))
            .mount(mock_server)
            .await;
    }

    #[tokio::test]
    async fn test_initiate_success() {
        let mock_server = MockServer::start().await;
        let provider = PaystackProvider::with_base_url("test_secret", mock_server.uri());

        setup_initiate_mock(
            &mock_server,
            200,
            serde_json::json!({
                "status": true,
                "message": "Authorization URL created",
                "data": {
                    "authorization_url": "https://paystack.com/authorize/abc123",
                    "access_code": "abc123",
                    "reference": "ref_123"
                }
            }),
        )
        .await;

        let req = InitiateChargeRequest {
            email: "customer@example.com".to_string(),
            amount_minor: 50000,
            currency: "NGN".to_string(),
            reference: "ref_123".to_string(),
            callback_url: Some("https://example.com/callback".to_string()),
        };

        let result = provider.initiate(&req).await.unwrap();
        assert_eq!(
            result.authorization_url,
            "https://paystack.com/authorize/abc123"
        );
        assert_eq!(result.access_code, "abc123");
        assert_eq!(result.reference, "ref_123");
    }

    #[tokio::test]
    async fn test_verify_success_with_numeric_amount() {
        let mock_server = MockServer::start().await;
        let provider = PaystackProvider::with_base_url("test_secret", mock_server.uri());

        setup_verify_mock(
            &mock_server,
            "ref_456",
            200,
            serde_json::json!({
                "status": true,
                "message": "Verification successful",
                "data": {
                    "status": "success",
                    "amount": 50000,
                    "reference": "ref_456",
                    "gateway_response": "Successful",
                    "currency": "NGN"
                }
            }),
        )
        .await;

        let result = provider.verify("ref_456").await.unwrap();
        assert_eq!(result.reference, "ref_456");
        assert_eq!(result.status, TransactionStatus::Success);
        assert_eq!(result.amount_minor, 50000);
        assert_eq!(result.currency, "NGN");
        assert_eq!(result.gateway_response.as_deref(), Some("Successful"));
    }

    #[tokio::test]
    async fn test_verify_status_mapping_failed_and_abandoned() {
        let mock_server = MockServer::start().await;
        let provider = PaystackProvider::with_base_url("test_secret", mock_server.uri());

        setup_verify_mock(
            &mock_server,
            "ref_failed",
            200,
            serde_json::json!({
                "status": true,
                "message": "Verification successful",
                "data": {
                    "status": "failed",
                    "amount": 50000,
                    "reference": "ref_failed",
                    "currency": "NGN"
                }
            }),
        )
        .await;

        let result = provider.verify("ref_failed").await.unwrap();
        assert_eq!(result.status, TransactionStatus::Failed);

        setup_verify_mock(
            &mock_server,
            "ref_abandoned",
            200,
            serde_json::json!({
                "status": true,
                "message": "Verification successful",
                "data": {
                    "status": "abandoned",
                    "amount": 50000,
                    "reference": "ref_abandoned",
                    "currency": "NGN"
                }
            }),
        )
        .await;

        let result = provider.verify("ref_abandoned").await.unwrap();
        assert_eq!(result.status, TransactionStatus::Failed);

        setup_verify_mock(
            &mock_server,
            "ref_pending",
            200,
            serde_json::json!({
                "status": true,
                "message": "Verification successful",
                "data": {
                    "status": "pending",
                    "amount": 50000,
                    "reference": "ref_pending",
                    "currency": "NGN"
                }
            }),
        )
        .await;

        let result = provider.verify("ref_pending").await.unwrap();
        assert_eq!(result.status, TransactionStatus::Pending);
    }

    #[tokio::test]
    async fn test_api_error_response() {
        let mock_server = MockServer::start().await;
        let provider = PaystackProvider::with_base_url("test_secret", mock_server.uri());

        setup_initiate_mock(
            &mock_server,
            200,
            serde_json::json!({
                "status": false,
                "message": "Invalid key"
            }),
        )
        .await;

        let req = InitiateChargeRequest {
            email: "customer@example.com".to_string(),
            amount_minor: 50000,
            currency: "NGN".to_string(),
            reference: "ref_err".to_string(),
            callback_url: None,
        };

        let err = provider.initiate(&req).await.unwrap_err();
        match err {
            PaymentError::Api { status, message } => {
                assert_eq!(status, 200);
                assert_eq!(message, "Invalid key");
            }
            other => panic!("expected Api error, got {other:?}"),
        }
    }
}
