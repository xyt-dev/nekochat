//! Password login and the access/admin gate middleware.

use std::time::Duration;

use axum::Json;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use serde::Deserialize;
use subtle::ConstantTimeEq;

use crate::api::ApiError;
use crate::state::{AppState, Scope};

pub const ACCESS_COOKIE: &str = "nk_access";
pub const ADMIN_COOKIE: &str = "nk_admin";
const LOGIN_FAILURE_DELAY: Duration = Duration::from_millis(500);

#[derive(Deserialize)]
pub struct Login {
    password: String,
}

fn password_matches(expected: &str, given: &str) -> bool {
    expected.as_bytes().ct_eq(given.as_bytes()).into()
}

async fn login(
    state: &AppState,
    scope: Scope,
    expected: &str,
    given: &str,
    jar: CookieJar,
) -> Result<(StatusCode, CookieJar), ApiError> {
    if !password_matches(expected, given) {
        tokio::time::sleep(LOGIN_FAILURE_DELAY).await;
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "密码错误"));
    }
    let name = match scope {
        Scope::Access => ACCESS_COOKIE,
        Scope::Admin => ADMIN_COOKIE,
    };
    let cookie = Cookie::build((name, state.sessions.issue(scope)))
        .http_only(true)
        .same_site(SameSite::Strict)
        .path("/");
    Ok((StatusCode::NO_CONTENT, jar.add(cookie)))
}

pub async fn access_login(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(body): Json<Login>,
) -> Result<Response, ApiError> {
    match state.config.access_password.as_deref() {
        None => Ok(StatusCode::NO_CONTENT.into_response()),
        Some(pw) => Ok(login(&state, Scope::Access, pw, &body.password, jar)
            .await?
            .into_response()),
    }
}

pub async fn admin_login(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(body): Json<Login>,
) -> Result<Response, ApiError> {
    match state.config.admin_password.as_deref() {
        None => Err(ApiError::not_found()),
        Some(pw) => Ok(login(&state, Scope::Admin, pw, &body.password, jar)
            .await?
            .into_response()),
    }
}

fn has_session(state: &AppState, jar: &CookieJar, scope: Scope, cookie: &str) -> bool {
    jar.get(cookie)
        .is_some_and(|c| state.sessions.contains(scope, c.value()))
}

/// Gates `/api/*`: admin routes need `nk_admin` (404 without an admin password);
/// everything else except health and login needs `nk_access` when an access password is set.
pub async fn gate(
    State(state): State<AppState>,
    jar: CookieJar,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path();
    if path == "/api/admin" || path.starts_with("/api/admin/") {
        if state.config.admin_password.is_none() {
            return ApiError::not_found().into_response();
        }
        if path != "/api/admin/session" && !has_session(&state, &jar, Scope::Admin, ADMIN_COOKIE) {
            return ApiError::unauthorized().into_response();
        }
    } else if path != "/api/health"
        && path != "/api/session"
        && state.config.access_password.is_some()
        && !has_session(&state, &jar, Scope::Access, ACCESS_COOKIE)
    {
        return ApiError::unauthorized().into_response();
    }
    next.run(req).await
}
