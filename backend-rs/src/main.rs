use std::{collections::HashMap, env, net::SocketAddr, sync::Arc};

use axum::{
    extract::{ConnectInfo, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use core::time::Duration;
use opensearch::OpenSearch;
use sqlx::postgres::{PgPool, PgPoolOptions};
use tower_http::trace::TraceLayer;
use tracing::{field, info_span, Span};
use tracing_subscriber::{fmt, EnvFilter};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use flakestry_dev::search::{create_flake_index, search_flakes};
use flakestry_dev::sql::{get_flakes, get_flakes_by_ids, FlakeRelease};

struct AppState {
    opensearch: OpenSearch,
    pool: PgPool,
}

enum AppError {
    OpenSearchError(opensearch::Error),
    SqlxError(sqlx::Error),
}

impl From<opensearch::Error> for AppError {
    fn from(value: opensearch::Error) -> Self {
        AppError::OpenSearchError(value)
    }
}

impl From<sqlx::Error> for AppError {
    fn from(value: sqlx::Error) -> Self {
        AppError::SqlxError(value)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let body = match self {
            AppError::OpenSearchError(error) => error.to_string(),
            AppError::SqlxError(error) => error.to_string(),
        };
        (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
    }
}

#[derive(serde::Serialize)]
struct GetFlakeResponse {
    releases: Vec<FlakeRelease>,
    count: usize,
    query: Option<String>,
}

#[tokio::main]
async fn main() {
    // TODO: read PG and OS host names from env variables
    // build our application with a single route
    dotenv::dotenv().ok();
    tracing_subscriber::registry()
        .with(fmt::layer().with_target(false))
        .with(EnvFilter::from_default_env())
        .init();
    let database_url = env::var("DATABASE_URL").unwrap();
    let pool = PgPoolOptions::new().connect(&database_url).await.unwrap();
    let state = Arc::new(AppState {
        opensearch: OpenSearch::default(),
        pool,
    });
    let _ = create_flake_index(&state.opensearch).await;
    // run our app with hyper, listening globally on port 3000
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::info!("Listening on 0.0.0.0:3000");
    axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

async fn add_ip_trace(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> impl IntoResponse {
    Span::current().record("ip", format!("{}", addr));

    next.run(req).await
}

fn app(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/flake", get(get_flake))
        .route("/publish", post(post_publish));
    Router::new()
        .nest("/api", api)
        .layer(middleware::from_fn(add_ip_trace))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(
                    |request: &Request| {
                        info_span!("request", ip = field::Empty, method = %request.method(), uri = %request.uri(), version = ?request.version())
                    }
                )
        )
        .with_state(state)
}

async fn get_flake(
    State(state): State<Arc<AppState>>,
    Query(mut params): Query<HashMap<String, String>>,
) -> Result<Json<GetFlakeResponse>, AppError> {
    let query = params.remove("q");
    let releases = if let Some(ref q) = query {
        let response = search_flakes(&state.opensearch, q).await?;
        // TODO: Remove this unwrap, use fold or map to create the HashMap
        let mut hits: HashMap<i32, f64> = HashMap::new();
        for hit in response["hits"]["hits"].as_array().unwrap() {
            // TODO: properly handle errors
            hits.insert(
                hit["_id"].as_str().unwrap().parse().unwrap(),
                hit["_score"].as_f64().unwrap(),
            );
        }

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

async fn post_publish() -> &'static str {
    "Publish"
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use serde_json::Value;
    use sqlx::postgres::PgConnectOptions;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_get_flake_with_params() {
        let host = env::var("PGHOST").unwrap().to_string();
        let opts = PgConnectOptions::new().host(&host);
        let pool = PgPoolOptions::new().connect_with(opts).await.unwrap();
        let state = Arc::new(AppState {
            opensearch: OpenSearch::default(),
            pool,
        });
        let app = app(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/flake?q=search")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&body).unwrap();
        println!("#{body}");
        // assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_get_flake_without_params() {
        let host = env::var("PGHOST").unwrap().to_string();
        let opts = PgConnectOptions::new().host(&host);
        let pool = PgPoolOptions::new().connect_with(opts).await.unwrap();
        let state = Arc::new(AppState {
            opensearch: OpenSearch::default(),
            pool,
        });
        let app = app(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/flake")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
