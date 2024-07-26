use axum::{
    extract::{Query, State},
    Json,
};
use std::{collections::HashMap, sync::Arc};

use crate::common::{AppError, AppState};
use crate::search::search_flakes;
use crate::sql::{get_flakes, get_flakes_by_ids, FlakeRelease};

#[derive(serde::Serialize)]
pub struct GetFlakeResponse {
    releases: Vec<FlakeRelease>,
    count: usize,
    query: Option<String>,
}

pub async fn get_flake(
    State(state): State<Arc<AppState>>,
    Query(mut params): Query<HashMap<String, String>>,
) -> Result<Json<GetFlakeResponse>, AppError> {
    let query = params.remove("q");
    let releases = if let Some(ref q) = query {
        let hits = search_flakes(&state.opensearch, q).await?;

        let mut releases = get_flakes_by_ids(hits.keys().collect(), &state.pool).await?;

        if !releases.is_empty() {
            // Should this be done by the DB?
            releases.sort_by(|a, b| hits[&b.id].partial_cmp(&hits[&a.id]).unwrap());
        }

        releases
    } else {
        get_flakes(&state.pool).await?
    };
    let count = releases.len();
    return Ok(Json(GetFlakeResponse {
        releases,
        count,
        query,
    }));
}
