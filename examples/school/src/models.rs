use chrono::{DateTime, Utc};
use djangors_macros::Model;
use djangors_orm::ForeignKey;

#[derive(Model, Debug, Clone)]
#[djangors(app = "school", table_name = "school_student")]
pub struct Student {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(max_length = 100)]
    pub first_name: String,

    #[djangors(max_length = 100)]
    pub last_name: String,

    #[djangors(max_length = 254, unique)]
    pub email: String,

    #[djangors(verbose_name = "date enrolled", db_index)]
    pub enrolled_date: DateTime<Utc>,

    pub is_active: bool,
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "school", table_name = "school_course")]
pub struct Course {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(max_length = 20, unique)]
    pub code: String,

    #[djangors(max_length = 200)]
    pub name: String,

    pub credits: i32,
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "school", table_name = "school_enrollment")]
pub struct Enrollment {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(foreign_key(on_delete = "cascade", related_name = "enrollments"))]
    pub student: ForeignKey<Student>,

    #[djangors(foreign_key(on_delete = "cascade", related_name = "enrollments"))]
    pub course: ForeignKey<Course>,

    #[djangors(verbose_name = "date enrolled", db_index)]
    pub enrolled_on: DateTime<Utc>,

    /// Empty string until a grade is recorded - kept as a plain (non-nullable)
    /// text field rather than `Option<String>` so it works with
    /// `list_editable` (5.6.5 scoped that feature to non-Boolean, non-null
    /// text/numeric fields; this keeps Enrollment usable with it without
    /// fighting that constraint - a blank grade is display-equivalent to "no
    /// grade yet" and is simpler than introducing a nullable text field this
    /// early).
    #[djangors(max_length = 5)]
    pub grade: String,
}
