use chrono::NaiveDateTime;
use sqlx::{Pool, Postgres};

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct FlakeRelease {
    #[serde(skip_serializing)]
    pub id: i32,
    owner: String,
    repo: String,
    version: String,
    description: Option<String>,
    created_at: NaiveDateTime,
}

pub async fn get_flakes_by_ids(
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

pub async fn get_flakes(pool: &Pool<Postgres>) -> Result<Vec<FlakeRelease>, sqlx::Error> {
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
