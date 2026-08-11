use minijinja::{AutoEscape, Environment};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;

use crate::error::TemplateError;
use crate::filters;
use crate::functions;

/// The template rendering engine for Djangors, powered by MiniJinja.
#[derive(Clone)]
pub struct TemplateEngine {
    env: Environment<'static>,
    functions_state: functions::FunctionsState,
}

impl TemplateEngine {
    /// Create a new `TemplateEngine` with search directories checked in order.
    ///
    /// The search directories are searched in the order they are listed.
    /// The first directory that contains the requested template name wins.
    /// This order implements Django's override precedence (project-level
    /// directory listed first overrides app-level templates).
    pub fn new(search_dirs: Vec<PathBuf>) -> Result<Self, TemplateError> {
        let mut env = Environment::new();

        // Configure auto-escaping callback: enable HTML escaping for .html and .htm files
        env.set_auto_escape_callback(|name| {
            if name.ends_with(".html") || name.ends_with(".htm") {
                AutoEscape::Html
            } else {
                AutoEscape::None
            }
        });

        // Set custom template loader using the provided search directories in order
        let dirs = Arc::new(search_dirs);
        let loader_dirs = Arc::clone(&dirs);
        env.set_loader(move |name| {
            for dir in loader_dirs.as_ref() {
                let path = dir.join(name);
                if path.exists() {
                    match std::fs::read_to_string(&path) {
                        Ok(content) => return Ok(Some(content)),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(e) => {
                            return Err(minijinja::Error::new(
                                minijinja::ErrorKind::InvalidOperation,
                                format!("Failed to read template file '{}': {}", path.display(), e),
                            ))
                        }
                    }
                }
            }
            Ok(None)
        });

        // Register the template filters
        env.add_filter("date", filters::date);
        env.add_filter("floatformat", filters::floatformat);
        env.add_filter("pluralize", filters::pluralize);
        env.add_filter("truncatewords", filters::truncatewords);
        env.add_filter("intcomma", filters::intcomma);
        env.add_filter("filesizeformat", filters::filesizeformat);
        env.add_filter("naturaltime", filters::naturaltime);
        env.add_filter("trans", djangors_i18n::trans);

        // Note: 'default' filter is built-in to minijinja and behaves matching Django's
        // default template filter when the value is undefined or falsy.

        let functions_state = functions::FunctionsState::default();
        functions::register_functions(&mut env, functions_state.clone());

        Ok(TemplateEngine {
            env,
            functions_state,
        })
    }

    /// Create a new `TemplateEngine` from templates embedded at compile time
    /// (e.g. via `include_str!`), rather than loaded from the filesystem at
    /// runtime. This is what library crates that ship their own templates
    /// (like `djangors-admin`) should use.
    pub fn from_embedded(
        templates: &[(&'static str, &'static str)],
    ) -> Result<Self, TemplateError> {
        let mut env = Environment::new();

        env.set_auto_escape_callback(|name| {
            if name.ends_with(".html") || name.ends_with(".htm") {
                AutoEscape::Html
            } else {
                AutoEscape::None
            }
        });

        for (name, source) in templates {
            env.add_template(name, source)
                .map_err(TemplateError::MiniJinja)?;
        }

        env.add_filter("date", filters::date);
        env.add_filter("floatformat", filters::floatformat);
        env.add_filter("pluralize", filters::pluralize);
        env.add_filter("truncatewords", filters::truncatewords);
        env.add_filter("intcomma", filters::intcomma);
        env.add_filter("filesizeformat", filters::filesizeformat);
        env.add_filter("naturaltime", filters::naturaltime);
        env.add_filter("trans", djangors_i18n::trans);

        let functions_state = functions::FunctionsState::default();
        functions::register_functions(&mut env, functions_state.clone());

        Ok(TemplateEngine {
            env,
            functions_state,
        })
    }

    /// Supply the resolver backing `{{ url(...) }}`.
    pub fn set_url_resolver<F>(&mut self, f: F)
    where
        F: Fn(&str, &[(String, String)]) -> Option<String> + Send + Sync + 'static,
    {
        *self.functions_state.url_resolver.lock().unwrap() = Some(Arc::new(f));
    }

    /// Prefix for `{{ static(...) }}`. Defaults to "/static/".
    pub fn set_static_url(&mut self, prefix: impl Into<String>) {
        *self.functions_state.static_url.lock().unwrap() = prefix.into();
    }

    /// Render a template by name with a serializable context.
    pub fn render(&self, name: &str, ctx: impl Serialize) -> Result<String, TemplateError> {
        let template = self.env.get_template(name).map_err(|e| {
            if e.kind() == minijinja::ErrorKind::TemplateNotFound {
                // Determine search directories from loader context or fallback to showing search dirs
                // Since loader only has path resolution, we construct a NotFound error.
                TemplateError::NotFound {
                    name: name.to_string(),
                    searched: vec![], // Loader handles search dynamically, so we report empty or can keep it simple.
                }
            } else {
                TemplateError::MiniJinja(e)
            }
        })?;

        let output = template.render(ctx).map_err(TemplateError::MiniJinja)?;
        Ok(output)
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
