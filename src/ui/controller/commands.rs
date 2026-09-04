//! Commands exposed by the `/` picker and process-ending input handled locally.

use crossterm::event::KeyCode;

use super::picker::{Line, LineContext, Picker, PickerInput, PickerItem};
use super::settings::keymap::Keymaps;
use super::{Editor, EditorEvent, OpenTarget, Reaction, ViewKind};
use crate::ui::action::Action;
use crate::ui::element::Text;
use crate::ui::key::KeyMod::Alt;

/// Prefix that turns input text into a live command query.
pub const PREFIX: &str = "/";

/// A bare input that must be handled without sending it through the M2 socket.
pub enum SubmittedCommand {
    Quit,
    Restart,
}

pub fn submitted_command(input: &str) -> Option<SubmittedCommand> {
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
    splittable: bool,
    callback: CommandCallback,
}

impl Command {
    const fn new(
        label: &'static str,
        shortcut: Action,
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

    fn item(self, keymaps: &Keymaps, split: bool) -> Box<dyn PickerItem> {
        let can_split = self.splittable && !split;
        let mut hint = keymaps
            .key_for(self.shortcut)
            .unwrap_or_else(|| "unbound".to_string());
        if can_split {
            hint.push_str(&format!(" · {} alternative view", KeyCode::Char('v') + Alt));
        }
        Box::new(CommandItem {
            command: self,
            hint,
            can_split,
        })
    }

    pub fn run(self, editor: &mut Editor, target: OpenTarget) -> Reaction {
        (self.callback)(editor, target)
    }

    pub const fn label(self) -> &'static str {
        self.label
    }

    fn literal(self) -> String {
        format!("/{0}", self.label)
    }
}

/// A command rendered as one picker line and invoked by Enter.
struct CommandItem {
    command: Command,
    hint: String,
    can_split: bool,
}

impl Text for CommandItem {
    type Context = LineContext;

    fn spans(&self, context: &LineContext) -> Vec<ratatui::text::Span<'static>> {
        let literal = self.command.literal();
        context.columns([
            (&literal, context.groups.prompt()),
            (self.command.shortcut.description(), context.groups.value()),
            (&self.hint, context.groups.value()),
        ])
    }
}

impl Line for CommandItem {
    fn column_widths(&self) -> [usize; 3] {
        [
            self.command.label.len() + 1,
            self.command.shortcut.description().len(),
            self.hint.len(),
        ]
    }
}

impl PickerItem for CommandItem {
    fn handle(&mut self, input: PickerInput) -> Option<EditorEvent> {
        let target = match input {
            PickerInput::Action(Action::Submit) => OpenTarget::Current,
            PickerInput::Key(key) => OpenTarget::alternative(key, self.can_split)?,
            _ => return None,
        };
        Some(EditorEvent::RunCommand(self.command, target))
    }
}

const COMMANDS: [Command; 11] = [
    Command::new("settings", Action::OpenSettings, false, open_settings),
    Command::new("style", Action::OpenStyle, false, style),
    Command::new("keymaps", Action::OpenKeymaps, false, open_keymaps),
    Command::new("clear", Action::ClearFeed, false, clear),
    Command::new("code", Action::OpenCode, true, code),
    Command::new("split", Action::NewWorkspace, false, split),
    Command::new("output", Action::OpenOutput, true, output),
    Command::new("ast", Action::OpenSyntaxTree, true, syntax_tree),
    Command::new("new", Action::Restart, false, restart),
    Command::new("quit", Action::Quit, false, quit),
    Command::new("debug", Action::ToggleHighlightDebug, false, debug),
];
const CLOSE: Command = Command::new("close focused window", Action::ClosePane, false, close);

pub fn shortcut(action: Action) -> Option<Command> {
    COMMANDS
        .iter()
        .chain([&CLOSE])
        .find(|command| command.shortcut == action)
        .copied()
}

pub fn matching(query: &str, split: bool) -> Vec<Command> {
    let query = query.to_lowercase();
    COMMANDS
        .iter()
        .chain(split.then_some(&CLOSE))
        .copied()
        .filter(|command| command.matches(&query))
        .collect()
}

pub fn query(input: &str) -> Option<&str> {
    let query = input.strip_prefix(PREFIX)?;
    (!query.contains('\n')).then(|| query.trim())
}

pub fn exact(input: &str, split: bool) -> Option<Command> {
    let label = query(input)?;
    matching(label, split)
        .into_iter()
        .find(|command| command.label == label)
}

pub fn picker(commands: &[Command], keymaps: &Keymaps, split: bool) -> Picker {
    Picker::new(
        "commands",
        commands
            .iter()
            .copied()
            .map(|command| command.item(keymaps, split))
            .collect(),
    )
}

