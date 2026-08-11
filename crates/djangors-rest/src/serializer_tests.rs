use super::*;
use djangors_macros::Model as DeriveModel;
use djangors_orm::expr::Value;

#[derive(DeriveModel, Debug, Clone, sqlx::FromRow)]
#[djangors(app = "ser_test", table_name = "ser_test_account")]
pub struct Account {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub email: String,
    pub password: String,
    pub is_admin: bool,
}

fn account() -> Account {
    Account {
        id: 7,
        email: "a@example.com".to_string(),
        password: "hashed".to_string(),
        is_admin: false,
    }
}

#[test]
fn default_serializer_matches_the_legacy_serialize_output() {
    let ser = ModelSerializer::<Account>::default();
    assert_eq!(ser.to_representation(&account()), serialize(&account()));
}

#[test]
fn write_only_fields_are_hidden_from_responses() {
    let ser = ModelSerializer::<Account>::new(FieldSet::all().write_only(&["password"]));
    let out = ser.to_representation(&account());
    assert!(out.get("password").is_none(), "secret leaked: {out}");
    assert_eq!(out["email"], "a@example.com");
}

#[test]
fn read_only_fields_are_rejected_on_write_rather_than_silently_dropped() {
    let ser = ModelSerializer::<Account>::new(FieldSet::all().read_only(&["is_admin"]));
    let body = serde_json::json!({
        "email": "b@example.com",
        "password": "pw",
        "is_admin": true,
    });
    let errors = ser.parse(&body, false).unwrap_err();
    assert_eq!(
        errors.get("is_admin").unwrap(),
        &["field is read-only".to_string()]
    );
}

#[test]
fn only_restricts_both_directions() {
    let ser = ModelSerializer::<Account>::new(FieldSet::only(&["email"]));
    let out = ser.to_representation(&account());
    assert!(out.get("password").is_none());
    assert!(out.get("is_admin").is_none());
    assert_eq!(out["email"], "a@example.com");
}

#[test]
fn full_write_requires_every_writable_field_but_partial_does_not() {
    let ser = ModelSerializer::<Account>::default();
    let partial_body = serde_json::json!({"email": "c@example.com"});

    let errors = ser.parse(&partial_body, false).unwrap_err();
    assert!(errors.contains_key("password"));
    assert!(errors.contains_key("is_admin"));

    let values = ser.parse(&partial_body, true).unwrap();
    assert_eq!(values, vec![("email", Value::Text("c@example.com".into()))]);
}

#[test]
fn object_level_validators_run_and_accumulate() {
    let ser = ModelSerializer::<Account>::default()
        .with_validator(
            |values: &Vec<(&'static str, Value)>, errs: &mut ValidationErrors| {
                let email = values.iter().find(|(name, _)| *name == "email");
                if let Some((_, Value::Text(addr))) = email {
                    if !addr.contains('@') {
                        errs.add("email", "must contain @");
                    }
                }
            },
        )
        .with_validator(
            |values: &Vec<(&'static str, Value)>, errs: &mut ValidationErrors| {
                if values
                    .iter()
                    .any(|(n, v)| *n == "password" && matches!(v, Value::Text(p) if p.len() < 8))
                {
                    errs.add_non_field("password too short for an admin account");
                }
            },
        );

    let body = serde_json::json!({
        "email": "not-an-email",
        "password": "short",
        "is_admin": true,
    });
    let errors = ser.parse(&body, false).unwrap_err();
    // Both validators ran; neither short-circuited the other.
    assert!(errors.contains_key("email"));
    assert_eq!(errors.non_field_errors().len(), 1);
}

#[test]
fn field_set_predicates_cover_exclude_read_only_and_write_only() {
    let fs = FieldSet::all()
        .excluding(&["is_admin"])
        .read_only(&["id"])
        .write_only(&["password"]);

    assert!(!fs.is_readable("is_admin") && !fs.is_writable("is_admin"));
    assert!(fs.is_readable("id") && !fs.is_writable("id"));
    assert!(!fs.is_readable("password") && fs.is_writable("password"));
    assert!(fs.is_readable("email") && fs.is_writable("email"));
}
