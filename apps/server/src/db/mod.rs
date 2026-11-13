pub mod devices;
#[cfg(target_os = "macos")]
pub(crate) mod pairings;

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
