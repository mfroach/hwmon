use axum::{
    extract::State,
    http::{header, StatusCode},
    response::Json,
    routing::get,
    Router,
};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};

use hwmond::{collect, SensorData};

/// Application state shared across request handlers.
#[derive(Clone)]
struct AppState {
    /// Most recently collected sensor data.
    data: Arc<RwLock<SensorData>>,
}

#[tokio::main]
async fn main() {
    // Pre-warm sensors once so the first API call is instant.
    let data = Arc::new(RwLock::new(collect()));

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/", get(index))
        .route("/api", get(get_sensors))
        .route("/health", get(health))
        .layer(cors)
        .with_state(AppState { data });

    let addr = "0.0.0.0:9899";
    println!("hwmond listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind to port 9899");

    axum::serve(listener, app)
        .await
        .expect("Server error");
}

async fn get_sensors(State(state): State<AppState>) -> Json<SensorData> {
    // If data is stale (> 3s), re-collect in background.
    let data = state.data.read().await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let snapshot = data.clone();
    drop(data);

    if now - snapshot.timestamp > 3 {
        let state = state.clone();
        tokio::spawn(async move {
            let new_data = collect();
            let _ = state.data.write().await;
            *state.data.write().await = new_data;
        });
    }

    Json(snapshot)
}

async fn health() -> (StatusCode, &'static str) {
    (StatusCode::OK, "ok")
}

/// Serve the dashboard HTML.
async fn index() -> (
    [(header::HeaderName, header::HeaderValue); 2],
    &'static str,
) {
    (
        [
            (
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            (
                header::CACHE_CONTROL,
                header::HeaderValue::from_static("no-cache"),
            ),
        ],
        include_str!("../../index.html"),
    )
}
