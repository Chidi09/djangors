# How to Internationalize Templates

## Problem
You want to translate text strings in HTML templates based on the user's preferred locale or HTTP `Accept-Language` header.

## Solution
Use `djangors_i18n::Locales` to load Fluent (`.ftl`) catalogs. Pass `locales_value(Arc::new(locales))` and the resolved locale code into the MiniJinja context, and use the `trans` template filter.

## Code Example

```rust,compile
use std::sync::Arc;
use djangors_i18n::{Locales, locales_value, trans};
use minijinja::Environment;

fn render_translated_template(user_locale: &str) -> Result<String, Box<dyn std::error::Error>> {
    // 1. Build translation catalogs using Fluent (.ftl) syntax
    let mut locales = Locales::new("en-US");
    locales.add_locale("en-US", "welcome = Welcome to our site!")?;
    locales.add_locale("fr-FR", "welcome = Bienvenue sur notre site !")?;

    let locales_arc = Arc::new(locales);

    // 2. Register `trans` filter on MiniJinja Environment
    let mut env = Environment::new();
    env.add_filter("trans", trans);
    env.add_template("index.html", r#"<h1>{{ "welcome" | trans(locales, locale) }}</h1>"#)?;

    // 3. Render template with locales_value and active locale code
    let template = env.get_template("index.html")?;
    let rendered = template.render(minijinja::context! {
        locales => locales_value(locales_arc),
        locale => user_locale,
    })?;

    Ok(rendered)
}
```
