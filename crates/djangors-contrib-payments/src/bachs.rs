use crate::provider::{
    InitiateChargeRequest, InitiateChargeResponse, PaymentError, PaymentProvider, RefundResult,
    TransactionStatus, VerifiedTransaction,
};
use crate::transaction::{
    find_by_reference, mark_transaction_status, record_charge_initiated, Transaction,
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const BACHS_BASE_URL: &str = "https://api.bachs.io";

/// Bachs payment provider implementation.
pub struct BachsProvider {
    client: reqwest::Client,
    secret_key: String,
    base_url: String,
}

impl BachsProvider {
    /// Production: talks to https://api.bachs.io
    pub fn new(secret_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            secret_key: secret_key.into(),
            base_url: BACHS_BASE_URL.to_string(),
        }
    }

    /// Sandbox: talks to https://sandbox-api.bachs.io. Build against this first.
    pub fn sandbox(secret_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            secret_key: secret_key.into(),
            base_url: "https://sandbox-api.bachs.io".to_string(),
        }
    }

    /// Point at an arbitrary base URL (used by the tests).
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
            let message = serde_json::from_str::<BachsErrorBody>(&body)
                .map(|e| e.detail)
                .unwrap_or(body);
            return Err(PaymentError::Api { status, message });
        }
        Ok(response)
    }
}

#[derive(Deserialize)]
struct BachsErrorBody {
    detail: String,
}

#[derive(Serialize)]
struct CustomerField {
    email: String,
    name: String,
}

#[derive(Serialize)]
struct PricingField {
    currency: String,
    amount: String,
    price_type: String,
}

#[derive(Serialize)]
struct CreateCheckoutSessionBody {
    customer: CustomerField,
    pricing: PricingField,
    reference: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    success_url: Option<String>,
}

#[derive(Deserialize)]
struct CreateCheckoutSessionResponse {
    checkout_id: String,
    checkout_url: String,
    /// Always `"open"` on a freshly created session; deserialized for completeness but the
    /// caller learns the real outcome from `verify` or the `collection.succeeded` webhook.
    #[allow(dead_code)]
    status: String,
    reference: Option<String>,
}

#[derive(Deserialize)]
struct VerifyCheckoutSessionResponse {
    checkout_id: String,
    status: String,
    payment_status: Option<String>,
    amount: String,
    currency: String,
    reference: Option<String>,
    charge: Option<BachsCharge>,
}

#[derive(Deserialize)]
struct BachsCharge {
    charge_id: String,
    #[allow(dead_code)]
    payment_id: Option<String>,
    status: String,
    amount: String,
    currency: String,
    message: Option<String>,
    #[allow(dead_code)]
    is_refundable: Option<bool>,
}

#[derive(Serialize)]
struct CreateRefundBody {
    charge_id: String,
    reference: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    amount: Option<String>,
}

#[derive(Deserialize)]
struct CreateRefundResponse {
    #[allow(dead_code)]
    refund_id: String,
    #[allow(dead_code)]
    charge_id: String,
    reference: String,
    status: String,
}

/// Helper function to derive name from email.
fn customer_name_from_email(email: &str) -> String {
    let local_part = email.split('@').next().unwrap_or("");
    if local_part.is_empty() {
        "Customer".to_string()
    } else {
        local_part.to_string()
    }
}

/// Number of decimal places Bachs expects for `currency`.
fn currency_exponent(currency: &str) -> u32 {
    match currency.to_ascii_uppercase().as_str() {
        "BIF" | "CLP" | "DJF" | "GNF" | "ISK" | "JPY" | "KMF" | "KRW" | "PYG" | "RWF" | "UGX"
        | "VND" | "VUV" | "XAF" | "XOF" | "XPF" => 0,
        "BHD" | "IQD" | "JOD" | "KWD" | "LYD" | "OMR" | "TND" => 3,
        _ => 2,
    }
}

