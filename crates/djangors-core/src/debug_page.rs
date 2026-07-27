use crate::error::DjangorsError;
use crate::html_escape;
use crate::request::Request;
use crate::response::Response;
use hyper::StatusCode;

/// Renders a rich HTML debug error page for development mode.
///
/// This matches Django's yellow debug screen behavior, showing details about the
/// exception/panic, the request path, and request headers.
///
/// # Security
///
/// This page contains sensitive details and must only be served when `debug = true`.
#[doc(hidden)]
pub fn render_debug_page(error: &DjangorsError, req: &Request) -> Response {
    let status = error.status_code();

    let (badge_class, error_type, error_details) = match error {
        DjangorsError::Panicked(msg) => ("panicked", "Handler Panicked", msg.as_str()),
        DjangorsError::Internal(msg) => ("internal", "Internal Server Error", msg.as_str()),
        DjangorsError::BadRequest(msg) => ("bad-request", "Bad Request", msg.as_str()),
        DjangorsError::NotFound => (
            "not-found",
            "Not Found",
            "The requested URL was not found on this server.",
        ),
        DjangorsError::Unauthorized(msg) => ("unauthorized", "Unauthorized", msg.as_str()),
        DjangorsError::Forbidden(msg) => ("forbidden", "Forbidden", msg.as_str()),
        DjangorsError::TooManyRequests(msg) => {
            ("too-many-requests", "Too Many Requests", msg.as_str())
        }
    };

    let mut headers_html = String::new();
    for (name, value) in req.headers().iter() {
        let val_str = value.to_str().unwrap_or("[invalid utf-8]");
        headers_html.push_str(&format!(
            "<tr><th>{}</th><td>{}</td></tr>\n",
            html_escape(name.as_str()),
            html_escape(val_str)
        ));
    }

    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>{error_type}</title>
    <style>
        body {{
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
            background-color: #f8f9fa;
            color: #212529;
            margin: 0;
            padding: 20px;
        }}
        .container {{
            max-width: 1000px;
            margin: 0 auto;
            background: #fff;
            padding: 30px;
            border-radius: 8px;
            box-shadow: 0 4px 6px rgba(0,0,0,0.1);
            border-top: 10px solid #ffb300;
        }}
        h1 {{
            color: #d32f2f;
            margin-top: 0;
            font-size: 24px;
            border-bottom: 2px solid #eee;
            padding-bottom: 15px;
        }}
        .info-box {{
            background-color: #fff8e1;
            border-left: 4px solid #ffb300;
            padding: 15px;
            margin-bottom: 20px;
            border-radius: 4px;
        }}
        .info-title {{
            font-weight: bold;
            color: #b78103;
            margin-bottom: 5px;
        }}
        .section-title {{
            font-size: 18px;
            font-weight: bold;
            margin: 30px 0 10px 0;
            color: #37474f;
            border-bottom: 1px solid #cfd8dc;
            padding-bottom: 5px;
        }}
        table {{
            width: 100%;
            border-collapse: collapse;
            margin-top: 10px;
        }}
        th, td {{
            text-align: left;
            padding: 10px;
            border-bottom: 1px solid #e0e0e0;
        }}
        th {{
            background-color: #f5f5f5;
            width: 30%;
            font-weight: 600;
        }}
        td {{
            font-family: monospace;
            word-break: break-all;
        }}
        .debug-note {{
            font-size: 13px;
            color: #78909c;
            margin-top: 40px;
            text-align: center;
            border-top: 1px dashed #cfd8dc;
            padding-top: 20px;
        }}
        .badge {{
            display: inline-block;
            padding: 4px 8px;
            font-size: 12px;
            font-weight: bold;
            border-radius: 4px;
            color: white;
            margin-bottom: 10px;
        }}
        .badge.panicked {{
            background-color: #ff8f00;
        }}
        .badge.internal {{
            background-color: #c62828;
        }}
        .badge.bad-request {{
            background-color: #0288d1;
        }}
        .badge.not-found {{
            background-color: #78909c;
        }}
        .badge.forbidden {{
            background-color: #d32f2f;
        }}
    </style>
</head>
<body>
    <div class="container">
        <span class="badge {badge_class}">{error_type}</span>
        <h1>{error_details_escaped}</h1>
        
        <div class="info-box">
            <div class="info-title">Request Information</div>
            <div><strong>Method:</strong> {method}</div>
            <div><strong>Path:</strong> {path}</div>
        </div>

        <div class="section-title">Request Headers</div>
        <table>
            <thead>
                <tr>
                    <th>Header</th>
                    <th>Value</th>
                </tr>
            </thead>
            <tbody>
                {headers_html}
            </tbody>
        </table>

        <div class="debug-note">
            You're seeing this error page because you have <code>DEBUG = true</code> in your Djangors settings.
            Change this to <code>false</code> in production to show a generic error page.
        </div>
    </div>
</body>
</html>"#,
        error_type = html_escape(error_type),
        badge_class = badge_class,
        error_details_escaped = html_escape(error_details),
        method = html_escape(req.method().as_str()),
        path = html_escape(req.path()),
        headers_html = headers_html
    );

    Response::html(status, html)
}

