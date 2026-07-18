use minijinja::{AutoEscape, Environment};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;

use crate::error::TemplateError;
use crate::filters;

/// The template rendering engine for Djangors, powered by MiniJinja.
#[derive(Clone)]
pub struct TemplateEngine {
    env: Environment<'static>,
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

        // Note: 'default' filter is built-in to minijinja and behaves matching Django's
        // default template filter when the value is undefined or falsy.

        Ok(TemplateEngine { env })
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

        Ok(TemplateEngine { env })
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
mod tests {
    use super::*;
    use crate::render;
    use djangors_core::StatusCode;
    use serde::Serialize;
    use std::fs;
    use tempfile::TempDir;

    fn create_temp_template_dir() -> TempDir {
        tempfile::Builder::new()
            .prefix("djangors_test_")
            .tempdir()
            .unwrap()
    }

    #[test]
    fn test_loader_precedence() {
        let dir1 = create_temp_template_dir();
        let dir2 = create_temp_template_dir();

        let file_name = "test_template.html";

        fs::write(dir1.path().join(file_name), "dir1 content").unwrap();
        fs::write(dir2.path().join(file_name), "dir2 content").unwrap();

        // With dir1 first, dir1 should win
        let engine1 =
            TemplateEngine::new(vec![dir1.path().to_path_buf(), dir2.path().to_path_buf()])
                .unwrap();
        let rendered1 = engine1.render(file_name, ()).unwrap();
        assert_eq!(rendered1, "dir1 content");

        // With dir2 first, dir2 should win
        let engine2 =
            TemplateEngine::new(vec![dir2.path().to_path_buf(), dir1.path().to_path_buf()])
                .unwrap();
        let rendered2 = engine2.render(file_name, ()).unwrap();
        assert_eq!(rendered2, "dir2 content");
    }

    #[test]
    fn test_template_inheritance() {
        let dir = create_temp_template_dir();

        let base_content = "<html><body>{% block content %}{% endblock %}</body></html>";
        let child_content = "{% extends 'base.html' %}{% block content %}Hello World{% endblock %}";

        fs::write(dir.path().join("base.html"), base_content).unwrap();
        fs::write(dir.path().join("child.html"), child_content).unwrap();

        let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();
        let rendered = engine.render("child.html", ()).unwrap();
        assert_eq!(rendered, "<html><body>Hello World</body></html>");
    }

    #[test]
    fn test_auto_escaping_is_real() {
        let dir = create_temp_template_dir();

        fs::write(dir.path().join("unsafe.html"), "{{ value }}").unwrap();
        fs::write(dir.path().join("unsafe.txt"), "{{ value }}").unwrap();

        let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();

        #[derive(Serialize)]
        struct Context {
            value: &'static str,
        }

        let ctx = Context {
            value: "<script>alert(1)</script>",
        };

        let rendered_html = engine.render("unsafe.html", &ctx).unwrap();
        assert_eq!(rendered_html, "&lt;script&gt;alert(1)&lt;&#x2f;script&gt;");

        let rendered_txt = engine.render("unsafe.txt", &ctx).unwrap();
        assert_eq!(rendered_txt, "<script>alert(1)</script>");
    }

    #[test]
    fn test_filter_pluralize() {
        let dir = create_temp_template_dir();
        fs::write(dir.path().join("plural.html"), "item{{ count|pluralize }}").unwrap();
        fs::write(
            dir.path().join("plural_custom.html"),
            "cherr{{ count|pluralize('y,ies') }}",
        )
        .unwrap();

        let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();

        #[derive(Serialize)]
        struct Context {
            count: i32,
        }

        assert_eq!(
            engine.render("plural.html", &Context { count: 1 }).unwrap(),
            "item"
        );
        assert_eq!(
            engine.render("plural.html", &Context { count: 2 }).unwrap(),
            "items"
        );
        assert_eq!(
            engine.render("plural.html", &Context { count: 0 }).unwrap(),
            "items"
        );

        assert_eq!(
            engine
                .render("plural_custom.html", &Context { count: 1 })
                .unwrap(),
            "cherry"
        );
        assert_eq!(
            engine
                .render("plural_custom.html", &Context { count: 2 })
                .unwrap(),
            "cherries"
        );
    }

