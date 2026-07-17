use djangors_forms::Form;

#[derive(Form)]
pub struct InvalidForm {
    #[djangors(email)]
    pub age: i64,
}

fn main() {}
