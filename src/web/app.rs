use crate::config::Config;
use crate::web::handler::webhook::*;
use anyhow::Result;
use axum::{extract::Extension, routing::post, Router};
use std::sync::Arc;
use tower::ServiceBuilder;
use tower_http::{
    cors::CorsLayer,
    trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer},
    LatencyUnit,
};
use tracing::info;
use tracing::Level;

#[derive(Clone)]
pub struct Context {
    pub config: Arc<Config>,
}

fn new_router() -> Router {
    Router::new()
        .route("/webhooks/github", post(post_webhooks_github))
        .layer(CorsLayer::permissive())
}

pub async fn start_server(config: Config) -> Result<()> {
    let bind_addr = config.bind_addr();
    let ctx = Context {
        config: Arc::new(config),
    };

    let r = new_router().layer(
        ServiceBuilder::new()
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(DefaultMakeSpan::new().include_headers(true))
                    .on_request(DefaultOnRequest::new().level(Level::INFO))
                    .on_response(
                        DefaultOnResponse::new()
                            .level(Level::INFO)
                            .latency_unit(LatencyUnit::Micros),
                    ),
            )
            .layer(Extension(ctx)),
    );

    info!("starting a server");
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, r).await?;

    Ok(())
}
