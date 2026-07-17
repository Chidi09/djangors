use djangors_forms::Form;
use std::collections::HashMap;

#[derive(Form)]
pub struct TestForm {
    #[djangors(max_length = 5)]
    pub short_code: String,

    pub is_accept: bool,
}

fn main() {
    let mut data = HashMap::new();
    data.insert("short_code".to_string(), "too_long_value".to_string());
    // is_accept is boolean, required defaults to true. So leaving it out/absent is false, which is not truthy, causing a required validation error.

    let res = TestForm::clean(&data);
    assert!(res.is_err());
    let errors = res.unwrap_err();

    // Verify errors exist for both fields
    assert!(errors.fields.contains_key("short_code"));
    assert!(errors.fields.contains_key("is_accept"));

    let short_code_err = errors.fields.get("short_code").unwrap().to_string();
    assert!(short_code_err.contains("Ensure this value has at most"));

    let is_accept_err = errors.fields.get("is_accept").unwrap().to_string();
    assert!(is_accept_err.contains("This field is required."));
}
