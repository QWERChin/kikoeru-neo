//! Error codes for the import, so the desktop shell and scripts can react to
//! a reason without parsing English text. The message stays a sentence for
//! people, and the code is the contract (book/src/contract.md, "导入的错误码").

use serde::Serialize;

/// Why an import or a dry run stopped. It is a DTO, because the desktop
/// shell hands it to the page.
pub use neo_core::dto::ImportErrorCode as ErrorCode;

/// An import error with its code. It travels inside `anyhow::Error`, and
/// [`code`] finds it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportError {
    pub code: ErrorCode,
    pub message: String,
}

impl ImportError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ImportError {}

/// The code of the first [`ImportError`] in the chain, or `other`.
pub fn code(err: &anyhow::Error) -> ErrorCode {
    err.chain()
        .find_map(|e| e.downcast_ref::<ImportError>())
        .map_or(ErrorCode::Other, |e| e.code)
}

/// The `{"error": {"code", "message"}}` body of `--json`, the same shape as
/// the HTTP API's errors. The message is the whole chain, as the CLI prints.
pub fn json(err: &anyhow::Error) -> serde_json::Value {
    serde_json::json!({
        "error": {
            "code": code(err),
            "message": format!("{err:#}"),
        }
    })
}

/// `bail!` with a code.
macro_rules! refuse {
    ($code:expr, $($arg:tt)+) => {
        return Err(anyhow::Error::from($crate::import::error::ImportError::new(
            $code,
            format!($($arg)+),
        )))
    };
}
pub(crate) use refuse;

#[cfg(test)]
mod tests {
    use anyhow::Context;

    use super::*;

    #[test]
    fn the_code_survives_context() {
        fn fails() -> anyhow::Result<()> {
            refuse!(ErrorCode::TooOld, "from {}", "0.6.13");
        }
        let err = fails().context("reading the old library").unwrap_err();
        assert_eq!(code(&err), ErrorCode::TooOld);
        let body = json(&err);
        assert_eq!(body["error"]["code"], "too_old");
        assert_eq!(
            body["error"]["message"],
            "reading the old library: from 0.6.13"
        );
        assert_eq!(code(&anyhow::anyhow!("disk full")), ErrorCode::Other);
    }
}
