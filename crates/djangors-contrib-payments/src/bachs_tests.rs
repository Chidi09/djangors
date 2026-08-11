use super::*;
use crate::provider::TransactionStatus;

#[test]
fn test_minor_to_decimal_string() {
    assert_eq!(minor_to_decimal_string(123456, "NGN"), "1234.56");
    assert_eq!(minor_to_decimal_string(5, "USD"), "0.05");
    assert_eq!(minor_to_decimal_string(0, "USD"), "0.00");
    assert_eq!(minor_to_decimal_string(1500, "JPY"), "1500");
    assert_eq!(minor_to_decimal_string(1234567, "KWD"), "1234.567");

    // Negative input keeps its sign
    assert_eq!(minor_to_decimal_string(-123456, "NGN"), "-1234.56");
    assert_eq!(minor_to_decimal_string(-5, "USD"), "-0.05");
    assert_eq!(minor_to_decimal_string(-1500, "JPY"), "-1500");
}

#[test]
fn test_decimal_string_to_minor() {
    assert_eq!(decimal_string_to_minor("1234.56", "NGN").unwrap(), 123456);
    assert_eq!(decimal_string_to_minor("0.05", "USD").unwrap(), 5);
    assert_eq!(decimal_string_to_minor("0.00", "USD").unwrap(), 0);
    assert_eq!(decimal_string_to_minor("1500", "JPY").unwrap(), 1500);
    assert_eq!(decimal_string_to_minor("1234.567", "KWD").unwrap(), 1234567);

    // Negative
    assert_eq!(decimal_string_to_minor("-1234.56", "NGN").unwrap(), -123456);

    // Rejections
    assert!(decimal_string_to_minor("", "USD").is_err());
    assert!(decimal_string_to_minor("abc", "USD").is_err());
    assert!(decimal_string_to_minor("1.234", "USD").is_err());
}

#[test]
fn test_customer_name_from_email() {
    assert_eq!(customer_name_from_email("jane@example.com"), "jane");
    assert_eq!(customer_name_from_email("@x.com"), "Customer");
    assert_eq!(customer_name_from_email(""), "Customer");
}

#[test]
fn test_status_mapping() {
    // 1. Charge is present
    let make_charge = |status: &str| {
        Some(BachsCharge {
            charge_id: "ch_123".to_string(),
            payment_id: None,
            status: status.to_string(),
            amount: "10.00".to_string(),
            currency: "USD".to_string(),
            message: None,
            is_refundable: None,
        })
    };

    assert_eq!(
        map_transaction_status(make_charge("succeeded").as_ref(), None, "OPEN"),
        TransactionStatus::Success
    );
    assert_eq!(
        map_transaction_status(make_charge("success").as_ref(), None, "OPEN"),
        TransactionStatus::Success
    );
    assert_eq!(
        map_transaction_status(make_charge("refunded").as_ref(), None, "OPEN"),
        TransactionStatus::Refunded
    );
    assert_eq!(
        map_transaction_status(make_charge("partially_refunded").as_ref(), None, "OPEN"),
        TransactionStatus::Refunded
    );
    assert_eq!(
        map_transaction_status(make_charge("failed").as_ref(), None, "OPEN"),
        TransactionStatus::Failed
    );
    assert_eq!(
        map_transaction_status(make_charge("expired").as_ref(), None, "OPEN"),
        TransactionStatus::Failed
    );
    assert_eq!(
        map_transaction_status(make_charge("cancelled").as_ref(), None, "OPEN"),
        TransactionStatus::Failed
    );
    assert_eq!(
        map_transaction_status(make_charge("canceled").as_ref(), None, "OPEN"),
        TransactionStatus::Failed
    );
    assert_eq!(
        map_transaction_status(make_charge("processing").as_ref(), None, "OPEN"),
        TransactionStatus::Pending
    );

    // 2. Charge is not present, fall back to payment_status
    assert_eq!(
        map_transaction_status(None, Some("succeeded"), "OPEN"),
        TransactionStatus::Success
    );
    assert_eq!(
        map_transaction_status(None, Some("failed"), "OPEN"),
        TransactionStatus::Failed
    );
    assert_eq!(
        map_transaction_status(None, Some("canceled"), "OPEN"),
        TransactionStatus::Failed
    );
    assert_eq!(
        map_transaction_status(None, Some("processing"), "OPEN"),
        TransactionStatus::Pending
    );

    // 3. Charge is not present, payment_status is not present, fall back to checkout status
    assert_eq!(
        map_transaction_status(None, None, "COMPLETED"),
        TransactionStatus::Success
    );
    assert_eq!(
        map_transaction_status(None, None, "EXPIRED"),
        TransactionStatus::Failed
    );
    assert_eq!(
        map_transaction_status(None, None, "CANCELLED"),
        TransactionStatus::Failed
    );
    assert_eq!(
        map_transaction_status(None, None, "OPEN"),
        TransactionStatus::Pending
    );
}

#[test]
fn test_webhook_signature() {
    let provider = BachsProvider::new("test_secret_key");
    let body = b"{\"event_type\":\"collection.succeeded\",\"data\":{}}";
    let timestamp = "1620000000";

    // Expected signature: HMAC-SHA256 hex digest of "{timestamp}.{raw_body}"
    let mut mac = HmacSha256::new_from_slice(b"test_secret_key").unwrap();
    let mut message = timestamp.as_bytes().to_vec();
    message.push(b'.');
    message.extend_from_slice(body);
    mac.update(&message);
    let expected = hex::encode(mac.finalize().into_bytes());

    assert!(provider.verify_webhook_signature_with_timestamp(body, Some(timestamp), &expected));

    // Tampered body
    let tampered = b"{\"event_type\":\"collection.failed\",\"data\":{}}";
    assert!(!provider.verify_webhook_signature_with_timestamp(
        tampered,
        Some(timestamp),
        &expected
    ));

    // Wrong timestamp
    assert!(!provider.verify_webhook_signature_with_timestamp(body, Some("1620000001"), &expected));

    // Timestamp is None
    assert!(!provider.verify_webhook_signature_with_timestamp(body, None, &expected));

    // Plain signature verification returns false unconditionally
    assert!(!provider.verify_webhook_signature(body, &expected));
}
