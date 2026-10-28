//! Password login, the access/admin gate middleware and the `Host` check.

use std::net::{IpAddr, Ipv6Addr};
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use serde::Deserialize;
use subtle::ConstantTimeEq;

use crate::api::{ApiError, JsonBody};
use crate::config::Config;
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
    JsonBody(body): JsonBody<Login>,
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
    JsonBody(body): JsonBody<Login>,
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
/// every other `/api/` path except health and login needs `nk_access` when an access password
/// is set.
pub async fn gate(
    State(state): State<AppState>,
    jar: CookieJar,
    mut req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path();
    // Ports are part of the origin: another localhost service is not this browser origin.
    if !matches!(
        *req.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    ) && let Some(origin) = req.headers().get(header::ORIGIN)
    {
        let same_origin = origin
            .to_str()
            .ok()
            .and_then(|value| reqwest::Url::parse(value).ok())
            .filter(|url| {
                matches!(url.scheme(), "http" | "https")
                    && url.username().is_empty()
                    && url.password().is_none()
            })
            .is_some_and(|origin| {
                req.headers()
                    .get(header::HOST)
                    .and_then(|host| host.to_str().ok())
                    .and_then(|host| {
                        reqwest::Url::parse(&format!("{}://{host}", origin.scheme())).ok()
                    })
                    .is_some_and(|target| {
                        target.host_str() == origin.host_str()
                            && target.port_or_known_default() == origin.port_or_known_default()
                    })
            });
        if !same_origin {
            return ApiError::new(StatusCode::FORBIDDEN, "不允许的请求来源").into_response();
        }
    }
    if path == "/api/admin" || path.starts_with("/api/admin/") {
        if state.config.admin_password.is_none() {
            return ApiError::not_found().into_response();
        }
        if path != "/api/admin/session" && !has_session(&state, &jar, Scope::Admin, ADMIN_COOKIE) {
            return ApiError::unauthorized().into_response();
        }
    } else if path.starts_with("/api/")
        && path != "/api/health"
        && path != "/api/session"
        && state.config.access_password.is_some()
        && !has_session(&state, &jar, Scope::Access, ACCESS_COOKIE)
        && !(path.starts_with("/api/avatars/")
            && has_session(&state, &jar, Scope::Admin, ADMIN_COOKIE))
    {
        return ApiError::unauthorized().into_response();
    }
    if path.starts_with("/api/")
        && path != "/api/health"
        && path != "/api/session"
        && !path.starts_with("/api/admin/")
    {
        let account_entry = matches!(
            path,
            "/api/account/register" | "/api/account/login" | "/api/account/session"
        );
        let admin_avatar = path.starts_with("/api/avatars/")
            && has_session(&state, &jar, Scope::Admin, ADMIN_COOKIE);
        if !account_entry && !admin_avatar {
            match crate::accounts::resolve(&state, &jar).await {
                Ok(Some(user)) => {
                    req.extensions_mut()
                        .insert(crate::accounts::CurrentUser::from(user));
                }
                Ok(None) => return ApiError::unauthorized().into_response(),
                Err(error) => return error.into_response(),
            }
        }
    }
    next.run(req).await
}

/// The hostname part of a `Host` value: brackets and port removed (a bare IPv6 address stays).
fn hostname(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split(']').next().unwrap_or_default();
    }
    match host.rsplit_once(':') {
        Some((name, _port)) if !name.contains(':') => name,
        _ => host,
    }
}

fn host_allowed(config: &Config, host: &str) -> bool {
    let name = hostname(host);
    if name.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(ip) = name.parse::<IpAddr>() {
        let listen = config.listen.ip();
        if ip == IpAddr::from([127, 0, 0, 1])
            || ip == IpAddr::from(Ipv6Addr::LOCALHOST)
            || (!listen.is_unspecified() && ip == listen)
        {
            return true;
        }
    }
    config
        .allowed_hosts
        .iter()
        .any(|h| h.eq_ignore_ascii_case(name))
}

/// Rejects requests whose `Host` is not this server's (DNS-rebinding defence): loopback names,
/// the specific listen address and `allowed_hosts` pass; anything else is 421.
pub async fn check_host(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .or_else(|| req.uri().authority().map(|a| a.as_str()));
    match host {
        Some(host) if host_allowed(&state.config, host) => next.run(req).await,
        _ => ApiError::new(StatusCode::MISDIRECTED_REQUEST, "不允许的访问地址").into_response(),
    }
}
