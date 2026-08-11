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

/// The DDL has to be built per dialect rather than kept as a `&'static str`: `BIGSERIAL` and
/// `TIMESTAMPTZ` are Postgres spellings, and SQLite needs `INTEGER PRIMARY KEY AUTOINCREMENT`
/// for the id to actually auto-assign. SQLite is permissive enough to *accept* the Postgres
/// text, so the CREATE succeeded and the failure only surfaced later as an insert returning
/// no row. `Dialect` already exposes both spellings — use them.
fn create_table_sql(dialect: djangors_orm::djangors_db::Dialect) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS djangors_payments_transaction (
            id {},
            reference VARCHAR(255) NOT NULL UNIQUE,
            provider VARCHAR(50) NOT NULL,
            amount_minor BIGINT NOT NULL,
            currency VARCHAR(3) NOT NULL,
            status VARCHAR(20) NOT NULL,
            metadata TEXT,
            created_at {} NOT NULL,
            updated_at {} NOT NULL
        )",
        dialect.auto_pk_type(),
        dialect.timestamp_type(),
        dialect.timestamp_type(),
    )
}

async fn setup_isolated_db() -> Option<djangors_test::TestDatabase> {
    let db = djangors_test::TestDatabase::isolated().await.ok()?;
    let sql = create_table_sql(db.database().dialect());
    db.create_table(&sql).await.unwrap();
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
