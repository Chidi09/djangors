//! REAL: `Router`, `.get()`/`.post()`, `{name}`/`{name:i64}` path syntax, and
//! `.mount()` all exist today exactly as used below (djangors-core Phase 1).
//!
//! ASPIRATIONAL: named routes (a third argument naming each route, used by
//! `reverse!()` in views.rs) aren't implemented — `PLAN.md`'s Router design
//! explicitly defers "no need for named-route reversing yet, that comes in a
//! later phase" (see the Phase 1 middleware dispatch's router.rs work). Until
//! then, `reverse!("polls:results", question_id)` in views.rs is a stand-in
//! for what will eventually be a real reverse lookup against this file's
//! route table.

use djangors_core::Router;

use crate::views;

pub fn urls() -> Router {
    Router::new()
        .get("/", views::index)
        .get("/{question_id:i64}/", views::detail)
        .get("/{question_id:i64}/results/", views::results)
        .post("/{question_id:i64}/vote/", views::vote)
}
