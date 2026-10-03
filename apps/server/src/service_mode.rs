//! Keep authenticated observation available when execution admission fails.
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};

pub const HEADER: &str = "x-codexsymphony-service-mode";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    ObservationOnly,
}

impl Mode {
    pub fn from_admission(admission: Result<(), Box<dyn std::error::Error + Send + Sync>>) -> Self {
        match admission {
            Ok(_) => Self::Normal,
            Err(error) => {
                let detail = crate::operator_view::redact_text(&error.to_string());
                tracing::error!(
                    "execution admission unavailable; authenticated observation remains available: {detail}"
                );
                Self::ObservationOnly
            }
        }
    }

    pub fn executions_enabled(self) -> bool {
        self == Self::Normal
    }

    fn value(self) -> HeaderValue {
        match self {
            Self::Normal => HeaderValue::from_static("normal"),
            Self::ObservationOnly => HeaderValue::from_static("observation-only"),
        }
    }
}

pub fn protect(router: Router, mode: Mode) -> Router {
    router.layer(middleware::from_fn_with_state(mode, enforce))
}

fn allowed(method: &Method, path: &str) -> bool {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return true;
    }
    *method == Method::POST && matches!(path, "/api/auth/login" | "/api/auth/logout")
}

async fn enforce(State(mode): State<Mode>, request: Request, next: Next) -> Response {
    let mut response = if mode.executions_enabled()
        || allowed(request.method(), request.uri().path())
    {
        next.run(request).await
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"message":"当前仅可查看，执行和修改功能暂不可用。","code":"execution_unavailable"})),
        )
            .into_response()
    };
    response.headers_mut().insert(HEADER, mode.value());
    response
}

#[cfg(test)]
#[path = "../tests/unit/service_mode.rs"]
mod tests;
