//! Mandatory local account identities, persistent sessions and administrator management.
use crate::{
    api::{ApiError, JsonBody, blocking},
    state::AppState,
};
use axum::{
    Extension, Json,
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use axum_extra::extract::{
    CookieJar,
    cookie::{Cookie, SameSite},
};
use nekochat_store::{PasswordStorage, User, UserPublic, now_ms};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const USER_COOKIE: &str = "nk_user";
const SESSION_MS: i64 = 30 * 24 * 60 * 60 * 1000;
#[derive(Clone, Debug)]
pub struct CurrentUser {
    pub id: String,
    pub username: String,
    pub avatar: Option<String>,
}
impl From<UserPublic> for CurrentUser {
    fn from(u: UserPublic) -> Self {
        Self {
            id: u.id,
            username: u.username,
            avatar: u.avatar,
        }
    }
}
pub fn fingerprint(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}
pub async fn resolve(state: &AppState, jar: &CookieJar) -> Result<Option<UserPublic>, ApiError> {
    let Some(cookie) = jar.get(USER_COOKIE) else {
        return Ok(None);
    };
    let digest = fingerprint(cookie.value());
    let store = state.store.clone();
    Ok(blocking(move || store.user_for_session(&digest))
        .await?
        .map(|u| u.public))
}

pub(crate) async fn http_observer(
    state: &AppState,
    user: &CurrentUser,
    jar: &CookieJar,
) -> Result<std::sync::Arc<tokio_util::sync::CancellationToken>, ApiError> {
    let cookie = jar
        .get(USER_COOKIE)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "请重新登录"))?;
    let cancellation = state
        .observations
        .register(&user.id, fingerprint(cookie.value()));
    // Register first, then revalidate to close the session-revocation race.
    if resolve(state, jar)
        .await?
        .is_none_or(|current| current.id != user.id)
    {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "请重新登录"));
    }
    Ok(cancellation)
}
async fn issue(state: &AppState, jar: CookieJar, user: &User) -> Result<CookieJar, ApiError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| ApiError::internal())?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut token = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        token.push(char::from(HEX[usize::from(byte >> 4)]));
        token.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    let digest = fingerprint(&token);
    let store = state.store.clone();
    let id = user.id.clone();
    let revision = user.credential_revision;
    blocking(move || store.create_user_session(&id, revision, &digest, now_ms() + SESSION_MS))
        .await?;
    Ok(jar.add(
        Cookie::build((USER_COOKIE, token))
            .http_only(true)
            .same_site(SameSite::Strict)
            .path("/")
            .max_age(time::Duration::days(30)),
    ))
}
fn validate_username(value: &str) -> Result<(), ApiError> {
    if value.trim() != value
        || value.is_empty()
        || value.chars().count() > 64
        || value.chars().any(char::is_control)
    {
        Err(ApiError::bad_request(
            "用户名须为1到64个非控制字符，且首尾不能有空格",
        ))
    } else {
        Ok(())
    }
}
fn validate_password(value: &str) -> Result<(), ApiError> {
    if value.len() < 8 || value.len() > 1024 {
        Err(ApiError::bad_request("密码须为8到1024字节"))
    } else {
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    username: String,
    password: String,
}
pub async fn session(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<Json<Option<UserPublic>>, ApiError> {
    Ok(Json(resolve(&state, &jar).await?))
}
pub async fn register(
    State(state): State<AppState>,
    jar: CookieJar,
    JsonBody(body): JsonBody<Credentials>,
) -> Result<(CookieJar, Json<UserPublic>), ApiError> {
    validate_username(&body.username)?;
    validate_password(&body.password)?;
    let store = state.store.clone();
    let user = blocking(move || store.create_user(&body.username, &body.password)).await?;
    switch_out(&state, &jar).await?;
    Ok((issue(&state, jar, &user).await?, Json(user.public)))
}
pub async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    JsonBody(body): JsonBody<Credentials>,
) -> Result<(CookieJar, Json<UserPublic>), ApiError> {
    validate_username(&body.username)?;
    validate_password(&body.password)?;
    let store = state.store.clone();
    let user = blocking(move || store.authenticate_user(&body.username, &body.password)).await?;
    let Some(user) = user else {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "用户名或密码错误"));
    };
    switch_out(&state, &jar).await?;
    Ok((issue(&state, jar, &user).await?, Json(user.public)))
}
async fn switch_out(state: &AppState, jar: &CookieJar) -> Result<(), ApiError> {
    if let Some(cookie) = jar.get(USER_COOKIE) {
        let digest = fingerprint(cookie.value());
        let revoked = digest.clone();
        let store = state.store.clone();
        blocking(move || store.revoke_user_session(&digest)).await?;
        state.observations.revoke_session(&revoked);
    }
    Ok(())
}
pub async fn logout(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<(StatusCode, CookieJar), ApiError> {
    switch_out(&state, &jar).await?;
    Ok((
        StatusCode::NO_CONTENT,
        jar.remove(Cookie::build(USER_COOKIE).path("/")),
    ))
}

// serde's default Option collapses null and absent; this preserves explicit avatar clearing.
fn optional_nullable<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProfilePatch {
    username: Option<String>,
    password: Option<String>,
    current_password: Option<String>,
    #[serde(default, deserialize_with = "optional_nullable")]
    avatar: Option<Option<String>>,
}
pub async fn avatar_url_allowed(state: &AppState, url: &str) -> Result<bool, ApiError> {
    let Some(id) = url.strip_prefix("/api/avatars/") else {
        return Ok(false);
    };
    if uuid::Uuid::parse_str(id).is_err() {
        return Ok(false);
    }
    let id = id.to_owned();
    let store = state.store.clone();
    blocking(move || store.avatar_exists(&id)).await
}
async fn validate_profile(
    state: &AppState,
    username: Option<&str>,
    password: Option<&str>,
    avatar: Option<&Option<String>>,
) -> Result<(), ApiError> {
    if let Some(value) = username {
        validate_username(value)?
    }
    if let Some(value) = password {
        validate_password(value)?
    }
    if let Some(Some(url)) = avatar {
        if !avatar_url_allowed(state, url).await? {
            return Err(ApiError::bad_request("头像必须为已上传的本站图片"));
        }
    }
    Ok(())
}
pub async fn update_profile(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    jar: CookieJar,
    JsonBody(body): JsonBody<ProfilePatch>,
) -> Result<(CookieJar, Json<UserPublic>), ApiError> {
    validate_profile(
        &state,
        body.username.as_deref(),
        body.password.as_deref(),
        body.avatar.as_ref(),
    )
    .await?;
    let verified_revision = if body.password.is_some() {
        let current = body
            .current_password
            .as_deref()
            .ok_or_else(|| ApiError::bad_request("修改密码需要原密码"))?
            .to_owned();
        let store = state.store.clone();
        let id = user.id.clone();
        let Some(revision) =
            blocking(move || store.verified_password_revision(&id, &current)).await?
        else {
            return Err(ApiError::bad_request("原密码错误"));
        };
        Some(revision)
    } else {
        None
    };
    let changed_password = body.password.is_some();
    let store = state.store.clone();
    let id = user.id.clone();
    let updated = blocking(move || {
        store.update_user(
            &id,
            verified_revision,
            body.username.as_deref(),
            body.password.as_deref(),
            body.avatar.as_ref().map(|a| a.as_deref()),
            None,
        )
    })
    .await?;
    let jar = if changed_password {
        state.observations.revoke_user(&user.id);
        issue(&state, jar, &updated).await?
    } else {
        jar
    };
    Ok((jar, Json(updated.public)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    last_character_id: Option<String>,
    last_conversation_id: Option<String>,
}
pub async fn preferences(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    JsonBody(body): JsonBody<Preferences>,
) -> Result<Json<UserPublic>, ApiError> {
    if let Some(id) = &body.last_character_id {
        if !crate::api::user::available_characters(&state)
            .await?
            .contains(id)
        {
            return Err(ApiError::bad_request("角色不可用"));
        }
    }
    let store = state.store.clone();
    Ok(Json(
        blocking(move || {
            store.set_user_preferences(
                &user.id,
                body.last_character_id.as_deref(),
                body.last_conversation_id.as_deref(),
            )
        })
        .await?,
    ))
}
pub async fn require_conversation(
    state: &AppState,
    user: &CurrentUser,
    id: &str,
) -> Result<(), ApiError> {
    let store = state.store.clone();
    let user = user.id.clone();
    let id = id.to_owned();
    blocking(move || store.require_conversation_owner(&id, &user)).await
}

pub const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;
async fn save_avatar(state: &AppState, bytes: Bytes) -> Result<Json<Value>, ApiError> {
    if bytes.len() > MAX_AVATAR_BYTES {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "头像不能超过2MiB",
        ));
    }
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        return Err(ApiError::bad_request("头像仅支持PNG、JPEG或WebP"));
    };
    let store = state.store.clone();
    let url = blocking(move || store.insert_avatar(mime, &bytes)).await?;
    Ok(Json(json!({"url":url})))
}
pub async fn upload_avatar(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    bytes: Bytes,
) -> Result<Json<Value>, ApiError> {
    save_avatar(&state, bytes).await
}
pub async fn admin_upload_avatar(
    State(state): State<AppState>,
    bytes: Bytes,
) -> Result<Json<Value>, ApiError> {
    save_avatar(&state, bytes).await
}
pub async fn avatar(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let store = state.store.clone();
    let (mime, bytes) = blocking(move || store.get_avatar(&id))
        .await?
        .ok_or_else(ApiError::not_found)?;
    Ok((
        [
            ("content-type", mime.as_str()),
            ("x-content-type-options", "nosniff"),
            ("cache-control", "private, max-age=86400"),
        ],
        bytes,
    )
        .into_response())
}

