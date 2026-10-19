use serde::{Deserialize, Serialize};

use crate::ErrorCode;

/// Version tag carried by every `--json` CLI response. Distinct from the
/// story schema (`gitstory/v1`), which versions stored stories.
pub const SCHEMA_VERSION: &str = "gitstory.cli/v1";

/// Top-level `--json` response for every command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub schema: String,
    pub command: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
}

impl Response {
    pub fn ok(command: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            schema: SCHEMA_VERSION.to_owned(),
            command: command.into(),
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    /// Error response. `data` carries structured detail when there is any.
    pub fn err(
        command: impl Into<String>,
        code: ErrorCode,
        message: impl Into<String>,
        data: Option<serde_json::Value>,
    ) -> Self {
        Self {
            schema: SCHEMA_VERSION.to_owned(),
            command: command.into(),
            ok: false,
            data,
            error: Some(ErrorBody {
                code,
                message: message.into(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_round_trips() {
        let resp = Response::err("show", ErrorCode::NotFound, "no story", None);
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("\"data\""));
        assert_eq!(serde_json::from_str::<Response>(&json).unwrap(), resp);
    }
}
