//! XML sitemap generation for Djangors.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use djangors_core::{PathParams, Request, Response, Router, StatusCode};

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// One URL in a generated sitemap.
#[derive(Debug, Clone, PartialEq)]
pub struct SitemapEntry {
    pub loc: String,
    pub lastmod: Option<DateTime<Utc>>,
    pub changefreq: Option<String>,
    pub priority: Option<f32>,
}

/// Supplies sitemap entries for one part of an application.
pub trait Sitemap: Send + Sync {
    fn items(&self) -> Vec<SitemapEntry>;
}

/// Adds `GET /sitemap.xml` to a router.
pub fn sitemap_routes(router: Router, providers: Vec<Arc<dyn Sitemap>>) -> Router {
    router.get("/sitemap.xml", move |_req: Request, _params: PathParams| {
        let providers = providers.clone();
        async move {
            let mut entries = Vec::new();
            for provider in providers {
                entries.extend(provider.items());
            }
            Ok(Response::bytes(
                StatusCode::OK,
                "application/xml; charset=utf-8",
                render_sitemap(&entries).into_bytes(),
            ))
        }
    })
}

/// Renders a standards-shaped XML sitemap document.
pub fn render_sitemap(entries: &[SitemapEntry]) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">"#,
    );
    for entry in entries {
        xml.push_str("<url><loc>");
        xml.push_str(&xml_escape(&entry.loc));
        xml.push_str("</loc>");
        if let Some(lastmod) = entry.lastmod {
            xml.push_str("<lastmod>");
            xml.push_str(&lastmod.to_rfc3339());
            xml.push_str("</lastmod>");
        }
        if let Some(changefreq) = &entry.changefreq {
            xml.push_str("<changefreq>");
            xml.push_str(&xml_escape(changefreq));
            xml.push_str("</changefreq>");
        }
        if let Some(priority) = entry.priority {
            xml.push_str("<priority>");
            xml.push_str(&priority.to_string());
            xml.push_str("</priority>");
        }
        xml.push_str("</url>");
    }
    xml.push_str("</urlset>");
    xml
}

/// Convenience registration for applications that keep contrib setup together.
pub fn register_routes(router: Router, providers: Vec<Arc<dyn Sitemap>>) -> Router {
    sitemap_routes(router, providers)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StaticSitemap(Vec<String>);
    impl Sitemap for StaticSitemap {
        fn items(&self) -> Vec<SitemapEntry> {
            self.0
                .iter()
                .map(|loc| SitemapEntry {
                    loc: loc.clone(),
                    lastmod: None,
                    changefreq: None,
                    priority: None,
                })
                .collect()
        }
    }

    #[test]
    fn renders_expected_escaped_locations() {
        let provider = StaticSitemap(vec!["/about/".into(), "/search?a=1&b=<2".into()]);
        let xml = render_sitemap(&provider.items());
        assert!(xml.starts_with("<?xml version=\"1.0\""));
        assert!(xml.contains("<loc>/about/</loc>"));
        assert!(xml.contains("<loc>/search?a=1&amp;b=&lt;2</loc>"));
        assert!(xml.ends_with("</urlset>"));
    }
}