#[derive(Serialize)]
pub struct Users {
    users: Vec<User>,
}
pub async fn admin_users(State(state): State<AppState>) -> Result<Json<Users>, ApiError> {
    let store = state.store.clone();
    Ok(Json(Users {
        users: blocking(move || store.list_users()).await?,
    }))
}
pub async fn admin_create_user(
    State(state): State<AppState>,
    JsonBody(body): JsonBody<Credentials>,
) -> Result<(StatusCode, Json<User>), ApiError> {
    validate_username(&body.username)?;
    validate_password(&body.password)?;
    let store = state.store.clone();
    Ok((
        StatusCode::CREATED,
        Json(blocking(move || store.create_user(&body.username, &body.password)).await?),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminPatch {
    username: Option<String>,
    password: Option<String>,
    disabled: Option<bool>,
    #[serde(default, deserialize_with = "optional_nullable")]
    avatar: Option<Option<String>>,
}
pub async fn admin_update_user(
    State(state): State<AppState>,
    Path(id): Path<String>,
    JsonBody(body): JsonBody<AdminPatch>,
) -> Result<Json<User>, ApiError> {
    validate_profile(
        &state,
        body.username.as_deref(),
        body.password.as_deref(),
        body.avatar.as_ref(),
    )
    .await?;
    let revoke = body.password.is_some() || body.disabled == Some(true);
    let store = state.store.clone();
    let update_id = id.clone();
    let updated = blocking(move || {
        store.update_user(
            &update_id,
            None,
            body.username.as_deref(),
            body.password.as_deref(),
            body.avatar.as_ref().map(|a| a.as_deref()),
            body.disabled,
        )
    })
    .await?;
    if revoke {
        state.observations.revoke_user(&id);
    }
    Ok(Json(updated))
}
pub async fn admin_delete_user(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let store = state.store.clone();
    let user_id = id.clone();
    let conversations = blocking(move || store.delete_user(&id)).await;
    state.observations.revoke_user(&user_id);
    let conversations = conversations?;
    for conversation in conversations {
        state.cancel_turn(&conversation);
    }
    let store = state.store.clone();
    blocking(move || store.purge_deleted_pages()).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn admin_password(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let store = state.store.clone();
    let (password, password_storage) = blocking(move || store.user_password(&id)).await?;
    Ok(Json(
        json!({"password":password,"password_storage":password_storage}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserSettings {
    password_storage: PasswordStorage,
}
pub async fn admin_user_settings(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let store = state.store.clone();
    let (mode, count) = blocking(move || {
        Ok((
            store.password_storage()?,
            store.legacy_conversation_count()?,
        ))
    })
    .await?;
    Ok(Json(
        json!({"password_storage":mode,"legacy_conversations":count}),
    ))
}
pub async fn admin_put_user_settings(
    State(state): State<AppState>,
    JsonBody(body): JsonBody<UserSettings>,
) -> Result<Json<Value>, ApiError> {
    let store = state.store.clone();
    blocking(move || store.set_password_storage(body.password_storage)).await?;
    admin_user_settings(State(state)).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyAssignment {
    conversation_ids: Option<Vec<String>>,
}
pub async fn admin_assign_legacy(
    State(state): State<AppState>,
    Path(id): Path<String>,
    JsonBody(body): JsonBody<LegacyAssignment>,
) -> Result<Json<Value>, ApiError> {
    let store = state.store.clone();
    let assigned =
        blocking(move || store.assign_legacy_conversations(&id, body.conversation_ids.as_deref()))
            .await?;
    Ok(Json(json!({"assigned":assigned})))
}
