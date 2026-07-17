use djangors_macros::Model;

#[derive(Model)]
#[djangors(app = "test_app")]
pub struct TwoPrimaryKeys {
    #[djangors(primary_key)]
    pub id1: i64,

    #[djangors(primary_key)]
    pub id2: i64,
}

fn main() {}
