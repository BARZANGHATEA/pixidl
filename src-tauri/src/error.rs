//! Errors returned to the frontend: classified, human-readable, with an
//! optional technical detail shown behind "Details".

use pixidl_core::types::ErrorKind;
use pixidl_core::DownloadError;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub kind: ErrorKind,
    pub message: String,
    pub detail: Option<String>,
}

impl From<DownloadError> for CommandError {
    fn from(e: DownloadError) -> Self {
        Self { kind: e.kind, message: e.message, detail: e.detail }
    }
}

impl CommandError {
    pub fn msg(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), detail: None }
    }
}

pub type CmdResult<T> = Result<T, CommandError>;
