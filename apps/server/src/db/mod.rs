pub(crate) mod artifacts;
pub mod devices;
pub mod pairing_attempts;
pub mod pairing_codes;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) mod pairings;
pub mod push_tokens;

use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use std::time::Duration;

pub static MIGRATOR: Migrator = sqlx::migrate!();

pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(10))
        .connect(database_url)
        .await
}
