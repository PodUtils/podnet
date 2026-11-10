use super::*;

mod download;
mod upload;

pub(super) async fn send(
    router: &Router,
    method: &str,
    path: &str,
    body: &str,
) -> (u16, Option<String>) {
    use tower::ServiceExt;
    let request = http::Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let location = response
        .headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().unwrap().to_string());
    (response.status().as_u16(), location)
}

#[test]
fn proxy_path_is_root() {
    let config = ShareConfig::proxy("127.0.0.1:9000");
    assert_eq!(config.url_path(), "/");
    assert!(config.gate.enabled());
}
