//! ASPIRATIONAL — nothing in this file exists yet. Target API for Phase 2
//! (djangors-orm, djangors-macros, djangors-migrations). See README.md.

use djangors::prelude::*;

#[derive(Model)]
#[djangors(app = "polls", ordering = "-pub_date")]
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

#[derive(Model)]
#[djangors(app = "polls")]
pub struct Choice {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(foreign_key(to = Question, on_delete = "cascade", related_name = "choices"))]
    pub question: ForeignKey<Question>,

    #[djangors(max_length = 200)]
    pub choice_text: String,

    #[djangors(default = 0)]
    pub votes: i32,
}
