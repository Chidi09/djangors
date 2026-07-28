//! The provider-agnostic payment trait and shared request/response/error types.

/// A request to start a new charge with a payment provider.
pub struct InitiateChargeRequest {
    /// Customer email (required by Paystack and most providers).
    pub email: String,
    /// Amount in the currency's minor unit (kobo for NGN, cents for USD) - never a float.
    pub amount_minor: i64,
    /// ISO 4217 currency code, e.g. "NGN".
    pub currency: String,
    /// Caller-supplied idempotency key / transaction reference.
    pub reference: String,
    /// Optional URL the customer is redirected to after completing payment.
    pub callback_url: Option<String>,
}

/// The result of successfully initiating a charge.
#[derive(Debug)]
pub struct InitiateChargeResponse {
    /// URL to redirect the customer to in order to complete payment.
    pub authorization_url: String,
    /// Provider-issued access code for the transaction.
    pub access_code: String,
    /// The transaction reference (echoes `InitiateChargeRequest::reference`).
    pub reference: String,
}

/// Simple status enum for a verified/recorded transaction. Stored in the `Transaction` model
/// (see the `transaction` module) as a plain string column, since djangors-orm has no native
/// enum/choices field type yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionStatus {
    /// Charge initiated but not yet confirmed.
    Pending,
    /// Charge confirmed successful by the provider.
    Success,
    /// Charge failed or was abandoned.
    Failed,
    /// A previously successful charge was refunded.
    Refunded,
}

impl TransactionStatus {
    /// Returns the canonical lowercase string stored in the database.
    pub fn as_str(&self) -> &'static str {
        match self {
            TransactionStatus::Pending => "pending",
            TransactionStatus::Success => "success",
            TransactionStatus::Failed => "failed",
            TransactionStatus::Refunded => "refunded",
        }
    }
}

impl std::str::FromStr for TransactionStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(TransactionStatus::Pending),
            "success" => Ok(TransactionStatus::Success),
            "failed" => Ok(TransactionStatus::Failed),
            "refunded" => Ok(TransactionStatus::Refunded),
            other => Err(format!("invalid transaction status `{other}`")),
        }
    }
}

impl std::fmt::Display for TransactionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The result of verifying a transaction directly with the provider (never trust a client-side
/// redirect/callback alone - always call `PaymentProvider::verify` server-side before treating a
/// payment as confirmed).
pub struct VerifiedTransaction {
    /// The transaction reference.
    pub reference: String,
    /// The confirmed status.
    pub status: TransactionStatus,
    /// Amount in minor units, as confirmed by the provider.
    pub amount_minor: i64,
    /// ISO 4217 currency code.
    pub currency: String,
    /// Provider's human-readable status message, if any (e.g. Paystack's `gateway_response`).
    pub gateway_response: Option<String>,
}

/// The result of a refund request.
pub struct RefundResult {
    /// The transaction reference that was refunded.
    pub reference: String,
    /// Provider-reported refund status string.
    pub status: String,
}

/// Errors from a payment provider integration.
#[derive(thiserror::Error, Debug)]
pub enum PaymentError {
    /// The underlying HTTP request to the provider failed.
    #[error("payment provider request failed: {0}")]
    Request(#[from] reqwest::Error),
    /// The provider's API returned a non-success response.
    #[error("payment provider API error (status {status}): {message}")]
    Api {
        /// HTTP status code returned.
        status: u16,
        /// Error message from the provider (or the raw response body if unparseable).
        message: String,
    },
    /// The provider's response didn't match the expected shape.
    #[error("unexpected payment provider response: {0}")]
    UnexpectedResponse(String),
    /// A webhook payload's signature was missing or did not match.
    #[error("invalid or missing webhook signature")]
    InvalidWebhookSignature,
}

/// A payment provider capable of initiating charges, verifying them, verifying webhook
/// authenticity, and issuing refunds.
#[async_trait::async_trait]
pub trait PaymentProvider: Send + Sync {
    /// Initiates a new charge. Returns a URL the customer should be redirected to.
    async fn initiate(
        &self,
        req: &InitiateChargeRequest,
    ) -> Result<InitiateChargeResponse, PaymentError>;

    /// Verifies a transaction's status directly with the provider by its reference. Always call
    /// this (or a process signature-verified webhook) before crediting anything - never trust a
    /// client-side "success" redirect alone.
    async fn verify(&self, reference: &str) -> Result<VerifiedTransaction, PaymentError>;

    /// Verifies a webhook payload's signature. `raw_body` MUST be the exact bytes as received on
    /// the wire - never a re-serialized/round-tripped copy, since the signature is computed over
    /// the exact byte sequence the provider sent.
    fn verify_webhook_signature(&self, raw_body: &[u8], signature: &str) -> bool;

    /// Refunds a previously successful transaction, fully if `amount_minor` is `None`, partially
    /// otherwise.
    async fn refund(
        &self,
        reference: &str,
        amount_minor: Option<i64>,
    ) -> Result<RefundResult, PaymentError>;
}
