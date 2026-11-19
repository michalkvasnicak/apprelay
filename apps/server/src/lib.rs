#[cfg(unix)]
pub mod agent;

#[cfg(unix)]
pub mod cleanup;

pub mod db;
pub mod error;
#[cfg(unix)]
pub mod http;
#[cfg(unix)]
mod r2;
mod secrets;
pub mod telemetry;
#[cfg(unix)]
pub mod vpn_reconciliation;

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

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod ios_app_install;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod peer_identity;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod phone;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod temporal_install;
