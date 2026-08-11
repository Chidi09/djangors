use super::*;

#[test]
fn accumulates_and_renders_field_and_non_field_errors() {
    let mut errors = ValidationErrors::new();
    errors.add("title", "must not be blank");
    errors.add("title", "must be under 200 characters");
    errors.add("view_count", "must be positive");
    errors.add_non_field("end must be after start");

    assert!(!errors.is_empty());
    assert_eq!(errors.get("title").unwrap().len(), 2);
    assert_eq!(
        errors.to_json(),
        serde_json::json!({
            "title": ["must not be blank", "must be under 200 characters"],
            "view_count": ["must be positive"],
            "non_field_errors": ["end must be after start"],
        })
    );
}

#[test]
fn empty_errors_convert_to_ok() {
    assert!(ValidationErrors::new().into_result().is_ok());
}

#[test]
fn renders_as_422_api_error_carrying_the_field_map() {
    let mut errors = ValidationErrors::new();
    errors.add("title", "must not be blank");

    let err: DjangorsError = errors.into();
    assert_eq!(err.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err.code(), VALIDATION_ERROR_CODE);
    assert_eq!(
        err.details().unwrap(),
        &serde_json::json!({"title": ["must not be blank"]})
    );
}

#[test]
fn merge_preserves_messages_from_both_sides() {
    let mut a = ValidationErrors::new();
    a.add("title", "too short");
    let mut b = ValidationErrors::new();
    b.add("title", "banned word");
    b.add("slug", "already taken");

    a.merge(b);
    assert_eq!(a.get("title").unwrap().len(), 2);
    assert!(a.contains_key("slug"));
}
