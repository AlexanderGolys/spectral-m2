//! Frontend commands recognized only for fresh, single-line input and never sent
//! to Macaulay2.

const COMMAND_PREFIX: char = '$';
const HELP_TEXT: &str = "commands:\n  $help  show this help\n  $code  toggle session code pane";

pub enum CommandResult {
    Message(&'static str),
    ToggleCode,
    Unrecognized,
}

pub fn dispatch(input: &str) -> Option<CommandResult> {
    if input.contains('\n') {
        return None;
    }
    let command = input.strip_prefix(COMMAND_PREFIX)?;
    let name = command.split_whitespace().next().unwrap_or_default();
    Some(match name {
        "help" => CommandResult::Message(HELP_TEXT),
        "code" => CommandResult::ToggleCode,
        _ => CommandResult::Unrecognized,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_multiline_code_are_not_commands() {
        assert!(dispatch("1 + 1").is_none());
        assert!(dispatch("R = QQ[x,y]").is_none());
        assert!(dispatch("$help\n1 + 1").is_none());
    }

    #[test]
    fn dispatches_known_commands() {
        assert!(matches!(dispatch("$code"), Some(CommandResult::ToggleCode)));
        assert!(matches!(
            dispatch("$help"),
            Some(CommandResult::Message(HELP_TEXT))
        ));
    }

    #[test]
    fn recognizes_unknown_prefixed_input_without_sending_it_to_m2() {
        assert!(matches!(
            dispatch("$unknown"),
            Some(CommandResult::Unrecognized)
        ));
    }
}
