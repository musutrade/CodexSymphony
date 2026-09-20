//! Preserve the original closed HTTP contract while the multi-repository API
//! exposes registration and selection identities explicitly.
use axum::{
    body::{Body, to_bytes},
    http::Request,
    middleware::Next,
    response::Response,
};
use serde_json::Value;

pub async fn response(request: Request<Body>, next: Next) -> Response {
    let response = next.run(request).await;
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = to_bytes(body, usize::MAX).await else {
        return Response::builder().status(503).body(Body::empty()).unwrap();
    };
    let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    strip(&mut value);
    parts.headers.remove(axum::http::header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(value.to_string()))
}
fn strip(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.remove("repository_id");
            object.remove("delivery_ready");
            if object.contains_key("repository") && object.contains_key("version") {
                object.remove("id");
            }
            for child in object.values_mut() {
                strip(child);
            }
        }
        Value::Array(array) => {
            for child in array {
                strip(child);
            }
        }
        _ => {}
    }
}
