//! Idempotent transaction recording. `reference` has a real UNIQUE constraint at the DB level (not
//! just an application-level check-then-insert, which would race under concurrent webhook
//! redeliveries or double-clicks) - `record_charge_initiated` treats a unique-violation on that
//! constraint as "already recorded", returning the existing row instead of erroring.

use crate::provider::{PaymentError, PaymentProvider, TransactionStatus};
use djangors_macros::Model;
use djangors_orm::djangors_db::Database;
use djangors_orm::Model as _;
use djangors_orm::OrmError;

/// A recorded payment transaction. `reference` is the idempotency key.
#[derive(Model, Debug, Clone)]
#[djangors(
    app = "djangors_contrib_payments",
    table_name = "djangors_payments_transaction"
)]
pub struct Transaction {
    /// Primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// The idempotency key / provider transaction reference. Has a real DB-level UNIQUE
    /// constraint - never insert without relying on it for concurrency safety.
    #[djangors(unique, max_length = 255)]
    pub reference: String,
    /// Which payment provider this transaction is with, e.g. "paystack".
    #[djangors(max_length = 50)]
    pub provider: String,
    /// Amount in the currency's minor unit (kobo/cents) - never a float.
    pub amount_minor: i64,
    /// ISO 4217 currency code.
    #[djangors(max_length = 3)]
    pub currency: String,
    /// One of "pending" / "success" / "failed" / "refunded" - see `TransactionStatus`.
    #[djangors(max_length = 20)]
    pub status: String,
    /// JSON-serialized raw provider payload/metadata, if any.
    pub metadata: Option<String>,
    /// When this row was first created.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// When this row was last updated.
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Records a newly-initiated charge, or returns the EXISTING row if `reference` was already
/// recorded (a duplicate initiate call, or a webhook redelivery racing an in-flight initiate) -
/// relies on the real UNIQUE constraint on `reference`, not a check-then-insert, which would race.
pub async fn record_charge_initiated(
    db: &Database,
    reference: &str,
    provider: &str,
    amount_minor: i64,
    currency: &str,
    metadata: Option<&serde_json::Value>,
) -> Result<Transaction, PaymentError> {
    let now = chrono::Utc::now();
    let txn = Transaction {
        id: 0,
        reference: reference.to_string(),
        provider: provider.to_string(),
        amount_minor,
        currency: currency.to_string(),
        status: TransactionStatus::Pending.as_str().to_string(),
        metadata: metadata.and_then(|m| serde_json::to_string(m).ok()),
        created_at: now,
        updated_at: now,
    };

    match txn.save(db).await {
        Ok(saved) => Ok(saved),
        Err(OrmError::Query(sqlx_err))
            if sqlx_err
                .as_database_error()
                .and_then(|e| e.code())
                .as_deref()
                .is_some_and(is_unique_violation) =>
        {
            find_by_reference(db, reference).await?.ok_or_else(|| {
                PaymentError::UnexpectedResponse(format!(
                    "unique violation on reference `{reference}` but row not found on lookup"
                ))
            })
        }
        Err(e) => Err(PaymentError::UnexpectedResponse(format!(
            "failed to record transaction: {e}"
        ))),
    }
}

/// Whether a driver-reported error code means "a UNIQUE constraint was violated".
///
/// Every supported backend has to be listed here, not just Postgres: this is what makes
/// `record_charge_initiated` idempotent, so a code that is not recognised turns a concurrent
/// webhook redelivery into a hard error instead of a no-op.
///
/// - `23505` — Postgres `unique_violation`.
/// - `2067` — SQLite `SQLITE_CONSTRAINT_UNIQUE`.
/// - `1555` — SQLite `SQLITE_CONSTRAINT_PRIMARYKEY`, which is what a conflict on an
///   `INTEGER PRIMARY KEY` reports rather than 2067.
fn is_unique_violation(code: &str) -> bool {
    matches!(code, "23505" | "2067" | "1555")
}

/// Looks up a transaction by its reference.
pub async fn find_by_reference(
    db: &Database,
    reference: &str,
) -> Result<Option<Transaction>, PaymentError> {
    Transaction::objects()
        .filter(djangors_orm::q!(reference = reference))
        .map_err(|e| PaymentError::UnexpectedResponse(format!("query build failed: {e}")))?
        .first(db)
        .await
        .map_err(|e| PaymentError::UnexpectedResponse(format!("query failed: {e}")))
}

/// Updates an existing transaction's status (and optionally replaces its metadata), by reference.
/// Returns the updated row, or `Ok(None)` if no row exists for that reference.
pub async fn mark_transaction_status(
    db: &Database,
    reference: &str,
    new_status: TransactionStatus,
    metadata: Option<&serde_json::Value>,
) -> Result<Option<Transaction>, PaymentError> {
    let Some(mut txn) = find_by_reference(db, reference).await? else {
        return Ok(None);
    };
    txn.status = new_status.as_str().to_string();
    if let Some(m) = metadata {
        txn.metadata = serde_json::to_string(m).ok();
    }
    txn.updated_at = chrono::Utc::now();
    // `save()` is INSERT-only (djangors-orm convention) - an already-persisted row must go
    // through `update()`, which matches on the primary key, or this would attempt to re-insert
    // and hit the UNIQUE constraint on `reference`.
    txn.update(db).await.map_err(|e| {
        PaymentError::UnexpectedResponse(format!("failed to update transaction: {e}"))
    })?;
    Ok(Some(txn))
}

/// The JSON body Paystack POSTs to a registered webhook URL for transaction events (a minimal
/// subset - just what's needed to confirm a charge). `amount` here is a plain integer in webhook
/// payloads (unlike the verify endpoint's flexible number-or-string type).
#[derive(serde::Deserialize)]
struct PaystackWebhookEvent {
    event: String,
    data: PaystackWebhookData,
}

#[derive(serde::Deserialize)]
struct PaystackWebhookData {
    status: String,
    reference: String,
    amount: i64,
}

/// Processes a raw incoming Paystack webhook delivery end to end:
/// 1. Verifies the signature against the RAW body bytes BEFORE parsing any JSON - a bad signature
///    is rejected without the payload ever being touched.
/// 2. Parses the JSON body.
/// 3. Only treats the charge as confirmed if `event == "charge.success"` AND
///    `data.status == "success"` (checking only one would be a real security gap).
/// 4. Idempotent: if this reference was already recorded (e.g. by an earlier `initiate` call, or a
///    previous delivery of this same webhook), updates it to Success; a genuine duplicate
///    delivery of an already-Success transaction is a harmless no-op re-save, not an error.
pub async fn handle_paystack_webhook<P: PaymentProvider>(
    provider: &P,
    db: &Database,
    raw_body: &[u8],
    signature: &str,
) -> Result<Transaction, PaymentError> {
    if !provider.verify_webhook_signature(raw_body, signature) {
        return Err(PaymentError::InvalidWebhookSignature);
    }

    let event: PaystackWebhookEvent = serde_json::from_slice(raw_body)
        .map_err(|e| PaymentError::UnexpectedResponse(format!("invalid webhook payload: {e}")))?;

    if event.event != "charge.success" || event.data.status != "success" {
        return Err(PaymentError::UnexpectedResponse(format!(
            "unhandled webhook event `{}` with status `{}`",
            event.event, event.data.status
        )));
    }

    match find_by_reference(db, &event.data.reference).await? {
        Some(_) => {
            mark_transaction_status(db, &event.data.reference, TransactionStatus::Success, None)
                .await?
                .ok_or_else(|| {
                    PaymentError::UnexpectedResponse(
                        "transaction disappeared between lookup and update".to_string(),
                    )
                })
        }
        None => {
            record_charge_initiated(
                db,
                &event.data.reference,
                "paystack",
                event.data.amount,
                "NGN",
                None,
            )
            .await?;
            mark_transaction_status(db, &event.data.reference, TransactionStatus::Success, None)
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
#[path = "transaction_tests.rs"]
mod tests;
