//! Runtime internationalization for Djangors.
//!
//! v1 deliberately provides runtime catalog loading and lookup only.  Catalogs are hand-written
//! Fluent (`.ftl`) sources; a future `dj makemessages`-style extractor is out of scope here.
//!
//! `LocaleLayer` resolves the first `Accept-Language` tag, or the session's `_locale` override,
//! and inserts [`ResolvedLocale`] into the request extensions.  Template filters cannot access a
//! request, so pass an opaque [`locales_value`] and the resolved locale in the template context:
//!
//! ```ignore
//! let ctx = minijinja::context! { locales => locales_value(std::sync::Arc::new(locales)), locale => "en-US" };
//! // Template: {{ "welcome" | trans(locales, locale) }}
//! ```
//!
//! Date formatting is intentionally a small convention mapping, not a CLDR implementation.

use chrono::{DateTime, NaiveDate, Utc};
use fluent_bundle::{concurrent::FluentBundle, FluentArgs, FluentResource};
use minijinja::{value::Object, Error as MiniJinjaError, ErrorKind as MiniJinjaErrorKind};
use std::{collections::HashMap, fmt, sync::Arc};
use thiserror::Error;
use tower::{Layer, Service};
use unic_langid::LanguageIdentifier;

#[derive(Debug, Error)]
pub enum I18nError {
    #[error("invalid locale '{0}'")]
    InvalidLocale(String),
    #[error("invalid Fluent source: {0:?}")]
    Fluent(Vec<fluent_bundle::FluentError>),
    #[error("invalid Fluent resource: {0}")]
    Resource(String),
}

pub struct Catalog {
    bundle: FluentBundle<FluentResource>,
}

impl Catalog {
    pub fn from_ftl(locale: &str, ftl_source: &str) -> Result<Self, I18nError> {
        let language: LanguageIdentifier = locale
            .parse()
            .map_err(|_| I18nError::InvalidLocale(locale.to_string()))?;
        let resource = FluentResource::try_new(ftl_source.to_string())
            .map_err(|(_, errors)| I18nError::Resource(format!("{errors:?}")))?;
        let mut bundle = FluentBundle::new_concurrent(vec![language]);
        bundle.add_resource(resource).map_err(I18nError::Fluent)?;
        Ok(Self { bundle })
    }

    pub fn get(&self, message_id: &str, args: Option<&FluentArgs<'_>>) -> Option<String> {
        let message = self.bundle.get_message(message_id)?;
        let pattern = message.value()?;
        let mut errors = Vec::new();
        Some(
            self.bundle
                .format_pattern(pattern, args, &mut errors)
                .into_owned(),
        )
    }
}

pub struct Locales {
    default_locale: String,
    catalogs: HashMap<String, Catalog>,
}

