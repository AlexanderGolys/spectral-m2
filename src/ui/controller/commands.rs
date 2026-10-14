//! Commands exposed by the `/` picker and process-ending input handled locally.

use crossterm::event::KeyCode;
use ratatui::text::{Line, Span};

use super::picker::{Picker, PickerInput, PickerItem};
use super::settings::keymap::Keymaps;
use super::settings::theme::groups::scheme;
use super::{Editor, EditorEvent, OpenTarget, Reaction, ViewKind};
use crate::ui::action::Action;
use crate::ui::key::KeyMod::Alt;

/// Prefix that turns input text into a live command query.
pub(crate) const PREFIX: &str = "/";

/// A bare input that must be handled without sending it through the M2 socket.
pub(crate) enum SubmittedCommand {
    Quit,
    Restart,
}

pub(crate) fn submitted_command(input: &str) -> Option<SubmittedCommand> {
    // @##Issue nothing special about this char in particular
    if input.contains('\n') {
        return None;
    }
    // @##Issue this just won't work! To check that we need to parse the cell
    // and check every Ident literal tokens. And it's also not the greatest
    // solution, what if someone writes "help restart"?
    match input.trim().trim_end_matches(';').trim_end() {
        "exit" | "quit" => Some(SubmittedCommand::Quit),
        "restart" => Some(SubmittedCommand::Restart),
        _ => None,
    }
}

/// Operation executed by a command-picker entry.
type CommandCallback = fn(&mut Editor, OpenTarget) -> Reaction;

/// One command-picker entry and the callback run when it is selected.
#[derive(Clone, Copy)]
pub struct Command {
    label: &'static str,
    shortcut: Action,
    /// Opens a view, beside the session unless asked to open it in place.
    opens_view: bool,
    callback: CommandCallback,
}

impl Command {
    const fn new(
        label: &'static str,
        shortcut: Action,
        opens_view: bool,
        callback: CommandCallback,
    ) -> Self {
        Self {
            label,
            shortcut,
            opens_view,
            callback,
        }
    }

    fn matches(self, query: &str) -> bool {
        self.label.starts_with(query)
            || self
                .label
                .split_whitespace()
                .any(|word| word.starts_with(query))
    }

    fn item(self, keymaps: &Keymaps) -> Box<dyn PickerItem> {
        let shortcut = keymaps
            .key_for(self.shortcut)
            .unwrap_or_else(|| "unbound".to_string());
        Box::new(CommandItem {
            command: self,
            shortcut,
        })
    }

    pub(crate) fn run(self, editor: &mut Editor, target: OpenTarget) -> Reaction {
        (self.callback)(editor, target)
    }

    #[cfg(test)]
    pub(crate) const fn label(self) -> &'static str {
        self.label
    }

    fn literal(self) -> String {
        format!("/{0}", self.label)
    }
}

/// A command rendered as one picker line and invoked by Enter.
struct CommandItem {
    command: Command,
    shortcut: String,
}

impl CommandItem {
    const IN_PLACE: &str = " open in place";
}

impl PickerItem for CommandItem {
    fn cells(&self) -> Vec<Line<'static>> {
        let groups = scheme();
        let mut hint = vec![Span::styled(self.shortcut.clone(), groups.prompt())];
        if self.command.opens_view {
            hint.extend([
                Span::styled(" · ", groups.muted()),
                Span::styled((KeyCode::Char('v') + Alt).to_string(), groups.prompt()),
                Span::styled(Self::IN_PLACE, groups.muted()),
            ]);
        }
        vec![
            Span::styled(self.command.literal(), groups.prompt()).into(),
            Span::styled(self.command.shortcut.description(), groups.value()).into(),
            hint.into(),
        ]
    }

    fn handle(&mut self, input: PickerInput) -> Option<EditorEvent> {
        let target = match input {
            PickerInput::Action(Action::Submit) => OpenTarget::Split,
            PickerInput::Key(key) => OpenTarget::alternative(key, self.command.opens_view)?,
            _ => return None,
        };
        Some(EditorEvent::RunCommand(self.command, target))
    }
}

const COMMANDS: [Command; 9] = [
    Command::new("settings", Action::OpenSettings, false, open_settings),
    Command::new("keymaps", Action::OpenKeymaps, false, open_keymaps),
    Command::new("clear", Action::ClearFeed, false, clear),
    Command::new("code", Action::OpenCode, true, code),
    Command::new("output", Action::OpenOutput, true, output),
    Command::new("ast", Action::OpenSyntaxTree, true, syntax_tree),
    Command::new("new", Action::Restart, false, restart),
    Command::new("quit", Action::Quit, false, quit),
    Command::new("debug", Action::ToggleHighlightDebug, false, debug),
];
const CLOSE: Command = Command::new("close view", Action::ClosePane, false, close);

pub(crate) fn shortcut(action: Action) -> Option<Command> {
    COMMANDS
        .iter()
        .chain([&CLOSE])
        .find(|command| command.shortcut == action)
        .copied()
}

pub(crate) fn matching(query: &str, split: bool) -> Vec<Command> {
    let query = query.to_lowercase();
    COMMANDS
        .iter()
        .chain(split.then_some(&CLOSE))
        .copied()
        .filter(|command| command.matches(&query))
        .collect()
}

pub(crate) fn query(input: &str) -> Option<&str> {
    let query = input.strip_prefix(PREFIX)?;
    (!query.chars().any(char::is_whitespace)).then_some(query)
}

pub(crate) fn exact(input: &str, split: bool) -> Option<Command> {
    let label = query(input)?;
    matching(label, split)
        .into_iter()
        .find(|command| command.label == label)
}

