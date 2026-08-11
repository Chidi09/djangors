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
