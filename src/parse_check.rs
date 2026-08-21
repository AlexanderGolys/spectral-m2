use serde::Deserialize;

use crate::errors::{M2Error, M2Result};

#[derive(Debug)]
pub enum ParseVerdict {
    Complete,
    Incomplete,
    SyntaxError(String),
}

/// The reply to a completeness check, including a diagnostic message when the
/// verdict is `ERROR`.
#[derive(Deserialize)]
pub struct ParseReply {
    pub verdict: String,
    pub message: String,
}

impl ParseReply {
    pub fn into_verdict(self) -> M2Result<ParseVerdict> {
        match self.verdict.as_str() {
            "COMPLETE" => Ok(ParseVerdict::Complete),
            "INCOMPLETE" => Ok(ParseVerdict::Incomplete),
            "ERROR" => Ok(ParseVerdict::SyntaxError(self.message)),
            other => Err(M2Error::ParserError(format!(
                "unexpected verdict {other:?}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(verdict: &str, message: &str) -> ParseReply {
        ParseReply {
            verdict: verdict.to_string(),
            message: message.to_string(),
        }
    }

    #[test]
    fn maps_known_verdicts() {
        assert!(matches!(
            reply("COMPLETE", "").into_verdict(),
            Ok(ParseVerdict::Complete)
        ));
        assert!(matches!(
            reply("INCOMPLETE", "").into_verdict(),
            Ok(ParseVerdict::Incomplete)
        ));
        assert!(matches!(
            reply("ERROR", "syntax error").into_verdict(),
            Ok(ParseVerdict::SyntaxError(message)) if message == "syntax error"
        ));
    }

    #[test]
    fn rejects_unknown_verdict() {
        assert!(matches!(
            reply("???", "").into_verdict(),
            Err(M2Error::ParserError(_))
        ));
    }
}
