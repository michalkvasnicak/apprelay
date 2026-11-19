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
// Reconciliation uses the agent client and does not require the local VPN backend.
#[cfg(unix)]
#[path = "vpn/reconciliation.rs"]
pub mod vpn_reconciliation;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod iphone;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod temporal_pairing;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod temporal_notifications;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod apns;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod vpn;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod temporal_install;
