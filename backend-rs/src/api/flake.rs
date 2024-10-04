use anyhow::Context;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::NaiveDateTime;
use opensearch::{OpenSearch, SearchParts};
use serde_json::{json, Value};
use sqlx::{postgres::PgRow, FromRow, Pool, Postgres, Row};
use std::{cmp::Ordering, collections::HashMap, sync::Arc};

use crate::common::{AppError, AppState};

#[derive(serde::Serialize)]
struct FlakeReleaseCompact {
    #[serde(skip_serializing)]
    id: i32,
    owner: String,
    repo: String,
    version: String,
    description: String,
    created_at: NaiveDateTime,
}

impl Eq for FlakeReleaseCompact {}

impl Ord for FlakeReleaseCompact {
    fn cmp(&self, other: &Self) -> Ordering {
        self.id.cmp(&other.id)
    }
}

impl PartialOrd for FlakeReleaseCompact {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for FlakeReleaseCompact {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl FromRow<'_, PgRow> for FlakeReleaseCompact {
    fn from_row(row: &PgRow) -> sqlx::Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            owner: row.try_get("owner")?,
            repo: row.try_get("repo")?,
            version: row.try_get("version")?,
            description: row.try_get("description").unwrap_or_default(),
            created_at: row.try_get("created_at")?,
        })
    }
}

#[derive(serde::Serialize)]
struct FlakeRelease {
    #[serde(skip_serializing)]
    id: i32,
    owner: String,
    repo: String,
    version: String,
    description: String,
    created_at: NaiveDateTime,
    commit: String,
    readme: String
}

impl Eq for FlakeRelease {}

impl Ord for FlakeRelease {
    fn cmp(&self, other: &Self) -> Ordering {
        self.id.cmp(&other.id)
    }
}

impl PartialOrd for FlakeRelease {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for FlakeRelease {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl FromRow<'_, PgRow> for FlakeRelease {
    fn from_row(row: &PgRow) -> sqlx::Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            owner: row.try_get("owner")?,
            repo: row.try_get("repo")?,
            version: row.try_get("version")?,
            description: row.try_get("description").unwrap_or_default(),
            created_at: row.try_get("created_at")?,
            commit: row.try_get("commit")?,
            readme: row.try_get("readme")?,
        })
    }
}

#[derive(Debug)]
struct RepoId(i32);

impl FromRow<'_, PgRow> for RepoId {
    fn from_row(row: &PgRow) -> sqlx::Result<Self> {
        Ok(Self(row.try_get("id")?))
    }
}

#[derive(serde::Serialize)]
pub struct GetFlakeResponse {
    releases: Vec<FlakeReleaseCompact>,
    count: usize,
    query: Option<String>,
}

#[derive(serde::Serialize)]
pub struct RepoResponse {
    releases: Vec<FlakeRelease>,
}

#[derive(serde::Serialize)]
pub struct NotFoundResponse 
{
    detail: String,
}

impl NotFoundResponse
{
    pub fn build() -> Self
    {
        NotFoundResponse 
        {
            detail: "Not Found".into()
        }
    }
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
            releases.sort();
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

pub async fn read_repo(
    Path((owner, repo)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
) -> Result<Response, AppError> {
    let repo_id = get_repo_id(&owner, &repo, &state.pool).await?;

    if let Some(repo_id) = repo_id {
        let mut releases = get_repo_releases(&repo_id, &state.pool).await?;

        if !releases.is_empty() {
            releases.sort_by(|a, b| a.version.cmp(&b.version));
            releases.reverse();
        }

        return Ok((StatusCode::OK, Json(RepoResponse { releases })).into_response());
    } else {
        return Ok((StatusCode::NOT_FOUND, Json(NotFoundResponse::build())).into_response());
    }
}

async fn get_repo_id(
    owner: &str,
    repo: &str,
    pool: &Pool<Postgres>,
) -> Result<Option<RepoId>, AppError> {
    let query = "SELECT githubrepo.id as id \
            FROM githubrepo \
            INNER JOIN githubowner ON githubowner.id = githubrepo.owner_id \
            WHERE githubrepo.name = $1 AND githubowner.name = $2 LIMIT 1";

    let repo_id: Option<RepoId> = sqlx::query_as(&query)
        .bind(&repo)
        .bind(&owner)
        .fetch_optional(pool)
        .await
        .context("Failed to fetch repo id from database")?;

    Ok(repo_id)
}

async fn get_repo_releases(
    repo_id: &RepoId,
    pool: &Pool<Postgres>,
) -> Result<Vec<FlakeRelease>, AppError> {
    let query = format!(
        "SELECT release.id AS id, \
            githubowner.name AS owner, \
            githubrepo.name AS repo, \
            release.version AS version, \
            release.description AS description, \
            release.commit AS commit, \
            release.readme AS readme, \
            release.created_at AS created_at \
            FROM release \
            INNER JOIN githubrepo ON githubrepo.id = release.repo_id \
            INNER JOIN githubowner ON githubowner.id = githubrepo.owner_id \
            WHERE release.repo_id = $1",
    );

    let releases: Vec<FlakeRelease> = sqlx::query_as(&query)
        .bind(&repo_id.0)
        .fetch_all(pool)
        .await
        .context("Failed to fetch repo releases from database")?;

    Ok(releases)
}

async fn get_flakes_by_ids(
    flake_ids: Vec<&i32>,
    pool: &Pool<Postgres>,
) -> Result<Vec<FlakeReleaseCompact>, AppError> {
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

    let releases: Vec<FlakeReleaseCompact> = 
        sqlx::query_as(&query)
        .fetch_all(pool)
        .await
        .context("Failed to fetch flakes by id from database")?;

    Ok(releases)
}

async fn get_flakes(pool: &Pool<Postgres>) -> Result<Vec<FlakeReleaseCompact>, AppError> {
    let releases: Vec<FlakeReleaseCompact> = sqlx::query_as(
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
    .await
    .context("Failed to fetch flakes from database")?;

    Ok(releases)
}

async fn search_flakes(opensearch: &OpenSearch, q: &String) -> Result<HashMap<i32, f64>, AppError> {
    let res = opensearch
        .search(SearchParts::Index(&["flakes"]))
        .size(10)
        .body(json!({
            "query": {
                "multi_match": {
                    "query": q,
                    "fuzziness": "AUTO",
                    "fields": [
                        "description^2",
                        "readme",
                        "outputs",
                        "repo^2",
                        "owner^2",
                    ],
                }
            }
        }))
        .send()
        .await
        .context("Failed to send opensearch request")?
        .json::<Value>()
        .await
        .context("Failed to decode opensearch response as json")?;

    // TODO: Remove this unwrap, use fold or map to create the HashMap
    let mut hits: HashMap<i32, f64> = HashMap::new();

    let hit_res = res["hits"]["hits"]
        .as_array()
        .context("failed to extract hits from open search response")?;

    for hit in hit_res {
        let id = hit["_id"]
            .as_str()
            .context("failed to read id as string from open search hit")?
            .parse()
            .context("failed to parse id from open search hit")?;
        let score = hit["_score"]
            .as_f64()
            .context("failed to parse score from open search hit")?;

        hits.insert(id, score);
    }

    Ok(hits)
}
