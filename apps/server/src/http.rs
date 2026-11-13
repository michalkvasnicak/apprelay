use crate::db::devices::DeviceRepository;
use axum::{Json, Router, routing::get};
use serde::Serialize;

#[derive(Serialize)]
struct Health {
    status: &'static str,
}

pub fn router(devices: DeviceRepository) -> Router {
    Router::new()
        .route("/health", get(health))
        .with_state(devices)
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}
