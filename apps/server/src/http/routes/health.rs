use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub(in crate::http) struct Health {
    status: &'static str,
}

#[utoipa::path(
    get,
    path = "/health",
    // Preserve the OpenAPI tag from before the handler moved into this module.
    tag = "super",
    responses((status = 200, body = Health))
)]
pub(in crate::http) async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}