/// Convert minor unit representation to decimal string format expected by Bachs API.
fn minor_to_decimal_string(amount_minor: i64, currency: &str) -> String {
    let exponent = currency_exponent(currency);
    if exponent == 0 {
        return amount_minor.to_string();
    }
    let is_negative = amount_minor < 0;
    let abs_amount = amount_minor.abs();
    let divisor = 10_i64.pow(exponent);
    let integer_part = abs_amount / divisor;
    let fractional_part = abs_amount % divisor;
    let sign = if is_negative { "-" } else { "" };
    format!(
        "{sign}{integer_part}.{fractional_part:0width$}",
        width = exponent as usize
    )
}

/// Convert decimal string format from Bachs API to minor unit representation.
fn decimal_string_to_minor(amount: &str, currency: &str) -> Result<i64, PaymentError> {
    if amount.is_empty() {
        return Err(PaymentError::UnexpectedResponse(
            "amount string is empty".to_string(),
        ));
    }
    let exponent = currency_exponent(currency);

    let (is_negative, rest) = if let Some(stripped) = amount.strip_prefix('-') {
        (true, stripped)
    } else {
        (false, amount)
    };

    let parts: Vec<&str> = rest.split('.').collect();
    if parts.len() > 2 {
        return Err(PaymentError::UnexpectedResponse(format!(
            "malformed amount string: `{}`",
            amount
        )));
    }

    let integer_str = parts[0];
    if integer_str.is_empty() || !integer_str.chars().all(|c| c.is_ascii_digit()) {
        return Err(PaymentError::UnexpectedResponse(format!(
            "malformed integer part: `{}`",
            amount
        )));
    }

    let integer_val: i64 = integer_str.parse().map_err(|e| {
        PaymentError::UnexpectedResponse(format!("failed to parse integer part in `{amount}`: {e}"))
    })?;

    let fractional_val = if exponent == 0 {
        if parts.len() > 1 && !parts[1].is_empty() {
            return Err(PaymentError::UnexpectedResponse(format!(
                "currency `{}` expects 0 decimal places, but got fractional part in `{}`",
                currency, amount
            )));
        }
        0
    } else {
        let frac_str = if parts.len() > 1 { parts[1] } else { "" };
        if !frac_str.chars().all(|c| c.is_ascii_digit()) {
            return Err(PaymentError::UnexpectedResponse(format!(
                "malformed fractional part: `{}`",
                amount
            )));
        }
        if frac_str.len() > exponent as usize {
            return Err(PaymentError::UnexpectedResponse(format!(
                "fractional part of `{}` is longer than allowed exponent {}",
                amount, exponent
            )));
        }
        let padded = format!("{:0<width$}", frac_str, width = exponent as usize);
        padded.parse::<i64>().map_err(|e| {
            PaymentError::UnexpectedResponse(format!(
                "failed to parse fractional part in `{amount}`: {e}"
            ))
        })?
    };

    // Checked arithmetic throughout: `amount` comes off the wire, so an absurdly large
    // integer part must surface as an error rather than wrapping (release) or panicking (debug).
    let multiplier = 10_i64.pow(exponent);
    let overflow =
        || PaymentError::UnexpectedResponse(format!("amount `{amount}` is too large to represent"));
    let mut combined = integer_val
        .checked_mul(multiplier)
        .and_then(|scaled| scaled.checked_add(fractional_val))
        .ok_or_else(overflow)?;
    if is_negative {
        combined = -combined;
    }
    Ok(combined)
}

fn map_transaction_status(
    charge: Option<&BachsCharge>,
    payment_status: Option<&str>,
    checkout_status: &str,
) -> TransactionStatus {
    if let Some(c) = charge {
        match c.status.to_lowercase().as_str() {
            "succeeded" | "success" => TransactionStatus::Success,
            "refunded" | "partially_refunded" => TransactionStatus::Refunded,
            "failed" | "expired" | "cancelled" | "canceled" => TransactionStatus::Failed,
            _ => TransactionStatus::Pending,
        }
    } else if let Some(p_status) = payment_status {
        match p_status.to_lowercase().as_str() {
            "succeeded" => TransactionStatus::Success,
            "failed" | "canceled" => TransactionStatus::Failed,
            _ => TransactionStatus::Pending,
        }
    } else {
        match checkout_status.to_uppercase().as_str() {
            "COMPLETED" => TransactionStatus::Success,
            "EXPIRED" | "CANCELLED" => TransactionStatus::Failed,
            _ => TransactionStatus::Pending,
        }
    }
}

