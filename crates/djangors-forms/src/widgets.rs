//! HTML form widget rendering layer.
//!
//! Provides the [`Widget`] trait and built-in widget implementations for rendering
//! HTML form controls. All string inputs (field names, values, option labels, and
//! attributes) are HTML-escaped using [`html_escape`].

use std::collections::BTreeMap;

/// Escapes HTML special characters in string input.
///
/// Converts `<`, `>`, `&`, `"`, `'`, and `/` into HTML entities.
///
/// # Why this duplicates `djangors_core::html_escape`
///
/// It is character-for-character identical to the one in `djangors-core`, and that is
/// deliberate rather than an oversight. `djangors-forms` **cannot** depend on
/// `djangors-core`: the dependency chain runs `djangors-core` -> `djangors-orm` ->
/// `djangors-forms`, so importing core here would close a cycle and the workspace would
/// stop building.
///
/// If you are tempted to unify these, the only safe route is to extract escaping into a
/// third crate that both depend on. Do not "simplify" this by adding a core dependency.
///
/// Both copies are security-relevant: every widget below reflects previously-submitted
/// values back into HTML attributes, so a change to one must be mirrored in the other.
pub fn html_escape(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#x27;"),
            '/' => escaped.push_str("&#x2F;"),
            _ => escaped.push(c),
        }
    }
    escaped
}

/// Attributes applied to HTML widget elements (e.g. `class`, `id`, `placeholder`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WidgetAttrs {
    map: BTreeMap<String, String>,
}

impl WidgetAttrs {
    /// Creates a new, empty set of widget attributes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets an attribute key and value.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.map.insert(key.into(), value.into());
    }

    /// Gets an attribute value by key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(|s| s.as_str())
    }

    /// Removes an attribute by key.
    pub fn remove(&mut self, key: &str) -> Option<String> {
        self.map.remove(key)
    }

    /// Checks if the attribute map is empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Iterates over key-value attribute pairs.
    pub fn iter(&self) -> std::collections::btree_map::Iter<'_, String, String> {
        self.map.iter()
    }
}

impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for WidgetAttrs {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        let mut attrs = WidgetAttrs::new();
        for (k, v) in iter {
            attrs.set(k, v);
        }
        attrs
    }
}

impl<K: Into<String>, V: Into<String>, const N: usize> From<[(K, V); N]> for WidgetAttrs {
    fn from(arr: [(K, V); N]) -> Self {
        arr.into_iter().collect()
    }
}

/// Trait implemented by form widgets that render as HTML controls.
pub trait Widget: Send + Sync {
    /// Render this widget as HTML for the given field name and current value.
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String;
}

fn render_attrs(name: &str, attrs: &WidgetAttrs) -> String {
    let mut out = String::new();

    let id_already_present = attrs.get("id").is_some();
    if !id_already_present && !name.is_empty() {
        let default_id = format!("id_{}", name);
        out.push_str(&format!(" id=\"{}\"", html_escape(&default_id)));
    }

    for (k, v) in attrs.iter() {
        out.push_str(&format!(" {}=\"{}\"", html_escape(k), html_escape(v)));
    }

    out
}

/// Single-line plain text input (`<input type="text">`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInput;

impl Widget for TextInput {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let val_attr = match value {
            Some(val) => format!(" value=\"{}\"", html_escape(val)),
            None => String::new(),
        };
        format!(
            "<input type=\"text\" name=\"{}\"{}{}>",
            esc_name, val_attr, attrs_str
        )
    }
}

/// Multiline text area (`<textarea>`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Textarea;

impl Widget for Textarea {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let val_content = value.map(html_escape).unwrap_or_default();
        format!(
            "<textarea name=\"{}\"{}>{}</textarea>",
            esc_name, attrs_str, val_content
        )
    }
}

/// Numeric input widget (`<input type="number">`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NumberInput;

impl Widget for NumberInput {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let val_attr = match value {
            Some(val) => format!(" value=\"{}\"", html_escape(val)),
            None => String::new(),
        };
        format!(
            "<input type=\"number\" name=\"{}\"{}{}>",
            esc_name, val_attr, attrs_str
        )
    }
}

/// Checkbox widget (`<input type="checkbox">`).
///
/// Evaluates boolean-ish truthiness (`"true"`, `"on"`, `"1"`, `"checked"`) to render
/// the boolean `checked` attribute. Does not emit `checked="false"`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckboxInput;

