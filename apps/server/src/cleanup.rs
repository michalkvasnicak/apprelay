use std::{error::Error, time::Duration};

use sqlx::PgPool;
use tokio::time::{MissedTickBehavior, interval};
use uuid::Uuid;

use crate::agent::Client;

/// Runs immediately at startup, then every minute. Failed removals remain in
/// PostgreSQL for retry; no HTTP request is involved.
pub async fn run(pool: PgPool, agent: Client) {
    let mut ticks = interval(Duration::from_secs(60));
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        ticks.tick().await;
        match cleanup_batch(&pool, &agent).await {
            Ok(0) => {}
            Ok(count) => eprintln!("Removed {count} expired device registrations"),
            Err(error) => eprintln!("Expired device cleanup failed; will retry: {error}"),
        }
    }
}

async fn cleanup_batch(pool: &PgPool, agent: &Client) -> Result<u32, Box<dyn Error + Send + Sync>> {
    sqlx::query("DELETE FROM pairing_attempts WHERE expires_at <= clock_timestamp()")
        .execute(pool)
        .await?;
    sqlx::query(
        "DELETE FROM public_pairing_attempts
        WHERE expires_at < clock_timestamp() - INTERVAL '24 hours'",
    )
    .execute(pool)
    .await?;
    let mut removed = 0;
    // Bound each sweep so a large backlog cannot monopolize the agent.
    for _ in 0..100 {
        let mut transaction = pool.begin().await?;
        let device = sqlx::query_as::<_, (Uuid, Vec<u8>)>(
            "SELECT id, wireguard_public_key FROM devices
             WHERE expires_at <= statement_timestamp()
             ORDER BY expires_at, id
             LIMIT 1
             FOR UPDATE SKIP LOCKED",
        )
        .fetch_optional(&mut *transaction)
        .await?;
        let Some((id, public_key)) = device else {
            transaction.commit().await?;
            break;
        };

        // Keep the row locked across peer removal and deletion. Registration
        // upserts must wait, so cleanup cannot remove a renewed registration.
        // Other server instances skip this row while it is being processed.
        let public_key = public_key.as_slice().try_into()?;
        agent.remove_peer(public_key).await?;
        sqlx::query("DELETE FROM devices WHERE id = $1")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        removed += 1;
    }
    Ok(removed)
}
