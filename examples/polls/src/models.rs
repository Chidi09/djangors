use chrono::{DateTime, Utc};
use djangors_macros::Model;
use djangors_orm::ForeignKey;

#[derive(Model, Debug, Clone)]
#[djangors(app = "polls", table_name = "polls_question", ordering = "-pub_date")]
pub struct Question {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(max_length = 200)]
    pub question_text: String,

    #[djangors(verbose_name = "date published", db_index)]
    pub pub_date: DateTime<Utc>,
}

impl Question {
    /// Not a framework feature — an ordinary inherent method, exactly like
    /// Django's `was_published_recently()` on the tutorial's `Question` model.
    /// Proves model structs stay plain Rust structs you can add real methods to.
    pub fn was_published_recently(&self) -> bool {
        self.pub_date > Utc::now() - chrono::Duration::days(1)
    }
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "polls", table_name = "polls_choice")]
pub struct Choice {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(foreign_key(on_delete = "cascade", related_name = "choices"))]
    pub question: ForeignKey<Question>,

    #[djangors(max_length = 200)]
    pub choice_text: String,

    #[djangors(default = 0)]
    pub votes: i32,
}
