use djangors_core::{DjangorsError, PathParams, Request, Response, Router, StatusCode};

/// Registers default admin favicon routes onto `router`.
pub fn favicon_routes(router: Router) -> Router {
    router
        .get("/favicon.ico", favicon_ico)
        .get("/favicon-16x16.png", favicon_16)
        .get("/favicon-32x32.png", favicon_32)
        .get("/apple-touch-icon.png", favicon_apple_touch)
        .get("/android-chrome-192x192.png", favicon_android_192)
        .get("/android-chrome-512x512.png", favicon_android_512)
        .get("/manifest.json", favicon_manifest)
}

pub(crate) async fn favicon_ico(
    _req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    Ok(Response::bytes(
        StatusCode::OK,
        "image/x-icon",
        include_bytes!("../static/branding/favicon.ico").to_vec(),
    ))
}

pub(crate) async fn favicon_16(
    _req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    Ok(Response::bytes(
        StatusCode::OK,
        "image/png",
        include_bytes!("../static/branding/favicon-16x16.png").to_vec(),
    ))
}

pub(crate) async fn favicon_32(
    _req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    Ok(Response::bytes(
        StatusCode::OK,
        "image/png",
        include_bytes!("../static/branding/favicon-32x32.png").to_vec(),
    ))
}

pub(crate) async fn favicon_apple_touch(
    _req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    Ok(Response::bytes(
        StatusCode::OK,
        "image/png",
        include_bytes!("../static/branding/apple-touch-icon.png").to_vec(),
    ))
}

pub(crate) async fn favicon_android_192(
    _req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    Ok(Response::bytes(
        StatusCode::OK,
        "image/png",
        include_bytes!("../static/branding/android-chrome-192x192.png").to_vec(),
    ))
}

pub(crate) async fn favicon_android_512(
    _req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    Ok(Response::bytes(
        StatusCode::OK,
        "image/png",
        include_bytes!("../static/branding/android-chrome-512x512.png").to_vec(),
    ))
}

pub(crate) async fn favicon_manifest(
    _req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    Ok(Response::bytes(
        StatusCode::OK,
        "application/json; charset=utf-8",
        include_bytes!("../static/branding/manifest.json").to_vec(),
    ))
}
