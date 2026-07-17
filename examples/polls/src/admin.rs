//! Admin registration for the polls app — the Djangors equivalent of
//! Django's `admin.py`. Registers each model with the default (no
//! customization) `ModelAdmin`; per-model customization (`list_display`,
//! filters, inlines) is a later Phase 5 feature.

use crate::models::{Choice, Question};
use djangors_admin::AdminSite;

pub fn admin_site() -> AdminSite {
    let site = AdminSite::new();
    site.register::<Question>();
    site.register::<Choice>();
    site
}