fn open_settings(editor: &mut Editor, _: OpenTarget) -> Reaction {
    editor.open_settings_from_commands();
    Reaction::Continue
}

fn style(editor: &mut Editor, _: OpenTarget) -> Reaction {
    editor.open_style_from_commands();
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
    editor.finish_command();
    editor.toggle_highlight_debug()
}

fn split(editor: &mut Editor, _: OpenTarget) -> Reaction {
    editor.finish_command();
    Reaction::New(OpenTarget::Split)
}

macro_rules! views {
    ($($name:ident: $view:ident),+ $(,)?) => {$(
        fn $name(editor: &mut Editor, target: OpenTarget) -> Reaction {
            editor.finish_command();
            Reaction::Open(ViewKind::$view, target)
        }
    )+};
}

macro_rules! finished {
    ($($name:ident: $action:ident),+ $(,)?) => {$(
        fn $name(editor: &mut Editor, _: OpenTarget) -> Reaction {
            editor.finish_command();
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
        assert_eq!(matching("style", false).len(), 1);
        assert_eq!(exact("/style", false).unwrap().label(), "style");
        assert!(exact("/new", false).unwrap().shortcut == Action::Restart);
        assert!(exact("/split", false).unwrap().shortcut == Action::NewWorkspace);
        assert!(exact("/sty", false).is_none());
    }

    #[test]
    fn new_restarts_while_split_always_opens_a_split() {
        let mut editor = Editor::new(String::new());

        assert!(matches!(
            exact("/new", false)
                .unwrap()
                .run(&mut editor, OpenTarget::Current),
            Reaction::Restart
        ));
        assert!(matches!(
            exact("/split", false)
                .unwrap()
                .run(&mut editor, OpenTarget::Current),
            Reaction::New(OpenTarget::Split)
        ));
    }

    #[test]
    fn every_command_has_a_resolvable_keymap() {
        let keymaps = Keymaps::default();

        for command in matching("", true) {
            let action = command.shortcut;
            assert!(keymaps.key_for(action).is_some(), "{}", command.label);
            assert_eq!(shortcut(action).unwrap().label, command.label);
            assert!(command
                .item(&keymaps, false)
                .spans(&LineContext {
                    width: 200,
                    value_column: 12,
                    columns: Some([24, 48]),
                    selected: false,
                    groups: Groups::default(),
                })
                .iter()
                .any(|span| span.content.contains(action.description())));
        }
        assert_eq!(exact("/ast", false).unwrap().label(), "ast");
    }

    #[test]
    fn command_metadata_stays_beside_its_label() {
        let item = matching("code", false)[0].item(&Keymaps::default(), false);
        let text = item
            .spans(&LineContext {
                width: 200,
                value_column: 12,
                columns: Some([24, 48]),
                selected: false,
                groups: Groups::default(),
            })
            .into_iter()
            .map(|span| span.content)
            .collect::<String>();
        let alt_c = (KeyCode::Char('c') + Alt).to_string();
        let alt_v = (KeyCode::Char('v') + Alt).to_string();

        assert_eq!(text.find(Action::OpenCode.description()), Some(27));
        assert_eq!(text.find(&alt_c), Some(51));
        assert!(text.contains(&format!("{alt_c} · {alt_v} alternative view")));

        let spans = item.spans(&LineContext {
            width: 200,
            value_column: 12,
            columns: Some([24, 48]),
            selected: false,
            groups: Groups::default(),
        });
        let literal = spans.iter().find(|span| span.content == "/code").unwrap();
        assert_eq!(literal.style.fg, Groups::default().prompt().fg);
    }

    #[test]
    fn alt_v_requests_an_alternative_view_only_for_splittable_unsplit_items() {
        let keymaps = Keymaps::default();
        let code = matching("code", false)[0];
        let mut item = code.item(&keymaps, false);
        assert!(matches!(
            item.handle(PickerInput::Key(KeyCode::Char('v') + Alt)),
            Some(EditorEvent::RunCommand(_, OpenTarget::Split))
        ));

        let mut item = code.item(&keymaps, true);
        assert!(item
            .handle(PickerInput::Key(KeyCode::Char('v') + Alt))
            .is_none());
        let settings = matching("settings", false)[0];
        let mut item = settings.item(&keymaps, false);
        assert!(item
            .handle(PickerInput::Key(KeyCode::Char('v') + Alt))
            .is_none());
        assert!(item
            .handle(PickerInput::Key(KeyCode::Enter + Ctrl))
            .is_none());
    }
}
