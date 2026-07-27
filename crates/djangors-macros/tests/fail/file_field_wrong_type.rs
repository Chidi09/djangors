use djangors_macros::Model;

#[derive(Model)]
#[djangors(app = "test_app")]
pub struct InvalidFileField {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(file_field)]
    pub attachment: i64,
}

fn main() {}
