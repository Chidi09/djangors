use djangors_forms::Form;
use std::collections::HashMap;

#[derive(Form)]
pub struct ContactForm {
    #[djangors(max_length = 100)]
    pub name: String,

    #[djangors(required = false, email)]
    pub email: String,

    #[djangors(min = 18, max = 120)]
    pub age: i64,

    pub subscribe: bool,
}

fn main() {
    let mut data = HashMap::new();
    data.insert("name".to_string(), "Alice".to_string());
    data.insert("email".to_string(), "alice@example.com".to_string());
    data.insert("age".to_string(), "30".to_string());
    data.insert("subscribe".to_string(), "true".to_string());

    let cleaned = ContactForm::clean(&data).unwrap();

    assert_eq!(cleaned.name, "Alice");
    assert_eq!(cleaned.email, "alice@example.com");
    assert_eq!(cleaned.age, Some(30));
    assert_eq!(cleaned.subscribe, true);
}
