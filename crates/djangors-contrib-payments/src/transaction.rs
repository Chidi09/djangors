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
                == Some("23505") =>
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
mod tests {
    use super::*;
    use crate::provider::PaymentProvider;

    const VALID_SIG: &str = "valid_signature";
    const INVALID_SIG: &str = "bad_signature";

    const PAYLOAD_SUCCESS: &str = r#"{"event":"charge.success","data":{"status":"success","reference":"ref_webhook_test","amount":5000}}"#;

    struct DummyPaystackProvider;

    #[async_trait::async_trait]
    impl PaymentProvider for DummyPaystackProvider {
        async fn initiate(
            &self,
            _req: &crate::provider::InitiateChargeRequest,
        ) -> Result<crate::provider::InitiateChargeResponse, PaymentError> {
            unimplemented!()
        }

        async fn verify(
            &self,
            _reference: &str,
        ) -> Result<crate::provider::VerifiedTransaction, PaymentError> {
            unimplemented!()
        }

        fn verify_webhook_signature(&self, _raw_body: &[u8], signature: &str) -> bool {
            signature == VALID_SIG
        }

        async fn refund(
            &self,
            _reference: &str,
            _amount_minor: Option<i64>,
        ) -> Result<crate::provider::RefundResult, PaymentError> {
            unimplemented!()
        }
    }

    const CREATE_TABLE_SQL: &str = "CREATE TABLE IF NOT EXISTS djangors_payments_transaction (
            id BIGSERIAL PRIMARY KEY,
            reference VARCHAR(255) NOT NULL UNIQUE,
            provider VARCHAR(50) NOT NULL,
            amount_minor BIGINT NOT NULL,
            currency VARCHAR(3) NOT NULL,
            status VARCHAR(20) NOT NULL,
            metadata TEXT,
            created_at TIMESTAMPTZ NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL
        )";

    async fn setup_isolated_db() -> Option<djangors_test::TestDatabase> {
        let db = djangors_test::TestDatabase::isolated().await.ok()?;
        db.create_table(CREATE_TABLE_SQL).await.unwrap();
        Some(db)
    }

    #[tokio::test]
    async fn record_charge_initiated_is_idempotent() {
        let Some(db) = setup_isolated_db().await else {
            return;
        };

        let first = record_charge_initiated(
            db.database(),
            "ref_idempotent",
            "paystack",
            5000,
            "NGN",
            None,
        )
        .await
        .unwrap();
        let second = record_charge_initiated(
            db.database(),
            "ref_idempotent",
            "paystack",
            5000,
            "NGN",
            None,
        )
        .await
        .unwrap();

        assert_eq!(
            first.id, second.id,
            "idempotent call must return same row id"
        );

        db.cleanup().await.unwrap();
    }

    #[tokio::test]
    async fn mark_transaction_status_updates_status() {
        let Some(db) = setup_isolated_db().await else {
            return;
        };

        let txn = record_charge_initiated(
            db.database(),
            "ref_status_test",
            "paystack",
            3000,
            "NGN",
            None,
        )
        .await
        .unwrap();
        assert_eq!(txn.status, "pending");

        let updated = mark_transaction_status(
            db.database(),
            "ref_status_test",
            TransactionStatus::Success,
            None,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(updated.status, "success");

        let fetched = find_by_reference(db.database(), "ref_status_test")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.status, "success");
        assert_eq!(fetched.id, txn.id);

        db.cleanup().await.unwrap();
    }

    #[tokio::test]
    async fn handle_paystack_webhook_with_new_reference_and_replay() {
        let Some(db) = setup_isolated_db().await else {
            return;
        };
        let provider = DummyPaystackProvider;

        let result = handle_paystack_webhook(
            &provider,
            db.database(),
            PAYLOAD_SUCCESS.as_bytes(),
            VALID_SIG,
        )
        .await
        .unwrap();
        assert_eq!(result.status, "success");

        let rows = find_by_reference(db.database(), "ref_webhook_test")
            .await
            .unwrap();
        assert!(rows.is_some(), "must exist after first webhook");

        let replay = handle_paystack_webhook(
            &provider,
            db.database(),
            PAYLOAD_SUCCESS.as_bytes(),
            VALID_SIG,
        )
        .await
        .unwrap();
        assert_eq!(replay.status, "success");

        let rows_again = find_by_reference(db.database(), "ref_webhook_test")
            .await
            .unwrap();
        assert!(rows_again.is_some(), "must exist after replay");

        db.cleanup().await.unwrap();
    }

    #[tokio::test]
    async fn handle_paystack_webhook_bad_signature_no_row_created() {
        let Some(db) = setup_isolated_db().await else {
            return;
        };
        let provider = DummyPaystackProvider;

        let err = handle_paystack_webhook(
            &provider,
            db.database(),
            PAYLOAD_SUCCESS.as_bytes(),
            INVALID_SIG,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, PaymentError::InvalidWebhookSignature));

        let rows = find_by_reference(db.database(), "ref_webhook_test")
            .await
            .unwrap();
        assert!(
            rows.is_none(),
            "no row should have been created with invalid signature"
        );

        db.cleanup().await.unwrap();
    }
}
