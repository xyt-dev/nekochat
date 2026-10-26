//! User-facing conversation API.

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::api::{ApiError, blocking};
use crate::state::AppState;

#[derive(Serialize)]
pub struct ConversationSummary {
    id: String,
    title: String,
    updated_at: i64,
}

/// `GET /api/conversations`, newest first.
pub async fn list_conversations(
    State(state): State<AppState>,
) -> Result<Json<Vec<ConversationSummary>>, ApiError> {
    let store = state.store.clone();
    let convs = blocking(move || store.list_conversations()).await?;
    Ok(Json(
        convs
            .into_iter()
            .map(|c| ConversationSummary {
                id: c.id,
                title: c.title,
                updated_at: c.updated_at,
            })
            .collect(),
    ))
}
