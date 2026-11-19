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
pub mod vpn;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod iphone;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod temporal_pairing;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod temporal_notifications;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod apns;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod temporal_install;
