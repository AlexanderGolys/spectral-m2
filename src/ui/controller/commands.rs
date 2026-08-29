//! Commands exposed by the `/` picker and process-ending input handled locally.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::picker::{
    value_spans, Line, LineContext, Picker, PickerItem, PickerStatus, TextComponent,
};
use super::settings::{Action, Keymap};
use super::{Editor, EditorAction, EditorEvent, OpenTarget, ViewKind};

/// A bare input that must be handled without sending it through the M2 socket.
pub enum SubmittedCommand {
    Quit,
    Restart,
}

pub fn submitted_command(input: &str) -> Option<SubmittedCommand> {
    if input.contains('\n') {
        return None;
    }
    match input.trim().trim_end_matches(';').trim_end() {
        "exit" | "quit" => Some(SubmittedCommand::Quit),
        "restart" => Some(SubmittedCommand::Restart),
        _ => None,
    }
}

/// Operation executed by a command-picker entry.
type CommandCallback = fn(&mut Editor, OpenTarget) -> EditorAction;

/// One command-picker entry and the callback run when it is selected.
#[derive(Clone, Copy)]
pub struct Command {
    label: &'static str,
    shortcut: Option<Action>,
    splittable: bool,
    callback: CommandCallback,
}

impl Command {
    const fn new(
        label: &'static str,
        shortcut: Option<Action>,
        splittable: bool,
        callback: CommandCallback,
    ) -> Self {
        Self {
            label,
            shortcut,
            splittable,
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

    fn item(self, keymap: &Keymap, split: bool) -> Box<dyn PickerItem> {
        Box::new(CommandItem {
            command: self,
            hint: if self.splittable && !split {
                "ctrl+enter open in split view".to_string()
            } else {
                self.shortcut
                    .and_then(|action| keymap.key_for(action))
                    .unwrap_or_default()
            },
            can_split: self.splittable && !split,
        })
    }

    pub fn run(self, editor: &mut Editor, target: OpenTarget) -> EditorAction {
        (self.callback)(editor, target)
    }

    pub const fn label(self) -> &'static str {
        self.label
    }
}

/// A command rendered as one picker line and invoked by Enter.
struct CommandItem {
    command: Command,
    hint: String,
    can_split: bool,
}

impl TextComponent for CommandItem {
    fn spans(&self, context: &LineContext<'_>) -> Vec<ratatui::text::Span<'static>> {
        value_spans(self.command.label, &self.hint, true, context)
    }
}

impl Line for CommandItem {}

impl PickerItem for CommandItem {
    fn handle_key(&mut self, key: KeyEvent) -> Option<EditorEvent> {
        if key.code != KeyCode::Enter {
            return None;
        }
        let target = if key.modifiers.contains(KeyModifiers::CONTROL) && self.can_split {
            OpenTarget::Split
        } else if key.modifiers.contains(KeyModifiers::CONTROL) {
            return None;
        } else {
            OpenTarget::Current
        };
        Some(EditorEvent::RunCommand(self.command, target))
    }
}

const COMMANDS: [Command; 11] = [
    Command::new("settings", Some(Action::OpenSettings), false, open_settings),
    Command::new("style", None, false, style),
    Command::new("keymaps", None, false, open_keymaps),
    Command::new("clear", None, false, clear),
    Command::new("code", None, true, code),
    Command::new("new", None, true, new),
    Command::new("output", None, true, output),
    Command::new("syntax tree", None, true, syntax_tree),
    Command::new("restart", None, false, restart),
    Command::new("quit", Some(Action::Quit), false, quit),
    Command::new("debug", Some(Action::ToggleHighlightDebug), false, debug),
];
const CLOSE: Command = Command::new("close focused window", None, false, close);

pub fn matching(query: &str, split: bool) -> Vec<Command> {
    let query = query.to_lowercase();
    COMMANDS
        .iter()
        .chain(split.then_some(&CLOSE))
        .copied()
        .filter(|command| command.matches(&query))
        .collect()
}

pub fn exact(input: &str, split: bool) -> Option<Command> {
    let label = input.strip_prefix('/')?;
    (!label.contains('\n'))
        .then(|| matching(label.trim(), split))?
        .into_iter()
        .find(|command| command.label == label.trim())
}

fn picker_with_status(
    commands: &[Command],
    keymap: &Keymap,
    split: bool,
    status: PickerStatus,
) -> Picker {
    Picker::new(
        "commands",
        commands
            .iter()
            .copied()
            .map(|command| command.item(keymap, split))
            .collect(),
        status,
    )
}

pub fn filtered_picker(commands: &[Command], keymap: &Keymap, split: bool) -> Picker {
    picker_with_status(
        commands,
        keymap,
        split,
        PickerStatus::new("/ ", "type to filter · ↑↓ select · enter run · esc close"),
    )
}

pub fn picker(commands: &[Command], keymap: &Keymap, split: bool) -> Picker {
    picker_with_status(
        commands,
        keymap,
        split,
        PickerStatus::new("commands ", "↑↓ select · enter run · esc close"),
    )
}

fn open_settings(editor: &mut Editor, _: OpenTarget) -> EditorAction {
    editor.open_settings_from_commands();
    EditorAction::Continue
}

fn style(editor: &mut Editor, _: OpenTarget) -> EditorAction {
    editor.open_style_from_commands();
    EditorAction::Continue
}

fn open_keymaps(editor: &mut Editor, _: OpenTarget) -> EditorAction {
    editor.open_keymaps_from_commands();
    EditorAction::Continue
}

fn clear(editor: &mut Editor, _: OpenTarget) -> EditorAction {
    editor.clear_feed();
    EditorAction::Continue
}

fn debug(editor: &mut Editor, _: OpenTarget) -> EditorAction {
    editor.finish_command();
    editor.toggle_highlight_debug();
    EditorAction::Continue
}

fn open(editor: &mut Editor, target: OpenTarget, view: ViewKind) -> EditorAction {
    editor.finish_command();
    EditorAction::Open(view, target)
}

fn new(editor: &mut Editor, target: OpenTarget) -> EditorAction {
    editor.finish_command();
    EditorAction::New(target)
}

fn syntax_tree(editor: &mut Editor, target: OpenTarget) -> EditorAction {
    open(editor, target, ViewKind::SyntaxTree)
}

fn code(editor: &mut Editor, target: OpenTarget) -> EditorAction {
    open(editor, target, ViewKind::Code)
}

fn output(editor: &mut Editor, target: OpenTarget) -> EditorAction {
    open(editor, target, ViewKind::Output)
}

fn close(editor: &mut Editor, _: OpenTarget) -> EditorAction {
    editor.finish_command();
    EditorAction::ClosePane
}

fn restart(editor: &mut Editor, _: OpenTarget) -> EditorAction {
    editor.finish_command();
    EditorAction::Restart
}

fn quit(editor: &mut Editor, _: OpenTarget) -> EditorAction {
    editor.finish_command();
    EditorAction::Quit
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(matching("syntax t", false).len(), 1);
        assert_eq!(matching("ode", false).len(), 0);
        assert!(matching("does-not-exist", false).is_empty());
        assert!(matching("close", false).is_empty());
        assert_eq!(matching("close", true).len(), 1);
        assert_eq!(matching("style", false).len(), 1);
        assert_eq!(exact("/style", false).unwrap().label(), "style");
        assert!(exact("/sty", false).is_none());
    }

    #[test]
    fn ctrl_enter_requests_a_split_only_for_splittable_unsplit_items() {
        let keymap = Keymap::default();
        let code = matching("code", false)[0];
        let mut item = code.item(&keymap, false);
        assert!(matches!(
            item.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL)),
            Some(EditorEvent::RunCommand(_, OpenTarget::Split))
        ));

        let mut item = code.item(&keymap, true);
        assert!(item
            .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL))
            .is_none());
        let settings = matching("settings", false)[0];
        let mut item = settings.item(&keymap, false);
        assert!(item
            .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL))
            .is_none());
    }
}
