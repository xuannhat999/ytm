use data::mpv::{MpvCommand, MpvEvent};
use reqwest::header::InvalidHeaderValue;
use std::{
    fmt::{Debug, Display},
    fs::{self, OpenOptions},
    io::Write,
};
use thiserror::Error;
use time::{OffsetDateTime, format_description};

#[derive(Debug, Error)]
pub enum YError {
    #[error("Invalid File Path: {0}")]
    InvalidPath(String),

    #[error("Invalid Response from: {0}")]
    InvalidResponse(String),

    #[error("IO Error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("JSON Error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Request Error: {0}")]
    ReqwestError(#[from] reqwest::Error),

    #[error("Request Header Inval: {0}")]
    InvalidHeader(#[from] InvalidHeaderValue),

    #[error("MPV Socket Error: {0}")]
    MpvSocketError(String),

    #[error("Invalid Cookie")]
    InvalidCookie,

    #[error(
        "Conflicting browser container identities: multiple containers contain active YouTube sessions"
    )]
    ConflictingContainerIdentities,

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("URL parsing failed: {0}")]
    UrlParseError(#[from] url::ParseError),

    #[error("Event Sender Error: {0}")]
    EventSenderError(#[from] tokio::sync::mpsc::error::SendError<MpvEvent>),

    #[error("Command Sender Error: {0}")]
    MpvCmdSenderError(#[from] tokio::sync::mpsc::error::SendError<MpvCommand>),

    #[error("Song alredy saved in playlist")]
    AlreadyInPlaylist,

    #[error("Bad Status from: {0}")]
    BadStatus(String),
}

pub type YResult<T> = std::result::Result<T, YError>;

pub fn startup_error_message(err: &YError) -> String {
    match err {
        YError::ReqwestError(e) => {
            if e.is_connect() || e.is_timeout() {
                "Connection failure: unable to connect to YouTube Music".to_string()
            } else {
                format!("HTTP request error: {e}")
            }
        }
        YError::InvalidCookie => {
            "Authentication error: invalid or expired session cookies".to_string()
        }
        YError::ConflictingContainerIdentities => {
            "Authentication error: multiple conflicting browser container identities found"
                .to_string()
        }
        YError::DatabaseError(msg) => {
            format!("Database error: {msg}")
        }
        _ => {
            format!("Initialization error: {err}")
        }
    }
}

pub fn log_to_file<T: Display>(message: T) {
    if let Some(log_path) = dirs::state_dir().map(|p| p.join("gytm")) {
        if !log_path.exists() {
            let _ = fs::create_dir_all(&log_path);
        }
        let file_path = log_path.join("log.txt");

        let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());

        let format =
            format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second]").unwrap();

        let datetime = now.format(&format).unwrap_or_default();

        let max_size = 5 * 1024 * 1024;
        let is_oversize = fs::metadata(&file_path)
            .map(|meta| meta.len() >= max_size)
            .unwrap_or(false);

        let mut options = OpenOptions::new();
        options.create(true).write(true);

        if is_oversize {
            options.truncate(true);
        } else {
            options.append(true);
        }
        if let Ok(mut file) = options.open(file_path) {
            let _ = writeln!(file, "{} : {}", datetime, message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_startup_error_reporting() {
        let auth_err = YError::InvalidCookie;
        assert_eq!(
            startup_error_message(&auth_err),
            "Authentication error: invalid or expired session cookies"
        );

        let container_err = YError::ConflictingContainerIdentities;
        assert_eq!(
            startup_error_message(&container_err),
            "Authentication error: multiple conflicting browser container identities found"
        );

        let db_err = YError::DatabaseError("failed to create SQLite snapshot".to_string());
        let db_msg = startup_error_message(&db_err);
        assert_eq!(db_msg, "Database error: failed to create SQLite snapshot");
        assert!(
            !db_msg.contains("Connection failure"),
            "Database error must not be mislabeled as network failure"
        );

        let init_err = YError::InvalidResponse("YouTube Music bootstrap data".to_string());
        assert_eq!(
            startup_error_message(&init_err),
            "Initialization error: Invalid Response from: YouTube Music bootstrap data"
        );
    }

    #[test]
    fn test_invalid_cookie_message_contains_no_cookie_data() {
        let err = YError::InvalidCookie;
        let msg = startup_error_message(&err);
        assert_eq!(
            msg,
            "Authentication error: invalid or expired session cookies"
        );
    }

    #[test]
    fn test_database_error_message_preserves_context() {
        let context = "query execution failed at index 3";
        let err = YError::DatabaseError(context.to_string());
        let msg = startup_error_message(&err);
        assert_eq!(msg, format!("Database error: {context}"));
        assert_eq!(format!("{err}"), format!("Database error: {context}"));
    }
}
