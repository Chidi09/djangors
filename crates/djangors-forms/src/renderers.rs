//! Form and field rendering utilities (`as_div`, `as_table`, `as_p`).
//!
//! Encapsulates [`BoundField`] and provides HTML rendering helpers for forms.

use crate::widgets::{html_escape, Widget, WidgetAttrs};

/// A field bound to a name, widget, current value, label, help text, and errors.
pub struct BoundField<'a> {
    /// Field input name in HTML form.
    pub name: &'a str,
    /// Associated form widget.
    pub widget: &'a dyn Widget,
    /// Current submitted or initial value.
    pub value: Option<&'a str>,
    /// Optional field label override (defaults to sentence-cased field name).
    pub label: Option<&'a str>,
    /// Optional help text rendered below or alongside the widget.
    pub help_text: Option<&'a str>,
    /// Validation error messages for this field.
    pub errors: Vec<String>,
    /// Custom widget HTML attributes.
    pub attrs: WidgetAttrs,
}

impl<'a> BoundField<'a> {
    /// Creates a new `BoundField`.
    pub fn new(name: &'a str, widget: &'a dyn Widget) -> Self {
        Self {
            name,
            widget,
            value: None,
            label: None,
            help_text: None,
            errors: Vec::new(),
            attrs: WidgetAttrs::new(),
        }
    }

    /// Sets current value.
    pub fn with_value(mut self, value: Option<&'a str>) -> Self {
        self.value = value;
        self
    }

    /// Sets label text.
    pub fn with_label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Sets help text.
    pub fn with_help_text(mut self, help_text: &'a str) -> Self {
        self.help_text = Some(help_text);
        self
    }

    /// Sets errors.
    pub fn with_errors(mut self, errors: Vec<String>) -> Self {
        self.errors = errors;
        self
    }

    /// Sets widget attributes.
    pub fn with_attrs(mut self, attrs: WidgetAttrs) -> Self {
        self.attrs = attrs;
        self
    }

    /// Returns the effective element ID attribute (`id` in `attrs` or `"id_<name>"`).
    pub fn id_for_label(&self) -> String {
        self.attrs
            .get("id")
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("id_{}", self.name))
    }

    /// Renders the HTML `<label>` tag for this field.
    pub fn label_tag(&self) -> String {
        let label_text = self.label.map(|s| s.to_string()).unwrap_or_else(|| {
            let s = self.name.replace('_', " ");
            let mut chars = s.chars();
            match chars.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
            }
        });
        let esc_id = html_escape(&self.id_for_label());
        let esc_text = html_escape(&label_text);
        format!("<label for=\"{}\">{}</label>", esc_id, esc_text)
    }

    /// Renders the HTML widget for this field.
    pub fn render_widget(&self) -> String {
        self.widget.render(self.name, self.value, &self.attrs)
    }

    /// Renders the field validation errors as an `<ul class="errorlist">`.
    pub fn render_errors(&self) -> String {
        if self.errors.is_empty() {
            String::new()
        } else {
            let mut out = String::from("<ul class=\"errorlist\">");
            for err in &self.errors {
                out.push_str(&format!("<li>{}</li>", html_escape(err)));
            }
            out.push_str("</ul>");
            out
        }
    }

    /// Renders the help text as `<span class="helptext">...</span>`.
    pub fn render_help_text(&self) -> String {
        match self.help_text {
            Some(ht) if !ht.is_empty() => {
                format!("<span class=\"helptext\">{}</span>", html_escape(ht))
            }
            _ => String::new(),
        }
    }
}

/// Renders a list of bound fields and non-field errors as `<div>` containers.
pub fn as_div(fields: &[BoundField], non_field_errors: &[String]) -> String {
    let mut out = String::new();

    if !non_field_errors.is_empty() {
        out.push_str("<ul class=\"errorlist nonfield\">");
        for err in non_field_errors {
            out.push_str(&format!("<li>{}</li>", html_escape(err)));
        }
        out.push_str("</ul>\n");
    }

    for field in fields {
        out.push_str("<div>");
        let errs = field.render_errors();
        if !errs.is_empty() {
            out.push_str(&errs);
        }
        out.push_str(&field.label_tag());
        out.push(' ');
        out.push_str(&field.render_widget());
        let help = field.render_help_text();
        if !help.is_empty() {
            out.push(' ');
            out.push_str(&help);
        }
        out.push_str("</div>\n");
    }

    out
}

/// Renders a list of bound fields and non-field errors as `<p>` paragraph blocks.
pub fn as_p(fields: &[BoundField], non_field_errors: &[String]) -> String {
    let mut out = String::new();

    if !non_field_errors.is_empty() {
        out.push_str("<ul class=\"errorlist nonfield\">");
        for err in non_field_errors {
            out.push_str(&format!("<li>{}</li>", html_escape(err)));
        }
        out.push_str("</ul>\n");
    }

    for field in fields {
        out.push_str("<p>");
        let errs = field.render_errors();
        if !errs.is_empty() {
            out.push_str(&errs);
        }
        out.push_str(&field.label_tag());
        out.push_str(": ");
        out.push_str(&field.render_widget());
        let help = field.render_help_text();
        if !help.is_empty() {
            out.push(' ');
            out.push_str(&help);
        }
        out.push_str("</p>\n");
    }

    out
}

/// Renders a list of bound fields and non-field errors as `<tr>` table rows.
pub fn as_table(fields: &[BoundField], non_field_errors: &[String]) -> String {
    let mut out = String::new();

    if !non_field_errors.is_empty() {
        out.push_str("<tr><td colspan=\"2\"><ul class=\"errorlist nonfield\">");
        for err in non_field_errors {
            out.push_str(&format!("<li>{}</li>", html_escape(err)));
        }
        out.push_str("</ul></td></tr>\n");
    }

    for field in fields {
        out.push_str("<tr><th>");
        out.push_str(&field.label_tag());
        out.push_str(":</th><td>");
        let errs = field.render_errors();
        if !errs.is_empty() {
            out.push_str(&errs);
        }
        out.push_str(&field.render_widget());
        let help = field.render_help_text();
        if !help.is_empty() {
            out.push(' ');
            out.push_str(&help);
        }
        out.push_str("</td></tr>\n");
    }

    out
}
