# Sites, Feeds, and Two-Factor Auth

Three small `djangors-contrib-*` crates cover chatty-but-mechanical web plumbing:
`djangors-contrib-otp` for TOTP two-factor auth, `djangors-contrib-sitemaps` for XML
sitemaps, and `djangors-contrib-syndication` for RSS/Atom feeds.

## Two-Factor Auth (OTP)

`djangors-contrib-otp` is the `django-otp` equivalent and ships TOTP only —
the base32 secret lives on an `OtpDevice` row, and verification is done by
`verify_code` on the caller's side.

```rust,illustrative
use djangors_auth::{Auth, User};
use djangors_contrib_otp::{generate_secret, provisioning_uri, verify_code, OtpDevice};
use djangors_core::extract::FromRequest;
use djangors_core::DjangorsError;
use djangors_orm::ForeignKey;

// Enrollment flow — roughly what your handler code looks like.
let Auth(user) = Auth::<User>::from_request(&req).await?;

let secret = generate_secret();                                   // base32 TOTP secret
let uri = provisioning_uri(&secret, &user.username, "Djangors"); // render as a QR code
//     => otpauth://totp/Djangors:alice?secret=...&issuer=Djangors

// The user scans the QR with their TOTP app, then submits their first 6-digit code.
// Do not trust the device until one code verifies:
if verify_code(&secret, &typed_code) {
    let device = OtpDevice {
        id: 0,
        user: ForeignKey::new(user.id),
        secret,
        confirmed: true, // your policy decision: device is now trusted
    }
    .save(db)
    .await?;
}
```

After enrollment, call `verify_code` again on every sensitive action (transfers,
password changes, new-admin grants) before proceeding:

```rust,illustrative
use djangors_contrib_otp::{OtpDevice, verify_code};
use djangors_orm::q;

if !verify_code(&device.secret, &typed_code) {
    return Err(DjangorsError::Unauthorized("invalid one-time code".into()));
}
```

Use `djangors-auth`'s `Auth<User>` extractor to get the authenticated user id; the
device references it through `ForeignKey<djangors_auth::User>`. The `OtpDevice` model
(columns `id`, `user`, `secret`, `confirmed`) lives in the `djangors_otp_device` table,
which you must create via a migration.

> [!NOTE]
> Scope is deliberately narrow: TOTP only, no WebAuthn/passkeys. The secret is stored in
> plaintext (no reversible encryption convention exists yet); the `confirmed` flag is your
> own policy — this crate never auto-trusts a device.

## XML Sitemaps

`djangors-contrib-sitemaps` is the `django.contrib.sitemaps` equivalent. You implement
the `Sitemap` trait to produce `SitemapEntry` values, and the crate renders the XML:

```rust,illustrative
use djangors_contrib_sitemaps::{Sitemap, SitemapEntry};

struct PostSitemap;

impl Sitemap for PostSitemap {
    fn items(&self) -> Vec<SitemapEntry> {
        Post::objects()
            .filter(djangors_orm::q!(is_published = true))
            .unwrap()
            .all(db)
            .await
            .unwrap()
            .into_iter()
            .map(|post| SitemapEntry {
                loc: format!("/posts/{}/", post.slug),
                lastmod: Some(post.updated_at),
                changefreq: Some("weekly".into()),
                priority: Some(0.8),
            })
            .collect()
    }
}
```

Mount it on a router; `GET /sitemap.xml` will concatenate every provider's entries:

```rust,compile
use djangors_contrib_sitemaps::{sitemap_routes, Sitemap, SitemapEntry};
use djangors_core::Router;
use std::sync::Arc;

struct StaticSitemap;

impl Sitemap for StaticSitemap {
    fn items(&self) -> Vec<SitemapEntry> {
        vec![SitemapEntry {
            loc: "/about/".to_string(),
            lastmod: None,
            changefreq: Some("yearly".to_string()),
            priority: Some(0.3),
        }]
    }
}

pub fn build_router() -> Router {
    // register_routes(router, providers) is an alias of sitemap_routes.
    sitemap_routes(Router::new(), vec![Arc::new(StaticSitemap)])
}
```

`render_sitemap(&entries)` produces the standard document shape:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url>
    <loc>/about/</loc>
    <lastmod>2026-08-05T00:00:00+00:00</lastmod>
    <changefreq>yearly</changefreq>
    <priority>0.3</priority>
  </url>
</urlset>
```

> [!NOTE]
> No database access is needed here — a `Sitemap` is a plain trait returning entries, so a
> provider can read from anything (queryset, filesystem, in-memory list). Unlike most contrib
> features, there is no table to migrate.

## RSS / Atom Feeds

`djangors-contrib-syndication` is the `django.contrib.syndication` equivalent. Implement
`Feed` to describe the channel and its `FeedItem`s, then pick the output format:

```rust,illustrative
use djangors_contrib_syndication::{Feed, FeedItem};

struct BlogFeed;

impl Feed for BlogFeed {
    fn title(&self) -> String {
        "Blog & News".into()
    }

    fn link(&self) -> String {
        "https://example.com/".into()
    }

    fn description(&self) -> String {
        "Latest posts, announcements, and changelogs".into()
    }

    fn items(&self) -> Vec<FeedItem> {
        Post::objects()
            .all(db)
            .await
            .unwrap()
            .into_iter()
            .map(|post| FeedItem {
                title: post.title,
                link: format!("https://example.com/posts/{}", post.slug),
                description: post.excerpt,
                pub_date: Some(post.published_at),
            })
            .collect()
    }
}
```

Mount the feed at a path on your router, choosing `FeedFormat::Rss` or
`FeedFormat::Atom`:

```rust,compile
use djangors_contrib_syndication::{feed_routes, Feed, FeedFormat, FeedItem};
use djangors_core::Router;
use std::sync::Arc;

struct TinyFeed;

impl Feed for TinyFeed {
    fn title(&self) -> String {
        "Tiny".into()
    }

    fn link(&self) -> String {
        "https://example.com/".into()
    }

    fn description(&self) -> String {
        "A tiny feed".into()
    }

    fn items(&self) -> Vec<FeedItem> {
        vec![FeedItem {
            title: "Hello".into(),
            link: "https://example.com/posts/hello".into(),
            description: "First post".into(),
            pub_date: None,
        }]
    }
}

pub fn build_router() -> Router {
    feed_routes(Router::new(), "/feed/", Arc::new(TinyFeed), FeedFormat::Rss)
}
```

`render_rss(&dyn Feed)` emits RSS 2.0, `render_atom(&dyn Feed)` emits Atom; `feed_routes`
wraps whichever you select and serves it as `application/xml`. Expose the same feed in both
formats by mounting it a second time with `FeedFormat::Atom` at another path, or swap
formats with a single query-aware handler.

> [!NOTE]
> Both renderers escape all text fields and use RFC 2822 (`RSS`) / RFC 3339 (`Atom`)
> timestamps from `FeedItem.pub_date`. Matching the `FeedItem.pub_date` URI shape is up to
> you — the crate renders exactly what the trait returns.
