#![deny(missing_docs)]
//! Hand-rolled RSS 2.0 and Atom feed generation for Djangors.

use chrono::{DateTime, Utc};
use djangors_core::{PathParams, Request, Response, Router, StatusCode};
use std::sync::Arc;

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Trait defining a syndication feed source.
pub trait Feed: Send + Sync {
    /// Returns the feed title.
    fn title(&self) -> String;
    /// Returns the main feed URL link.
    fn link(&self) -> String;
    /// Returns the feed description or subtitle.
    fn description(&self) -> String;
    /// Returns the list of entries contained in this feed.
    fn items(&self) -> Vec<FeedItem>;
}

/// A single entry in an RSS or Atom syndication feed.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedItem {
    /// The title of the feed entry.
    pub title: String,
    /// The canonical URL link for the entry.
    pub link: String,
    /// The body or summary description of the entry.
    pub description: String,
    /// Optional publication timestamp.
    pub pub_date: Option<DateTime<Utc>>,
}

/// Renders a RSS 2.0 XML document string from the given `Feed`.
pub fn render_rss(feed: &dyn Feed) -> String {
    let mut out = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><rss version="2.0"><channel><title>{}</title><link>{}</link><description>{}</description>"#,
        xml_escape(&feed.title()),
        xml_escape(&feed.link()),
        xml_escape(&feed.description())
    );
    for item in feed.items() {
        out.push_str(&format!(
            "<item><title>{}</title><link>{}</link><description>{}</description>",
            xml_escape(&item.title),
            xml_escape(&item.link),
            xml_escape(&item.description)
        ));
        if let Some(date) = item.pub_date {
            out.push_str(&format!("<pubDate>{}</pubDate>", date.to_rfc2822()));
        }
        out.push_str("</item>");
    }
    out.push_str("</channel></rss>");
    out
}

/// Renders an Atom XML document string from the given `Feed`.
pub fn render_atom(feed: &dyn Feed) -> String {
    let updated = feed
        .items()
        .iter()
        .filter_map(|item| item.pub_date)
        .max()
        .unwrap_or_else(Utc::now);
    let mut out = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><feed xmlns="http://www.w3.org/2005/Atom"><title>{}</title><link href="{}"/><id>{}</id><updated>{}</updated><subtitle>{}</subtitle>"#,
        xml_escape(&feed.title()),
        xml_escape(&feed.link()),
        xml_escape(&feed.link()),
        updated.to_rfc3339(),
        xml_escape(&feed.description())
    );
    for item in feed.items() {
        out.push_str(&format!(
            "<entry><title>{}</title><link href=\"{}\"/><id>{}</id><summary>{}</summary>",
            xml_escape(&item.title),
            xml_escape(&item.link),
            xml_escape(&item.link),
            xml_escape(&item.description)
        ));
        if let Some(date) = item.pub_date {
            out.push_str(&format!("<updated>{}</updated>", date.to_rfc3339()));
        }
        out.push_str("</entry>");
    }
    out.push_str("</feed>");
    out
}

/// Target output XML format for feed generation.
#[derive(Clone, Copy)]
pub enum FeedFormat {
    /// Standard RSS 2.0 format.
    Rss,
    /// Standard Atom format.
    Atom,
}

/// Mounts a feed route at `path` on the given router in the specified `format`.
pub fn feed_routes(router: Router, path: &str, feed: Arc<dyn Feed>, format: FeedFormat) -> Router {
    let path = path.to_owned();
    router.get(&path, move |_req: Request, _params: PathParams| {
        let feed = feed.clone();
        async move {
            let body = match format {
                FeedFormat::Rss => render_rss(feed.as_ref()),
                FeedFormat::Atom => render_atom(feed.as_ref()),
            };
            Ok(Response::bytes(
                StatusCode::OK,
                "application/xml; charset=utf-8",
                body.into_bytes(),
            ))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Example;
    impl Feed for Example {
        fn title(&self) -> String {
            "News & Notes".into()
        }
        fn link(&self) -> String {
            "/news?a=1&b=2".into()
        }
        fn description(&self) -> String {
            "All <the> news".into()
        }
        fn items(&self) -> Vec<FeedItem> {
            vec![
                FeedItem {
                    title: "A & B".into(),
                    link: "/one".into(),
                    description: "<hello> & goodbye".into(),
                    pub_date: None,
                },
                FeedItem {
                    title: "Second".into(),
                    link: "/two".into(),
                    description: "Description".into(),
                    pub_date: None,
                },
            ]
        }
    }
    #[test]
    fn rss_escapes_all_text_fields() {
        let xml = render_rss(&Example);
        assert!(xml.contains("News &amp; Notes"));
        assert!(xml.contains("A &amp; B"));
        assert!(xml.contains("&lt;hello&gt; &amp; goodbye"));
        assert_eq!(xml.matches("<item>").count(), 2);
    }
    #[test]
    fn atom_escapes_all_text_fields() {
        let xml = render_atom(&Example);
        assert!(xml.contains("News &amp; Notes"));
        assert!(xml.contains("A &amp; B"));
        assert!(xml.contains("&lt;hello&gt; &amp; goodbye"));
        assert_eq!(xml.matches("<entry>").count(), 2);
    }
}
