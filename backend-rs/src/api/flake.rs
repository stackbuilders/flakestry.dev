use axum::{
    extract::{Query, State},
    Json,
};
use chrono::NaiveDateTime;
use sqlx::{Pool, Postgres};
use std::{collections::HashMap, sync::Arc};

use crate::common::{AppError, AppState};
use crate::search::search_flakes;

#[derive(serde::Serialize, sqlx::FromRow)]
struct FlakeRelease {
    #[serde(skip_serializing)]
    id: i32,
    owner: String,
    repo: String,
    version: String,
    description: Option<String>,
    created_at: NaiveDateTime,
}

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

async fn get_flakes_by_ids(
    flake_ids: Vec<&i32>,
    pool: &Pool<Postgres>,
) -> Result<Vec<FlakeRelease>, sqlx::Error> {
    if flake_ids.is_empty() {
        return Ok(vec![]);
    }

    let param_string = flake_ids.iter().fold(String::new(), |acc, &id| {
        format!("{acc}{}{id}", if acc.is_empty() { "" } else { "," })
    });
    let query = format!(
        "SELECT release.id AS id, \
            githubowner.name AS owner, \
            githubrepo.name AS repo, \
            release.version AS version, \
            release.description AS description, \
            release.created_at AS created_at \
            FROM release \
            INNER JOIN githubrepo ON githubrepo.id = release.repo_id \
            INNER JOIN githubowner ON githubowner.id = githubrepo.owner_id \
            WHERE release.id IN ({param_string})",
    );

    let releases: Vec<FlakeRelease> = sqlx::query_as(&query).fetch_all(pool).await?;

    Ok(releases)
}

async fn get_flakes(pool: &Pool<Postgres>) -> Result<Vec<FlakeRelease>, sqlx::Error> {
    let releases: Vec<FlakeRelease> = sqlx::query_as(
        "SELECT release.id AS id, \
            githubowner.name AS owner, \
            githubrepo.name AS repo, \
            release.version AS version, \
            release.description AS description, \
            release.created_at AS created_at \
            FROM release \
            INNER JOIN githubrepo ON githubrepo.id = release.repo_id \
            INNER JOIN githubowner ON githubowner.id = githubrepo.owner_id \
            ORDER BY release.created_at DESC LIMIT 100",
    )
    .fetch_all(pool)
    .await?;

    Ok(releases)
}
