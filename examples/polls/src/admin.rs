//! ASPIRATIONAL — nothing in this file exists yet. Target API for Phase 5
//! (djangors-admin). See README.md. This is the file that should make a
//! Django developer feel most at home: register a model, get a full CRUD
//! back-office with zero hand-written views/templates.

use djangors::prelude::*;

use crate::models::{Choice, Question};

pub fn register(site: &mut AdminSite) {
    site.register::<Question>(
        ModelAdmin::new()
            .list_display(&["question_text", "pub_date", "was_published_recently"])
            .list_filter(&["pub_date"])
            .search_fields(&["question_text"])
            .inlines(&[Inline::<Choice>::tabular().extra(3)]),
    );
}
