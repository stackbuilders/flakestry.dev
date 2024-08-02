mod api;
mod common;

use axum::{
    extract::{ConnectInfo, Request},
    middleware::{self, Next},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use opensearch::{
    http::StatusCode,
    indices::{IndicesCreateParts, IndicesGetParts},
    OpenSearch,
};
use sqlx::postgres::PgPoolOptions;
use std::{env, net::SocketAddr, sync::Arc};
use tower_http::trace::TraceLayer;
use tracing::{field, info_span, Span};
use tracing_subscriber::{fmt, EnvFilter};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::api::{get_flake, post_publish};
use crate::common::AppState;

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

async fn create_flake_index(opensearch: &OpenSearch) -> Result<(), opensearch::Error> {
    let status = opensearch
        .indices()
        .get(IndicesGetParts::Index(&["flakes"]))
        .send()
        .await?
        .status_code();

    if status == StatusCode::NOT_FOUND {
        let _ = opensearch
            .indices()
            .create(IndicesCreateParts::Index("flakes"))
            .send()
            .await?;
    }

    Ok(())
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
