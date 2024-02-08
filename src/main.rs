// This starter uses the `axum` crate to create an asynchronous web server
// The async runtime being used, is `tokio`
// This starter also has logging, powered by `tracing` and `tracing-subscriber`

use std::net::SocketAddr;

use axum::{extract::{MatchedPath, Request}, middleware::{self, Next}, response::IntoResponse, routing::get, Router, Json};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use std::{
    future::ready,
    time::{Instant},
};
use std::time::Duration;
use axum::error_handling::HandleErrorLayer;

use tower::{BoxError, ServiceBuilder};
use tower_http::trace::TraceLayer;
use axum::http::StatusCode;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn metrics_app() -> Router {
    let recorder_handle = setup_metrics_recorder();
    Router::new().route("/metrics", get(move || ready(recorder_handle.render())))
}

fn main_app() -> Router {
    // Then, we create a router, which is a way of routing requests to different handlers
    let app = Router::new()
        // In order to add a route, we use the `route` method on the router
        // The `route` method takes a path (as a &str), and a handler (MethodRouter)
        // In our invocation below, we create a route, that goes to "/"
        // We specify what HTTP method we want to accept on the route (via the `get` function)
        // And finally, we provide our route handler
        // The code of the root function is below
        .route("/", get(root))
        // This can be repeated as many times as you want to create more routes
        // We are also going to create a more complex route, using `impl IntoResponse`
        // The code of the complex function is below
        .route("/complex", get(complex))
        .route_layer(middleware::from_fn(track_metrics))
        // Add middleware to all routes
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(|error: BoxError| async move {
                    if error.is::<tower::timeout::error::Elapsed>() {
                        Ok(StatusCode::REQUEST_TIMEOUT)
                    } else {
                        Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("Unhandled internal error: {error}"),
                        ))
                    }
                }))
                .timeout(Duration::from_secs(10))
                .layer(TraceLayer::new_for_http())
                .into_inner(),
        );
    app
}

async fn start_main_server() {
    let app = main_app();

    let port: u16 = std::env::var("PORT")
        .unwrap_or("3000".into())
        .parse()
        .expect("failed to convert to number");
    // We then create a socket address, listening on 0.0.0.0:PORT
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

async fn start_metrics_server() {
    let app = metrics_app();

    // NOTE: expose metrics endpoint on a different port
    let port: u16 = std::env::var("PORT_PROMETHEUS")
        .unwrap_or("3001".into())
        .parse()
        .expect("failed to convert to number");
    // We then create a socket address, listening on 0.0.0.0:PORT
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}



#[tokio::main]
async fn main() {
    // First, we initialize the tracing subscriber with default configuration
    // This is what allows us to print things to the console
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "example_todos=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // The `/metrics` endpoint should not be publicly available. If behind a reverse proxy, this
    // can be achieved by rejecting requests to `/metrics`. In this example, a second server is
    // started on another port to expose `/metrics`.
    let (_main_server, _metrics_server) = tokio::join!(start_main_server(), start_metrics_server());

    tracing::info!("listening on main {:?}", _main_server);
    tracing::info!("listening on metrics {:?}", _metrics_server);
}

// This is our route handler, for the route root
// Make sure the function is `async`
// We specify our return type, `&'static str`, however a route handler can return anything that implements `IntoResponse`

async fn root() -> &'static str {
    "Hello, World!"
}

// This is our route handler, for the route complex
// Make sure the function is async
// We specify our return type, this time using `impl IntoResponse`

async fn complex() -> impl IntoResponse {
    // For this route, we are going to return a Json response
    // We create a tuple, with the first parameter being a `StatusCode`
    // Our second parameter, is the response body, which in this example is a `Json` instance
    // We construct data for the `Json` struct using the `serde_json::json!` macro
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "message": "Hello, World!"
        })),
    )
}



fn setup_metrics_recorder() -> PrometheusHandle {
    const EXPONENTIAL_SECONDS: &[f64] = &[
        0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
    ];

    PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full("http_requests_duration_seconds".to_string()),
            EXPONENTIAL_SECONDS,
        )
        .unwrap()
        .install_recorder()
        .unwrap()
}

async fn track_metrics(req: Request, next: Next) -> impl IntoResponse {
    let start = Instant::now();
    let path = if let Some(matched_path) = req.extensions().get::<MatchedPath>() {
        matched_path.as_str().to_owned()
    } else {
        req.uri().path().to_owned()
    };
    let method = req.method().clone();

    let response = next.run(req).await;

    let latency = start.elapsed().as_secs_f64();
    let status = response.status().as_u16().to_string();

    let labels = [
        ("method", method.to_string()),
        ("path", path),
        ("status", status),
    ];

    metrics::counter!("http_requests_total", &labels).increment(1);
    metrics::histogram!("http_requests_duration_seconds", &labels).record(latency);

    response
}