/// Renders a production-ready, minimal error page that does not leak any
/// internal details of the server.
pub fn render_production_error_page(status: StatusCode) -> Response {
    let code = status.as_u16();
    let reason = status.canonical_reason().unwrap_or("Internal Server Error");

    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>{code} — {reason}</title>
    <style>
        body {{
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
            background-color: #f8f9fa;
            color: #495057;
            display: flex;
            align-items: center;
            justify-content: center;
            height: 100vh;
            margin: 0;
        }}
        .error-container {{
            text-align: center;
        }}
        h1 {{
            font-size: 72px;
            margin: 0;
            color: #343a40;
            font-weight: 300;
        }}
        p {{
            font-size: 20px;
            margin: 10px 0 0 0;
            color: #6c757d;
        }}
    </style>
</head>
<body>
    <div class="error-container">
        <h1>{code}</h1>
        <p>{reason}</p>
    </div>
</body>
</html>"#,
        code = code,
        reason = html_escape(reason)
    );

    Response::html(status, html)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use hyper::http::{HeaderMap, HeaderValue, Method, Uri};

    fn make_request(method: Method, path: &str) -> Request {
        let uri = Uri::try_from(path).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("X-Test-Header", HeaderValue::from_static("test-value"));
        Request::new(method, uri, headers, Bytes::new())
    }

    #[test]
    fn test_render_debug_page() {
        let req = make_request(Method::GET, "/some/path");
        let err = DjangorsError::Panicked("something exploded".to_string());
        let resp = render_debug_page(&err, &req);

        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = String::from_utf8(resp.body().to_vec()).unwrap();
        assert!(body.contains("something exploded"));
        assert!(body.contains("&#x2F;some&#x2F;path"));
        assert!(body.contains("x-test-header"));
        assert!(body.contains("test-value"));
        assert!(body.contains("DEBUG = true"));
    }

    #[test]
    fn test_render_production_error_page() {
        let err_resp = render_production_error_page(StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(err_resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = String::from_utf8(err_resp.body().to_vec()).unwrap();
        assert!(body.contains("500"));
        assert!(body.contains("Internal Server Error"));
        // Check no leak of debug note
        assert!(!body.contains("DEBUG = true"));

        let not_found_resp = render_production_error_page(StatusCode::NOT_FOUND);
        assert_eq!(not_found_resp.status(), StatusCode::NOT_FOUND);
        let body_nf = String::from_utf8(not_found_resp.body().to_vec()).unwrap();
        assert!(body_nf.contains("404"));
        assert!(body_nf.contains("Not Found"));
    }
}
