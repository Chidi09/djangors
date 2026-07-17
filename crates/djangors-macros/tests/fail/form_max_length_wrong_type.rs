use djangors_forms::Form;

#[derive(Form)]
pub struct InvalidForm {
    #[djangors(max_length = 100)]
    pub age: i64,
}

fn main() {}
