use std::path::StripPrefixError;

use rmcp::{
    handler::server::tool::IntoCallToolResult,
    model::{CallToolResult, ErrorData, IntoContents},
};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("RMCP error: {0}")]
    Rmcp(rmcp::Error),

    // #[error("HTTP client error: {0}")]
    // HttpClient(#[from] reqwest::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Internal server error: {0}")]
    InternalServerError(String),
}

impl From<anyhow::Error> for AppError {
    fn from(error: anyhow::Error) -> Self {
        AppError::InternalServerError(error.to_string())
    }
}

impl From<StripPrefixError> for AppError {
    fn from(value: StripPrefixError) -> Self {
        AppError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, value))
    }
}

impl From<rmcp::Error> for AppError {
    fn from(value: rmcp::Error) -> Self {
        AppError::Rmcp(value)
    }
}

impl IntoContents for AppError {
    fn into_contents(self) -> Vec<rmcp::model::Content> {
        match self {
            // AppError::Database(err) => vec![rmcp::model::Content::text(format!("Database error: {}", err))],
            AppError::Rmcp(err) => vec![rmcp::model::Content::text(format!("RMCP error: {}", err))],
            // AppError::HttpClient(err) => vec![rmcp::model::Content::text(format!("HTTP client error: {}", err))],
            AppError::Io(err) => vec![rmcp::model::Content::text(format!("IO error: {}", err))],
            // AppError::Git(err) => vec![rmcp::model::Content::text(format!("Git error: {}", err))],
            AppError::NotFound(msg) => {
                vec![rmcp::model::Content::text(format!("Not found: {}", msg))]
            }
            AppError::BadRequest(msg) => {
                vec![rmcp::model::Content::text(format!("Bad request: {}", msg))]
            }
            AppError::InternalServerError(msg) => vec![rmcp::model::Content::text(format!(
                "Internal server error: {}",
                msg
            ))],
        }
    }
}

// impl IntoCallToolResult for AppError {
//     fn into_call_tool_result(self) -> CallToolResult {
//         CallToolResult::failure(self.into_contents())
//     }
// }

// Newtype wrapper for Result<CallToolResult, AppError>
pub struct AppResultWrapper(pub Result<CallToolResult, AppError>);

impl IntoCallToolResult for AppResultWrapper {
    fn into_call_tool_result(self) -> Result<rmcp::model::CallToolResult, ErrorData> {
        match self.0 {
            Ok(res) => Ok(res),
            Err(e) => e.into_call_tool_result(),
        }
    }
}

// impl IntoCallToolResult for Result<CallToolResult, AppError> {
//     fn into_call_tool_result(self) -> CallToolResult {
//         match self {
//             Ok(res) => res,
//             Err(e) => e.into_call_tool_result(),
//         }
//     }
// }

pub type AppResult<T> = Result<T, AppError>;
