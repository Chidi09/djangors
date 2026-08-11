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
        Ok::<_, std::convert::Infallible>(req.extensions().get::<ResolvedLocale>().unwrap().clone())
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
        Ok::<_, std::convert::Infallible>(req.extensions().get::<ResolvedLocale>().unwrap().clone())
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
        Ok::<_, std::convert::Infallible>(req.extensions().get::<ResolvedLocale>().unwrap().clone())
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