    #[test]
    fn test_filter_floatformat() {
        let dir = create_temp_template_dir();
        fs::write(dir.path().join("float.html"), "{{ val|floatformat }}").unwrap();
        fs::write(
            dir.path().join("float_arg.html"),
            "{{ val|floatformat(2) }}",
        )
        .unwrap();
        fs::write(
            dir.path().join("float_neg.html"),
            "{{ val|floatformat(-2) }}",
        )
        .unwrap();

        let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();

        #[derive(Serialize)]
        struct Context {
            val: f64,
        }

        assert_eq!(
            engine
                .render("float.html", &Context { val: 34.232 })
                .unwrap(),
            "34.2"
        );
        assert_eq!(
            engine
                .render("float.html", &Context { val: 34.000 })
                .unwrap(),
            "34"
        );
        assert_eq!(
            engine
                .render("float.html", &Context { val: 34.260 })
                .unwrap(),
            "34.3"
        );

        assert_eq!(
            engine
                .render("float_arg.html", &Context { val: 34.232 })
                .unwrap(),
            "34.23"
        );
        assert_eq!(
            engine
                .render("float_arg.html", &Context { val: 34.000 })
                .unwrap(),
            "34.00"
        );

        assert_eq!(
            engine
                .render("float_neg.html", &Context { val: 34.232 })
                .unwrap(),
            "34.23"
        );
        assert_eq!(
            engine
                .render("float_neg.html", &Context { val: 34.000 })
                .unwrap(),
            "34"
        );
    }

    #[test]
    fn test_filter_truncatewords() {
        let dir = create_temp_template_dir();
        fs::write(dir.path().join("trunc.html"), "{{ val|truncatewords(3) }}").unwrap();

        let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();

        #[derive(Serialize)]
        struct Context {
            val: &'static str,
        }

        assert_eq!(
            engine
                .render(
                    "trunc.html",
                    &Context {
                        val: "one two three four five"
                    }
                )
                .unwrap(),
            "one two three…"
        );
        assert_eq!(
            engine
                .render("trunc.html", &Context { val: "one two" })
                .unwrap(),
            "one two"
        );
    }

    #[test]
    fn test_filter_default() {
        let dir = create_temp_template_dir();
        fs::write(
            dir.path().join("default.html"),
            "{{ val|default('nothing', true) }}",
        )
        .unwrap();

        let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();

        #[derive(Serialize)]
        struct Context1 {
            val: Option<&'static str>,
        }
        assert_eq!(
            engine
                .render("default.html", &Context1 { val: None })
                .unwrap(),
            "nothing"
        );
        assert_eq!(
            engine
                .render(
                    "default.html",
                    &Context1 {
                        val: Some("something")
                    }
                )
                .unwrap(),
            "something"
        );
    }

    #[test]
    fn test_filter_date() {
        let dir = create_temp_template_dir();
        fs::write(
            dir.path().join("date.html"),
            "{{ val|date('Y-m-d H:i:s') }}",
        )
        .unwrap();

        let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();

        #[derive(Serialize)]
        struct Context {
            val: String,
        }

        let ctx = Context {
            val: "2026-07-17T18:02:05+00:00".to_string(),
        };
        assert_eq!(
            engine.render("date.html", &ctx).unwrap(),
            "2026-07-17 18:02:05"
        );
    }

    #[test]
    fn test_render_shortcut() {
        let dir = create_temp_template_dir();
        fs::write(dir.path().join("index.html"), "Hello {{ name }}").unwrap();

        let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();

        #[derive(Serialize)]
        struct Context {
            name: &'static str,
        }

        let resp = render(&engine, "index.html", &Context { name: "Alice" }).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            String::from_utf8(resp.body().to_vec()).unwrap(),
            "Hello Alice"
        );
    }

    #[test]
    fn test_from_embedded_auto_escaping() {
        let engine = TemplateEngine::from_embedded(&[
            ("hello.html", "Hello {{ name }}!"),
            ("hello.txt", "Hello {{ name }}!"),
        ])
        .unwrap();

        #[derive(Serialize)]
        struct Context {
            name: &'static str,
        }

        let ctx = Context {
            name: "<script>alert(1)</script>",
        };

        let rendered_html = engine.render("hello.html", &ctx).unwrap();
        assert_eq!(
            rendered_html,
            "Hello &lt;script&gt;alert(1)&lt;&#x2f;script&gt;!"
        );

        let rendered_txt = engine.render("hello.txt", &ctx).unwrap();
        assert_eq!(rendered_txt, "Hello <script>alert(1)</script>!");
    }

    #[test]
    fn test_missing_template() {
        let engine = TemplateEngine::new(vec![]).unwrap();
        let res = engine.render("does_not_exist.html", ());
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(matches!(err, TemplateError::NotFound { .. }));
    }
}