pub fn picker(commands: &[Command], keymaps: &Keymaps) -> Picker {
    Picker::new(
        "commands",
        commands
            .iter()
            .copied()
            .map(|command| command.item(keymaps))
            .collect(),
    )
}

fn open_settings(editor: &mut Editor, _: OpenTarget) -> Reaction {
    editor.open_settings_from_commands();
    Reaction::Continue
}

fn open_keymaps(editor: &mut Editor, _: OpenTarget) -> Reaction {
    editor.open_keymaps_from_commands();
    Reaction::Continue
}

fn clear(editor: &mut Editor, _: OpenTarget) -> Reaction {
    editor.clear_feed();
    Reaction::Continue
}

fn debug(editor: &mut Editor, _: OpenTarget) -> Reaction {
    editor.close_pickers();
    editor.toggle_highlight_debug()
}

macro_rules! views {
    ($($name:ident: $view:ident),+ $(,)?) => {$(
        fn $name(editor: &mut Editor, target: OpenTarget) -> Reaction {
            editor.close_pickers();
            Reaction::Open(ViewKind::$view, target)
        }
    )+};
}

macro_rules! finished {
    ($($name:ident: $action:ident),+ $(,)?) => {$(
        fn $name(editor: &mut Editor, _: OpenTarget) -> Reaction {
            editor.close_pickers();
            Reaction::$action
        }
    )+};
}

views!(syntax_tree: SyntaxTree, code: Code, output: Output);
finished!(close: ClosePane, restart: Restart, quit: Quit);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::controller::settings::theme::groups::Groups;
    use crate::ui::key::KeyMod::Ctrl;

    #[test]
    fn bare_process_commands_are_intercepted() {
        assert!(matches!(
            submitted_command("exit"),
            Some(SubmittedCommand::Quit)
        ));
        assert!(matches!(
            submitted_command(" quit ;"),
            Some(SubmittedCommand::Quit)
        ));
        assert!(matches!(
            submitted_command("restart"),
            Some(SubmittedCommand::Restart)
        ));
    }

    #[test]
    fn macro_delimiters_and_m2_code_are_not_commands() {
        assert!(submitted_command("$code").is_none());
        assert!(submitted_command("$help").is_none());
        assert!(submitted_command("exit 1").is_none());
        assert!(submitted_command("f = () -> exit").is_none());
        assert!(submitted_command("restart\n1 + 1").is_none());
    }

    #[test]
    fn picker_entries_filter_by_word_prefix_in_the_visible_name() {
        assert_eq!(matching("settings", false).len(), 1);
        assert_eq!(matching("co", false).len(), 1);
        assert_eq!(matching("ast", false).len(), 1);
        assert_eq!(matching("ode", false).len(), 0);
        assert!(matching("does-not-exist", false).is_empty());
        assert!(matching("close", false).is_empty());
        assert_eq!(matching("close", true).len(), 1);
        assert!(exact("/new", false).unwrap().shortcut == Action::Restart);
        assert!(exact("/sett", false).is_none());
    }

    #[test]
    fn neither_a_second_session_nor_a_separate_style_picker_is_a_command() {
        assert!(exact("/split", false).is_none());
        assert!(exact("/style", false).is_none());
    }

    #[test]
    fn new_restarts_the_session() {
        let mut editor = Editor::new(String::new());

        assert!(matches!(
            exact("/new", false)
                .unwrap()
                .run(&mut editor, OpenTarget::Split),
            Reaction::Restart
        ));
    }

    #[test]
    fn every_command_has_a_resolvable_keymap() {
        let keymaps = Keymaps::default();

        for command in matching("", true) {
            let action = command.shortcut;
            assert!(keymaps.key_for(action).is_some(), "{}", command.label);
            assert_eq!(shortcut(action).unwrap().label, command.label);
            assert_eq!(
                command.item(&keymaps).cells()[1].to_string(),
                action.description()
            );
        }
        assert_eq!(exact("/ast", false).unwrap().label(), "ast");
    }

    #[test]
    fn command_metadata_stays_beside_its_label() {
        let cells = matching("code", false)[0].item(&Keymaps::default()).cells();
        let alt_c = (KeyCode::Char('c') + Alt).to_string();
        let alt_v = (KeyCode::Char('v') + Alt).to_string();

        assert_eq!(cells[0].to_string(), "/code");
        assert_eq!(cells[0].spans[0].style.fg, Groups::default().prompt().fg);
        assert_eq!(cells[1].to_string(), Action::OpenCode.description());
        assert_eq!(
            cells[2].to_string(),
            format!("{alt_c} · {alt_v} open in place")
        );
    }

    #[test]
    fn a_view_opens_beside_the_session_unless_asked_to_open_in_place() {
        let keymaps = Keymaps::default();
        let code = matching("code", false)[0];
        let mut item = code.item(&keymaps);
        assert!(matches!(
            item.handle(PickerInput::Action(Action::Submit)),
            Some(EditorEvent::RunCommand(_, OpenTarget::Split))
        ));
        assert!(matches!(
            item.handle(PickerInput::Key(KeyCode::Char('v') + Alt)),
            Some(EditorEvent::RunCommand(_, OpenTarget::InPlace))
        ));

        let settings = matching("settings", false)[0];
        let mut item = settings.item(&keymaps);
        assert!(item
            .handle(PickerInput::Key(KeyCode::Char('v') + Alt))
            .is_none());
        assert!(item
            .handle(PickerInput::Key(KeyCode::Enter + Ctrl))
            .is_none());
    }
}