#[async_trait::async_trait]
impl PaymentProvider for BachsProvider {
    /// Initiates a new checkout session.
    /// Bachs has no "access code" concept, so `access_code` carries the Bachs `checkout_id`,
    /// and the caller must persist it because `verify` and `refund` need it.
    async fn initiate(
        &self,
        req: &InitiateChargeRequest,
    ) -> Result<InitiateChargeResponse, PaymentError> {
        let name = customer_name_from_email(&req.email);
        let amount_decimal = minor_to_decimal_string(req.amount_minor, &req.currency);
        let body = CreateCheckoutSessionBody {
            customer: CustomerField {
                email: req.email.clone(),
                name,
            },
            pricing: PricingField {
                currency: req.currency.clone(),
                amount: amount_decimal,
                price_type: "fixed".to_string(),
            },
            reference: req.reference.clone(),
            success_url: req.callback_url.clone(),
        };

        let response = self
            .send_and_check(
                self.client
                    .post(format!("{}/v1/checkout-sessions", self.base_url))
                    .header("Authorization", format!("Bearer {}", self.secret_key))
                    .json(&body),
            )
            .await?;

        let payload: CreateCheckoutSessionResponse = response.json().await?;

        Ok(InitiateChargeResponse {
            authorization_url: payload.checkout_url,
            access_code: payload.checkout_id,
            reference: payload.reference.unwrap_or_else(|| req.reference.clone()),
        })
    }

    /// Verifies a transaction's status.
    /// Bachs has NO lookup-by-merchant-reference endpoint. So `verify`'s `reference` argument is
    /// interpreted as the Bachs `checkout_id` (the value returned as `access_code` from `initiate`).
    async fn verify(&self, reference: &str) -> Result<VerifiedTransaction, PaymentError> {
        let response = self
            .send_and_check(
                self.client
                    .get(format!(
                        "{}/v1/checkout-sessions/{}",
                        self.base_url, reference
                    ))
                    .header("Authorization", format!("Bearer {}", self.secret_key)),
            )
            .await?;

        let payload: VerifyCheckoutSessionResponse = response.json().await?;

        let actual_currency = payload
            .charge
            .as_ref()
            .map(|c| c.currency.clone())
            .unwrap_or_else(|| payload.currency.clone());
        let amount_str = payload
            .charge
            .as_ref()
            .map(|c| c.amount.clone())
            .unwrap_or_else(|| payload.amount.clone());
        let amount_minor = decimal_string_to_minor(&amount_str, &actual_currency)?;

        let status = map_transaction_status(
            payload.charge.as_ref(),
            payload.payment_status.as_deref(),
            &payload.status,
        );

        let gateway_response = payload.charge.as_ref().and_then(|c| c.message.clone());

        Ok(VerifiedTransaction {
            reference: payload
                .reference
                .unwrap_or_else(|| payload.checkout_id.clone()),
            status,
            amount_minor,
            currency: actual_currency,
            gateway_response,
        })
    }

    /// Returns false unconditionally because Bachs signatures cannot be verified
    /// without the `X-Bachs-Timestamp` value. Callers must use `verify_webhook_signature_with_timestamp`.
    fn verify_webhook_signature(&self, _raw_body: &[u8], _signature: &str) -> bool {
        false
    }

    fn verify_webhook_signature_with_timestamp(
        &self,
        raw_body: &[u8],
        timestamp: Option<&str>,
        signature: &str,
    ) -> bool {
        let Some(ts) = timestamp else {
            return false;
        };
        let sig_bytes = match hex::decode(signature) {
            Ok(b) => b,
            Err(_) => return false,
        };
        let mut mac = match HmacSha256::new_from_slice(self.secret_key.as_bytes()) {
            Ok(m) => m,
            Err(_) => return false,
        };
        let mut message = ts.as_bytes().to_vec();
        message.push(b'.');
        message.extend_from_slice(raw_body);
        mac.update(&message);
        mac.verify_slice(&sig_bytes).is_ok()
    }

