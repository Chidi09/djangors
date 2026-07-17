use djangors_macros::Model;

#[derive(Model)]
#[djangors(app = "test_app")]
pub struct NoPrimaryKey {
    pub name: String,
}

fn main() {}
