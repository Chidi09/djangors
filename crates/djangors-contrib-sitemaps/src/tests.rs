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