impl Widget for CheckboxInput {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let is_checked = matches!(
            value.map(|s| s.trim()),
            Some("true" | "on" | "1" | "checked")
        );
        let checked_str = if is_checked { " checked" } else { "" };
        format!(
            "<input type=\"checkbox\" name=\"{}\"{}{}>",
            esc_name, checked_str, attrs_str
        )
    }
}

/// Dropdown select list (`<select>`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Select {
    /// Vector of (option_value, option_label) pairs.
    pub choices: Vec<(String, String)>,
}

impl Select {
    /// Creates a new `Select` widget with choices.
    pub fn new(choices: Vec<(String, String)>) -> Self {
        Self { choices }
    }
}

impl Widget for Select {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let mut options_html = String::new();

        for (val, label) in &self.choices {
            let esc_val = html_escape(val);
            let esc_label = html_escape(label);
            let selected_str = if value == Some(val.as_str()) {
                " selected"
            } else {
                ""
            };
            options_html.push_str(&format!(
                "<option value=\"{}\"{}>{}</option>",
                esc_val, selected_str, esc_label
            ));
        }

        format!(
            "<select name=\"{}\"{}>{}</select>",
            esc_name, attrs_str, options_html
        )
    }
}

/// Radio button list (`<input type="radio">`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RadioSelect {
    /// Vector of (option_value, option_label) pairs.
    pub choices: Vec<(String, String)>,
}

impl RadioSelect {
    /// Creates a new `RadioSelect` widget with choices.
    pub fn new(choices: Vec<(String, String)>) -> Self {
        Self { choices }
    }
}

impl Widget for RadioSelect {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let base_id = attrs
            .get("id")
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("id_{}", name));
        let esc_base_id = html_escape(&base_id);

        let mut items_html = String::new();
        for (i, (val, label)) in self.choices.iter().enumerate() {
            let esc_val = html_escape(val);
            let esc_label = html_escape(label);
            let opt_id = format!("{}_{}", base_id, i);
            let esc_opt_id = html_escape(&opt_id);
            let checked_str = if value == Some(val.as_str()) {
                " checked"
            } else {
                ""
            };

            items_html.push_str(&format!(
                "<div><label for=\"{}\"><input type=\"radio\" name=\"{}\" value=\"{}\" id=\"{}\"{}> {}</label></div>",
                esc_opt_id, esc_name, esc_val, esc_opt_id, checked_str, esc_label
            ));
        }

        format!("<div id=\"{}\">{}</div>", esc_base_id, items_html)
    }
}

/// Date input widget (`<input type="date">`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DateInput;

impl Widget for DateInput {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let val_attr = match value {
            Some(val) => format!(" value=\"{}\"", html_escape(val)),
            None => String::new(),
        };
        format!(
            "<input type=\"date\" name=\"{}\"{}{}>",
            esc_name, val_attr, attrs_str
        )
    }
}

/// Email input widget (`<input type="email">`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmailInput;

impl Widget for EmailInput {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let val_attr = match value {
            Some(val) => format!(" value=\"{}\"", html_escape(val)),
            None => String::new(),
        };
        format!(
            "<input type=\"email\" name=\"{}\"{}{}>",
            esc_name, val_attr, attrs_str
        )
    }
}

/// Password input widget (`<input type="password">`).
///
/// **Security Notice:** By default, `PasswordInput` does NOT render the submitted
/// password back into the `value` attribute (`render_value = false`). This prevents
/// sensitive cleartext passwords from being exposed in rendered HTML forms or cached responses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PasswordInput {
    /// Whether to render the password value into the `value` attribute.
    /// Defaults to `false` for security.
    pub render_value: bool,
}

impl PasswordInput {
    /// Creates a new `PasswordInput` widget with default secure settings (`render_value = false`).
    pub fn new() -> Self {
        Self::default()
    }
}

impl Widget for PasswordInput {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let val_attr = if self.render_value {
            match value {
                Some(val) => format!(" value=\"{}\"", html_escape(val)),
                None => String::new(),
            }
        } else {
            String::new()
        };
        format!(
            "<input type=\"password\" name=\"{}\"{}{}>",
            esc_name, val_attr, attrs_str
        )
    }
}

/// Hidden input widget (`<input type="hidden">`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HiddenInput;

impl Widget for HiddenInput {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let val_attr = match value {
            Some(val) => format!(" value=\"{}\"", html_escape(val)),
            None => String::new(),
        };
        format!(
            "<input type=\"hidden\" name=\"{}\"{}{}>",
            esc_name, val_attr, attrs_str
        )
    }
}