impl fmt::Debug for Locales {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Locales")
            .field("default_locale", &self.default_locale)
            .field("catalogs", &self.catalogs.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Locales {
    pub fn new(default_locale: &str) -> Self {
        Self {
            default_locale: default_locale.to_string(),
            catalogs: HashMap::new(),
        }
    }

    pub fn add_locale(&mut self, locale: &str, ftl_source: &str) -> Result<(), I18nError> {
        self.catalogs
            .insert(locale.to_string(), Catalog::from_ftl(locale, ftl_source)?);
        Ok(())
    }

    pub fn translate(
        &self,
        locale: &str,
        message_id: &str,
        args: Option<&FluentArgs<'_>>,
    ) -> String {
        self.catalogs
            .get(locale)
            .and_then(|catalog| catalog.get(message_id, args))
            .or_else(|| {
                self.catalogs
                    .get(&self.default_locale)
                    .and_then(|c| c.get(message_id, args))
            })
            .unwrap_or_else(|| message_id.to_string())
    }

    pub fn default_locale(&self) -> &str {
        &self.default_locale
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedLocale(pub String);

#[derive(Clone)]
pub struct LocaleLayer {
    default_locale: String,
}

impl LocaleLayer {
    pub fn new(default_locale: impl Into<String>) -> Self {
        Self {
            default_locale: default_locale.into(),
        }
    }
}

impl Default for LocaleLayer {
    fn default() -> Self {
        Self::new("en-US")
    }
}

#[derive(Clone)]
pub struct LocaleService<S> {
    inner: S,
    default_locale: String,
}

impl<S> Layer<S> for LocaleLayer {
    type Service = LocaleService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        LocaleService {
            inner,
            default_locale: self.default_locale.clone(),
        }
    }
}

impl<S, B> Service<hyper::Request<B>> for LocaleService<S>
where
    S: Service<hyper::Request<B>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    B: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: hyper::Request<B>) -> Self::Future {
        let mut inner = self.inner.clone();
        let header_locale = req
            .headers()
            .get("accept-language")
            .and_then(|v| v.to_str().ok())
            .and_then(first_locale_tag);
        let session_locale = req
            .extensions()
            .get::<djangors_sessions::Session>()
            .and_then(|s| s.get::<String>("_locale"));
        let locale = header_locale
            .or(session_locale)
            .unwrap_or_else(|| self.default_locale.clone());
        req.extensions_mut().insert(ResolvedLocale(locale));
        Box::pin(async move { inner.call(req).await })
    }
}

pub fn first_locale_tag(header: &str) -> Option<String> {
    let tag = header
        .split(',')
        .next()
        .and_then(|tag| tag.trim().split(';').next())
        .map(str::trim)
        .filter(|tag| !tag.is_empty() && *tag != "*")?;
    tag.parse::<LanguageIdentifier>()
        .ok()
        .map(|_| tag.to_string())
}

#[derive(Clone, Debug)]
pub struct LocalesValue(pub Arc<Locales>);
impl Object for LocalesValue {}
pub fn locales_value(locales: Arc<Locales>) -> minijinja::Value {
    minijinja::Value::from_object(LocalesValue(locales))
}

pub fn trans(
    message_id: minijinja::Value,
    locales: minijinja::Value,
    locale: String,
) -> Result<String, MiniJinjaError> {
    let locales = locales
        .downcast_object_ref::<LocalesValue>()
        .ok_or_else(|| {
            MiniJinjaError::new(
                MiniJinjaErrorKind::InvalidOperation,
                "trans filter expects locales_value(...)",
            )
        })?;
    Ok(locales.0.translate(&locale, &message_id.to_string(), None))
}

pub fn localized_date(date: NaiveDate, locale: &str) -> String {
    let format = match locale {
        "en-US" => "%m/%d/%Y",
        "en-GB" | "en-AU" | "en-NZ" => "%d/%m/%Y",
        _ if locale.starts_with("en-")
            || locale.starts_with("de-")
            || locale.starts_with("fr-") =>
        {
            "%d/%m/%Y"
        }
        _ => "%Y-%m-%d",
    };
    date.format(format).to_string()
}

pub fn localized_datetime(date: DateTime<Utc>, locale: &str) -> String {
    localized_date(date.date_naive(), locale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::Request;
    use tower::{util::service_fn, ServiceExt};

    #[test]
    fn translate_falls_back_to_default_then_id() {
        let mut locales = Locales::new("en-US");
        locales.add_locale("en-US", "hello = Hello").unwrap();
        locales.add_locale("fr-FR", "hello = Bonjour").unwrap();
        assert_eq!(locales.translate("fr-FR", "hello", None), "Bonjour");
        assert_eq!(locales.translate("de-DE", "hello", None), "Hello");
        assert_eq!(locales.translate("de-DE", "missing", None), "missing");
    }

    #[tokio::test]
    async fn layer_reads_header_and_defaults() {
        let service = LocaleLayer::new("en-US").layer(service_fn(|req: Request<()>| async move {
            Ok::<_, std::convert::Infallible>(
                req.extensions().get::<ResolvedLocale>().unwrap().clone(),
            )
        }));
        let response = service
            .oneshot(
                Request::builder()
                    .header("accept-language", "fr-FR, en;q=0.8")
                    .body(())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response, ResolvedLocale("fr-FR".into()));

        let session = djangors_sessions::Session::new_empty();
        session.set("_locale", "de-DE");
        let service = LocaleLayer::new("en-US").layer(service_fn(|req: Request<()>| async move {
            Ok::<_, std::convert::Infallible>(
                req.extensions().get::<ResolvedLocale>().unwrap().clone(),
            )
        }));
        let response = service
            .oneshot(
                Request::builder()
                    .header("accept-language", "not a locale")
                    .extension(session)
                    .body(())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response, ResolvedLocale("de-DE".into()));

        let service = LocaleLayer::new("en-US").layer(service_fn(|req: Request<()>| async move {
            Ok::<_, std::convert::Infallible>(
                req.extensions().get::<ResolvedLocale>().unwrap().clone(),
            )
        }));
        let response = service.oneshot(Request::new(())).await.unwrap();
        assert_eq!(response, ResolvedLocale("en-US".into()));
    }

    #[test]
    fn trans_filter_and_date_are_localized() {
        let mut locales = Locales::new("en-US");
        locales.add_locale("en-US", "hello = Hello").unwrap();
        locales.add_locale("fr-FR", "hello = Bonjour").unwrap();
        let value = locales_value(Arc::new(locales));
        assert_eq!(
            trans("hello".into(), value.clone(), "en-US".into()).unwrap(),
            "Hello"
        );
        assert_eq!(
            trans("hello".into(), value, "fr-FR".into()).unwrap(),
            "Bonjour"
        );
        let date = NaiveDate::from_ymd_opt(2026, 7, 22).unwrap();
        assert_eq!(localized_date(date, "en-US"), "07/22/2026");
        assert_eq!(localized_date(date, "en-GB"), "22/07/2026");
    }
}
