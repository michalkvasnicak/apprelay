//! Operation context that preserves an error without putting it in the message.

use std::{error::Error, io};

#[derive(Debug, thiserror::Error)]
#[error("{operation}")]
pub struct OperationError {
    pub(crate) operation: &'static str,
    #[source]
    cause: Option<Box<dyn Error + Send + Sync>>,
}

impl OperationError {
    pub fn at<E>(operation: &'static str) -> impl FnOnce(E) -> Self
    where
        E: Into<Box<dyn Error + Send + Sync>>,
    {
        move |cause| Self {
            operation,
            cause: Some(cause.into()),
        }
    }

    pub(crate) fn without_source(operation: &'static str) -> Self {
        Self {
            operation,
            cause: None,
        }
    }

    /// A fixed code from the deepest recognized cause, never a raw message.
    pub(crate) fn cause_code(&self) -> &'static str {
        let mut code = "unclassified";
        let mut cause = self.source();
        while let Some(error) = cause {
            if let Some(error) = error.downcast_ref::<sqlx::Error>() {
                return database_cause(error);
            }
            if let Some(error) = error.downcast_ref::<io::Error>() {
                code = io_cause(error);
            }
            cause = error.source();
        }
        code
    }
}

pub(crate) fn database_cause(error: &sqlx::Error) -> &'static str {
    match error {
        sqlx::Error::PoolTimedOut => "db_pool_timeout",
        sqlx::Error::PoolClosed => "db_pool_closed",
        sqlx::Error::Io(_) => "db_io",
        sqlx::Error::Database(_) => "db_server_error",
        sqlx::Error::Tls(_) => "db_tls",
        _ => "db_other",
    }
}

pub(crate) fn io_cause(error: &io::Error) -> &'static str {
    match error.kind() {
        io::ErrorKind::TimedOut => "io_timeout",
        io::ErrorKind::PermissionDenied => "io_permission_denied",
        io::ErrorKind::NotFound => "io_not_found",
        io::ErrorKind::AddrInUse => "io_address_in_use",
        io::ErrorKind::ConnectionRefused => "io_connection_refused",
        io::ErrorKind::InvalidData => "io_invalid_data",
        io::ErrorKind::InvalidInput => "io_invalid_input",
        _ => "io_other",
    }
}
