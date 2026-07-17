use djangors_macros::Model;

#[derive(Model)]
pub struct MissingAppAttr {
    #[djangors(primary_key)]
    pub id: i64,
}

fn main() {}