    async fn refund(
        &self,
        reference: &str,
        amount_minor: Option<i64>,
    ) -> Result<RefundResult, PaymentError> {
        // Step 1: GET /v1/checkout-sessions/{reference}
        let verify_response = self
            .send_and_check(
                self.client
                    .get(format!(
                        "{}/v1/checkout-sessions/{}",
                        self.base_url, reference
                    ))
                    .header("Authorization", format!("Bearer {}", self.secret_key)),
            )
            .await?;

        let verify_payload: VerifyCheckoutSessionResponse = verify_response.json().await?;
        let charge = verify_payload.charge.ok_or_else(|| {
            PaymentError::UnexpectedResponse(format!(
                "checkout {} has no charge to refund",
                reference
            ))
        })?;

        let refund_reference = format!("refund_{}", charge.charge_id);

        let amount_decimal = amount_minor.map(|a| minor_to_decimal_string(a, &charge.currency));

        let body = CreateRefundBody {
            charge_id: charge.charge_id,
            reference: refund_reference,
            amount: amount_decimal,
        };

        // Step 2: POST /v1/refunds
        let refund_response = self
            .send_and_check(
                self.client
                    .post(format!("{}/v1/refunds", self.base_url))
                    .header("Authorization", format!("Bearer {}", self.secret_key))
                    .json(&body),
            )
            .await?;

        let refund_payload: CreateRefundResponse = refund_response.json().await?;

        Ok(RefundResult {
            reference: refund_payload.reference,
            status: refund_payload.status,
        })
    }
}

#[derive(Deserialize)]
struct BachsWebhookEvent {
    #[serde(rename = "type")]
    event_type: String,
    data: BachsWebhookData,
}

#[derive(Deserialize)]
struct BachsWebhookData {
    checkout_id: Option<String>,
    reference: Option<String>,
    amount: Option<String>,
    currency: Option<String>,
}

/// Processes a raw incoming Bachs webhook delivery.
pub async fn handle_bachs_webhook<P: PaymentProvider>(
    provider: &P,
    db: &djangors_orm::djangors_db::Database,
    raw_body: &[u8],
    timestamp: Option<&str>,
    signature: &str,
) -> Result<Transaction, PaymentError> {
    if !provider.verify_webhook_signature_with_timestamp(raw_body, timestamp, signature) {
        return Err(PaymentError::InvalidWebhookSignature);
    }

    let event: BachsWebhookEvent = serde_json::from_slice(raw_body)
        .map_err(|e| PaymentError::UnexpectedResponse(format!("invalid webhook payload: {e}")))?;

    if event.event_type != "collection.succeeded" {
        return Err(PaymentError::UnexpectedResponse(format!(
            "unhandled webhook event `{}`",
            event.event_type
        )));
    }

    let reference = event
        .data
        .reference
        .clone()
        .or_else(|| event.data.checkout_id.clone())
        .ok_or_else(|| {
            PaymentError::UnexpectedResponse(
                "missing reference/checkout_id in webhook data".to_string(),
            )
        })?;

    // Never default these. Bachs settles in many currencies, so guessing "USD"/"0.00" for an
    // absent field would silently record a transaction for the wrong amount in the wrong
    // currency — worse than refusing the delivery and letting Bachs retry it.
    let currency = event.data.currency.clone().ok_or_else(|| {
        PaymentError::UnexpectedResponse("missing `currency` in webhook data".to_string())
    })?;
    let amount_str = event.data.amount.clone().ok_or_else(|| {
        PaymentError::UnexpectedResponse("missing `amount` in webhook data".to_string())
    })?;
    let amount_minor = decimal_string_to_minor(&amount_str, &currency)?;

    match find_by_reference(db, &reference).await? {
        Some(_) => mark_transaction_status(db, &reference, TransactionStatus::Success, None)
            .await?
            .ok_or_else(|| {
                PaymentError::UnexpectedResponse(
                    "transaction disappeared between lookup and update".to_string(),
                )
            }),
        None => {
            record_charge_initiated(db, &reference, "bachs", amount_minor, &currency, None).await?;
            mark_transaction_status(db, &reference, TransactionStatus::Success, None)
                .await?
                .ok_or_else(|| {
                    PaymentError::UnexpectedResponse(
                        "transaction disappeared between insert and update".to_string(),
                    )
                })
        }
    }
}

#[cfg(test)]
#[path = "bachs_tests.rs"]
mod tests;
