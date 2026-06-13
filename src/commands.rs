//! Frontend REPL commands: `$`-prefixed lines handled entirely in the TUI and
//! never sent to M2. See the `REPL Command Prefix` decision and `REPL Commands`
//! plan in the Obsidian docs.
//!
//! This is the minimal first slice: parse a fresh, single-line input whose first
//! character is `$` into a [`ReplCommand`]. The completion picker and the `\$`
//! escape-peel (for code that must legitimately begin with `$`) are deferred.

/// The character that, as the first byte of a fresh input, marks a REPL command.
pub const COMMAND_PREFIX: char = '$';

/// A recognised REPL command, or an unrecognised name to report back.
pub enum ReplCommand {
    /// `$code` — toggle the split pane showing this session's committed code.
    ToggleCode,
    /// `$name` with no matching handler; carries the bare name for the message.
    Unknown(String),
}

/// Parse `input` as a REPL command iff its first character is [`COMMAND_PREFIX`].
/// Returns `None` for ordinary M2 code. The caller is responsible for only
/// offering a *fresh, single-line* input (commands are not recognised mid-block).
pub fn parse_command(input: &str) -> Option<ReplCommand> {
    let rest = input.strip_prefix(COMMAND_PREFIX)?;
    let name = rest.split_whitespace().next().unwrap_or("");
    Some(match name {
        "code" => ReplCommand::ToggleCode,
        other => ReplCommand::Unknown(other.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_code_is_not_a_command() {
        assert!(parse_command("1 + 1").is_none());
        assert!(parse_command("R = QQ[x,y]").is_none());
    }

    #[test]
    fn dollar_code_toggles() {
        assert!(matches!(
            parse_command("$code"),
            Some(ReplCommand::ToggleCode)
        ));
    }

    #[test]
    fn unknown_command_keeps_its_name() {
        match parse_command("$theme dark") {
            Some(ReplCommand::Unknown(name)) => assert_eq!(name, "theme"),
            _ => panic!("expected Unknown"),
        }
    }
}
