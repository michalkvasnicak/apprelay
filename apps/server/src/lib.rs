#[cfg(unix)]
pub mod agent;

#[cfg(unix)]
pub mod cleanup;

pub mod db;
#[cfg(unix)]
pub mod http;
#[cfg(unix)]
pub mod reconciliation;
mod secrets;
pub mod telemetry;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod pairing;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod temporal_pairing;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod temporal_notifications;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod apns;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod vpn;
