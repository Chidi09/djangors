//! FormSets for managing multiple form instances submitted together in an HTTP request.
//!
//! Provides [`FormSet`], management form handling, prefixing (`form-0-title`, ...),
//! deletion tracking, and security caps on `TOTAL_FORMS`.

use std::collections::HashMap;
use std::fmt;

use crate::error::FormErrors;
use crate::widgets::{html_escape, CheckboxInput, HiddenInput, Widget, WidgetAttrs};

/// Error produced during `FormSet` validation or management form demultiplexing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormSetError {
    /// Errors originating from the management form (e.g. missing `TOTAL_FORMS` or exceeding `max_num`).
    pub management_errors: Vec<String>,
    /// Per-form validation errors across the formset.
    pub form_errors: Vec<Option<FormErrors>>,
}

impl fmt::Display for FormSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.management_errors.is_empty() {
            write!(
                f,
                "Management form errors: {}",
                self.management_errors.join(", ")
            )
        } else {
            write!(f, "FormSet validation errors in individual forms")
        }
    }
}

impl std::error::Error for FormSetError {}

/// Cleaned result for an individual form within a [`FormSet`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormSetFormResult<C> {
    /// Typed cleaned data produced by form validation.
    pub cleaned: C,
    /// Whether the `DELETE` checkbox was submitted for this form.
    pub delete: bool,
}

/// A container managing N instances of a form.
#[derive(Debug, Clone)]
pub struct FormSet<F = ()> {
    /// Prefix for form field names in HTTP submissions (default: `"form"`).
    pub prefix: String,
    /// Total number of form instances.
    pub total_forms: usize,
    /// Number of initial pre-populated forms.
    pub initial_forms: usize,
    /// Maximum allowed forms for security (default: `1000`).
    pub max_num: usize,
    /// Whether to render and parse a `DELETE` checkbox field per form.
    pub can_delete: bool,
    _marker: std::marker::PhantomData<F>,
}

impl<F> Default for FormSet<F> {
    fn default() -> Self {
        Self {
            prefix: "form".to_string(),
            total_forms: 0,
            initial_forms: 0,
            max_num: 1000,
            can_delete: false,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<F> FormSet<F> {
    /// Creates a new `FormSet` with default configuration (`prefix = "form"`, `max_num = 1000`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets form prefix (e.g. `"inline_author"`).
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// Sets total and initial form counts for rendering.
    pub fn with_counts(mut self, total: usize, initial: usize) -> Self {
        self.total_forms = total;
        self.initial_forms = initial;
        self
    }

    /// Sets maximum allowed total forms.
    pub fn with_max_num(mut self, max_num: usize) -> Self {
        self.max_num = max_num;
        self
    }

    /// Enables or disables `can_delete` support.
    pub fn with_can_delete(mut self, can_delete: bool) -> Self {
        self.can_delete = can_delete;
        self
    }

    /// Generates field name for form at index `i` (e.g. `"form-0-title"`).
    pub fn add_prefix(&self, index: usize, field_name: &str) -> String {
        format!("{}-{}-{}", self.prefix, index, field_name)
    }

    /// Renders management form hidden inputs (`TOTAL_FORMS` and `INITIAL_FORMS`).
    pub fn render_management_form(&self) -> String {
        let hidden = HiddenInput;
        let total_name = format!("{}-TOTAL_FORMS", self.prefix);
        let initial_name = format!("{}-INITIAL_FORMS", self.prefix);

        let total_html = hidden.render(
            &total_name,
            Some(&self.total_forms.to_string()),
            &WidgetAttrs::new(),
        );
        let initial_html = hidden.render(
            &initial_name,
            Some(&self.initial_forms.to_string()),
            &WidgetAttrs::new(),
        );

        format!("{}\n{}", total_html, initial_html)
    }

    /// Renders `DELETE` checkbox for form at index `i` if `can_delete` is enabled.
    pub fn render_delete_checkbox(&self, index: usize, checked: bool) -> String {
        if !self.can_delete {
            return String::new();
        }
        let name = self.add_prefix(index, "DELETE");
        let esc_name = html_escape(&name);
        let val_str = if checked { Some("on") } else { None };
        let checkbox = CheckboxInput.render(&name, val_str, &WidgetAttrs::new());
        format!("<label for=\"id_{}\">{} Delete</label>", esc_name, checkbox)
    }

    /// Demultiplexes POST submission data into N form maps and cleans each using `clean_fn`.
    ///
    /// Checks `TOTAL_FORMS` against `self.max_num` before allocating form instances to prevent
    /// memory exhaustion attacks.
    pub fn clean_with<C, CleanFn>(
        &self,
        data: &HashMap<String, String>,
        clean_fn: CleanFn,
    ) -> Result<Vec<FormSetFormResult<C>>, FormSetError>
    where
        CleanFn: Fn(&HashMap<String, String>) -> Result<C, FormErrors>,
    {
        let total_key = format!("{}-TOTAL_FORMS", self.prefix);
        let initial_key = format!("{}-INITIAL_FORMS", self.prefix);

        let mut mgmt_errors = Vec::new();

        let total_forms = match data.get(&total_key) {
            Some(s) => match s.parse::<usize>() {
                Ok(n) => n,
                Err(_) => {
                    mgmt_errors
                        .push("TOTAL_FORMS must be a valid non-negative integer.".to_string());
                    0
                }
            },
            None => {
                mgmt_errors.push("ManagementForm missing TOTAL_FORMS field.".to_string());
                0
            }
        };

        let _initial_forms = match data.get(&initial_key) {
            Some(s) => s.parse::<usize>().unwrap_or(0),
            None => 0,
        };

        if total_forms > self.max_num {
            mgmt_errors.push(format!(
                "TOTAL_FORMS ({}) exceeds maximum allowed ({})",
                total_forms, self.max_num
            ));
        }

        if !mgmt_errors.is_empty() {
            return Err(FormSetError {
                management_errors: mgmt_errors,
                form_errors: Vec::new(),
            });
        }

        let mut cleaned_forms = Vec::with_capacity(total_forms);
        let mut form_errors = Vec::with_capacity(total_forms);
        let mut has_errors = false;

        let prefix_dash = format!("{}-", self.prefix);

        for i in 0..total_forms {
            let form_prefix = format!("{}{}-", prefix_dash, i);
            let mut sub_map = HashMap::new();

            for (k, v) in data {
                if let Some(stripped) = k.strip_prefix(&form_prefix) {
                    sub_map.insert(stripped.to_string(), v.clone());
                }
            }

            let delete_checked = if self.can_delete {
                matches!(
                    sub_map.get("DELETE").map(|s| s.as_str()),
                    Some("on" | "true" | "1")
                )
            } else {
                false
            };

            match clean_fn(&sub_map) {
                Ok(cleaned) => {
                    cleaned_forms.push(FormSetFormResult {
                        cleaned,
                        delete: delete_checked,
                    });
                    form_errors.push(None);
                }
                Err(errs) => {
                    has_errors = true;
                    form_errors.push(Some(errs));
                }
            }
        }

        if has_errors {
            Err(FormSetError {
                management_errors: Vec::new(),
                form_errors,
            })
        } else {
            Ok(cleaned_forms)
        }
    }
}
