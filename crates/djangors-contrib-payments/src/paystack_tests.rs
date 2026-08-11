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
