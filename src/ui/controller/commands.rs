//! Commands: names typed after [`PREFIX`] or picked from the command picker.
//!
//! A command owns no behaviour and knows nothing of keys. It names an
//! [`Action`] and hands it to the editor, the same entry point a keymap uses, so
//! typing a command and pressing a key are two independent ways to trigger the
//! same action. An action may have either, both, or neither. Only the picker
//! brings them together, showing beside each command the key, if any, that
//! triggers the same action.

use std::ops::Range;

use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use these_macros_should_be_illegal::discriminated_str;

use super::picker::{Picker, PickerInput, PickerItem};
use super::settings::keymap::Keymaps;
use super::settings::theme::groups::scheme;
use super::EditorEvent;
use crate::ui::action::Action;
use crate::ui::key::KeyEvent;

/// Prefix that turns input text into a live command query.
pub const PREFIX: &str = "/";

/// What a command's argument is, named as its usage shows it.
#[discriminated_str(name)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Operand {
    File = "file",
}

/// What a command reads after its name.
#[derive(Clone, Copy)]
pub enum Argument {
    /// The command runs with or without it.
    Optional(Operand),
    /// The command needs it to run.
    Required(Operand),
}

impl Argument {
    pub const fn operand(self) -> Operand {
        match self {
            Self::Optional(operand) | Self::Required(operand) => operand,
        }
    }
}

/// One command-picker entry: a name typed to trigger an action, followed by the
/// argument that action takes, if it takes one.
#[derive(Clone, Copy)]
pub struct Command {
    label: &'static str,
    pub action: Action,
    pub argument: Option<Argument>,
}

impl Command {
    const fn new(label: &'static str, action: Action) -> Self {
        Self {
            label,
            action,
            argument: None,
        }
    }

    const fn taking(label: &'static str, action: Action, argument: Argument) -> Self {
        Self {
            label,
            action,
            argument: Some(argument),
        }
    }

    /// Whether `query` starts the label or any of its words, so a multi-word
    /// label is found from any of them.
    fn matches(self, query: &str) -> bool {
        self.label.starts_with(query)
            || self
                .label
                .split_whitespace()
                .any(|word| word.starts_with(query))
    }

    #[cfg(test)]
    pub const fn label(self) -> &'static str {
        self.label
    }

    /// The part of the picker's line for this command that `query` has typed, the
    /// prefix included when it starts the label and only the word otherwise.
    fn typed_range(self, query: &str) -> Range<usize> {
        if query.is_empty() {
            return 0..0;
        }
        let prefix = PREFIX.len();
        if self.label.starts_with(query) {
            return 0..prefix + query.len();
        }
        let word = self
            .label
            .split_whitespace()
            .find(|word| word.starts_with(query))
            .and_then(|word| self.label.find(word));
        word.map_or(0..0, |start| prefix + start..prefix + start + query.len())
    }

    /// The command as typed into the input, prefix included.
    pub fn literal(self) -> String {
        format!("{PREFIX}{}", self.label)
    }

    /// The command as the picker shows it: its typed form and its argument.
    fn usage(self) -> String {
        match self.argument {
            Some(Argument::Optional(operand)) => format!("{} [{}]", self.literal(), operand.name()),
            Some(Argument::Required(operand)) => format!("{} <{}>", self.literal(), operand.name()),
            None => self.literal(),
        }
    }
}

/// A command's picker line: its typed form, what its action does, and the key
/// that triggers the same action, when one does. Enter runs it.
struct CommandItem {
    command: Command,
    shortcut: Option<KeyEvent>,
    /// The range of the usage the query typed, drawn bold.
    typed: Range<usize>,
}

impl PickerItem for CommandItem {
    fn cells(&self) -> Vec<Line<'static>> {
        let groups = scheme();
        let name = groups.command();
        let usage = self.command.usage();
        let (before, rest) = usage.split_at(self.typed.start);
        let (typed, after) = rest.split_at(self.typed.len());
        let usage = Line::from(vec![
            Span::styled(before.to_string(), name),
            Span::styled(typed.to_string(), name.add_modifier(Modifier::BOLD)),
            Span::styled(after.to_string(), name),
        ]);
        let mut cells = vec![
            usage,
            Span::styled(self.command.action.description(), groups.value()).into(),
        ];
        if let Some(shortcut) = self.shortcut {
            cells.push(Span::styled(shortcut.to_string(), groups.prompt()).into());
        }
        cells
    }

    fn handle(&mut self, input: PickerInput) -> Option<EditorEvent> {
        matches!(input, PickerInput::Action(Action::Submit))
            .then_some(EditorEvent::RunCommand(self.command))
    }
}

/// Every command, in the order the picker lists them.
const COMMANDS: [Command; 11] = [
    Command::new("settings", Action::OpenSettings),
    Command::new("keymaps", Action::OpenKeymaps),
    Command::new("clear", Action::ClearFeed),
    Command::new("code", Action::OpenCode),
    Command::new("values", Action::OpenValues),
    Command::new("stdout", Action::OpenStdOut),
    Command::new("output", Action::OpenOutput),
    Command::new("ast", Action::OpenSyntaxTree),
    Command::taking(
        "save",
        Action::SaveSession,
        Argument::Optional(Operand::File),
    ),
    Command::taking("load", Action::LoadCode, Argument::Required(Operand::File)),
    Command::new("resume", Action::ResumeSession),
];

/// The commands whose label or one of its words starts with `query`, ignoring
/// case; an empty query matches them all.
pub fn matching(query: &str) -> Vec<Command> {
    let query = query.to_lowercase();
    COMMANDS
        .iter()
        .copied()
        .filter(|command| command.matches(&query))
        .collect()
}

/// The command `input` names in full, prefix included, as a submitted command
/// does, with the argument typed after it; a prefix of a label is not enough,
/// and only a command that takes an argument may be followed by one.
pub fn exact(input: &str) -> Option<(Command, &str)> {
    let typed = input.strip_prefix(PREFIX)?;
    let (label, argument) = typed
        .split_once(char::is_whitespace)
        .map_or((typed, ""), |(label, argument)| (label, argument.trim()));
    COMMANDS
        .into_iter()
        .find(|command| {
            command.label == label && (command.argument.is_some() || argument.is_empty())
        })
        .map(|command| (command, argument))
}

/// The operand of the whole command `input` starts with, and the range of its
/// argument typed so far; a command is followed by its argument only once a
/// space separates them.
pub fn operand(input: &str) -> Option<(Operand, Range<usize>)> {
    let (label, rest) = input
        .strip_prefix(PREFIX)?
        .split_once(char::is_whitespace)?;
    let argument = COMMANDS
        .iter()
        .find(|command| command.label == label)?
        .argument?;
    Some((
        argument.operand(),
        input.len() - rest.trim_start().len()..input.len(),
    ))
}

/// The input `command` completes to: its typed form, then the space its
/// argument follows when it takes one.
pub fn completed(command: Command) -> String {
    match command.argument {
        Some(_) => format!("{} ", command.literal()),
        None => command.literal(),
    }
}

/// Whether `input` is a command being typed: the start of a command's name, or
/// a whole command with the argument typed after it.
pub fn typed(input: &str) -> bool {
    input
        .strip_prefix(PREFIX)
        .is_some_and(|query| !matching(query).is_empty())
        || exact(input).is_some()
}

/// The range of `input` that is a command's name, prefix included, when `input`
/// is a command being typed.
pub fn name_range(input: &str) -> Option<Range<usize>> {
    let end = input.find(char::is_whitespace).unwrap_or(input.len());
    typed(input).then_some(0..end)
}

/// A picker listing `commands`, each beside the key `keymaps` binds to its action,
/// with what `query` has typed of each in bold.
pub fn picker(commands: &[Command], keymaps: &Keymaps, query: &str) -> Picker {
    Picker::new(
        "commands",
        commands
            .iter()
            .map(|&command| {
                Box::new(CommandItem {
                    command,
                    shortcut: keymaps.key(command.action),
                    typed: command.typed_range(query),
                }) as Box<dyn PickerItem>
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::controller::settings::theme::groups::Groups;
    use crate::ui::controller::{Editor, Reaction, ViewKind};
    use crate::ui::key::KeyMod::Ctrl;
    use crossterm::event::KeyCode;

    /// The picker line for the command named `label`, with the default keys.
    fn line(label: &str) -> CommandItem {
        let (command, _) = exact(&format!("{PREFIX}{label}")).expect("the command exists");
        CommandItem {
            command,
            shortcut: Keymaps::default().key(command.action),
            typed: 0..0,
        }
    }

    #[test]
    fn picker_entries_filter_by_word_prefix_in_the_visible_name() {
        assert_eq!(matching("settings").len(), 1);
        assert_eq!(matching("co").len(), 1);
        assert_eq!(matching("ast").len(), 1);
        assert_eq!(matching("ode").len(), 0);
        assert!(matching("does-not-exist").is_empty());
        assert!(matching("close").is_empty());
        assert!(exact(&format!("{PREFIX}clear")).unwrap().0.action == Action::ClearFeed);
        assert!(exact(&format!("{PREFIX}sett")).is_none());
    }

    #[test]
    fn only_a_command_taking_an_argument_may_be_followed_by_one() {
        let typed = format!("{PREFIX}save  notes.m2 ");
        let (save, file) = exact(&typed).expect("save takes a file");
        assert!(save.action == Action::SaveSession);
        assert_eq!(file, "notes.m2");
        assert_eq!(exact(&format!("{PREFIX}load")).unwrap().1, "");

        assert!(exact(&format!("{PREFIX}code notes.m2")).is_none());
    }

    #[test]
    fn a_whole_command_with_its_argument_reads_as_a_command() {
        assert!(typed(&format!("{PREFIX}sa")));
        assert!(typed(&format!("{PREFIX}save notes.m2")));
        assert!(!typed(&format!("{PREFIX}code notes.m2")));
        assert!(!typed("save notes.m2"));
    }

    #[test]
    fn save_without_a_file_keeps_the_session_where_exit_does() {
        let mut editor = Editor::new(String::new());
        editor.paste(&format!("{PREFIX}save"));
        assert!(matches!(
            editor.handle_action(Action::Submit),
            Reaction::Save(None)
        ));

        editor.paste(&format!("{PREFIX}save notes"));
        assert!(matches!(
            editor.handle_action(Action::Submit),
            Reaction::Save(Some(file)) if file == "notes"
        ));
    }

    #[test]
    fn a_file_command_without_a_file_asks_for_one_instead_of_running() {
        let mut editor = Editor::new(String::new());
        editor.paste(&format!("{PREFIX}load"));
        assert!(matches!(
            editor.handle_action(Action::Submit),
            Reaction::Continue
        ));

        editor.paste(&format!("{PREFIX}load ring.m2"));
        assert!(matches!(
            editor.handle_action(Action::Submit),
            Reaction::Load(file) if file == "ring.m2"
        ));
    }

    #[test]
    fn neither_a_second_session_nor_a_separate_style_picker_is_a_command() {
        assert!(exact(&format!("{PREFIX}split")).is_none());
        assert!(exact(&format!("{PREFIX}style")).is_none());
    }

    #[test]
    fn process_actions_are_keymaps_not_commands() {
        for action in [Action::Restart, Action::Quit, Action::ToggleHighlightDebug] {
            assert!(matching("").iter().all(|command| command.action != action));
        }
        let mut editor = Editor::new(String::new());

        assert!(matches!(
            editor.handle_action(Action::Restart),
            Reaction::Restart
        ));
        assert!(matches!(editor.handle_action(Action::Quit), Reaction::Quit));
        assert!(matches!(
            editor.handle_action(Action::ToggleHighlightDebug),
            Reaction::Continue
        ));
    }

    #[test]
    fn every_command_describes_its_action() {
        for command in matching("") {
            let item = CommandItem {
                command,
                shortcut: None,
                typed: 0..0,
            };
            assert_eq!(item.cells()[1].to_string(), command.action.description());
        }
        assert_eq!(exact(&format!("{PREFIX}ast")).unwrap().0.label(), "ast");
    }

    #[test]
    fn a_typed_command_triggers_the_same_action_as_its_key() {
        let mut editor = Editor::new(String::new());
        assert!(matches!(
            editor.handle_action(Action::OpenCode),
            Reaction::Open(ViewKind::Code)
        ));

        editor.paste(&format!("{PREFIX}code"));
        assert!(matches!(
            editor.handle_action(Action::Submit),
            Reaction::Open(ViewKind::Code)
        ));
    }

    #[test]
    fn a_command_line_shows_the_key_of_its_action_when_it_has_one() {
        let cells = line("code").cells();
        let key = Keymaps::default()
            .key(Action::OpenCode)
            .expect("opening code is bound");

        assert_eq!(cells[0].to_string(), format!("{PREFIX}code"));
        assert_eq!(cells[0].spans[0].style.fg, Groups::default().prompt().fg);
        assert_eq!(cells[1].to_string(), Action::OpenCode.description());
        assert_eq!(cells[2].to_string(), key.to_string());

        let unbound = CommandItem {
            shortcut: None,
            ..line("code")
        };
        assert_eq!(unbound.cells().len(), 2);
    }

    #[test]
    fn a_command_runs_only_when_submitted() {
        let mut item = line("code");
        assert!(matches!(
            item.handle(PickerInput::Action(Action::Submit)),
            Some(EditorEvent::RunCommand(_))
        ));

        let mut item = line("settings");
        assert!(item
            .handle(PickerInput::Key(KeyCode::Enter + Ctrl))
            .is_none());
    }

    #[test]
    fn the_typed_part_of_each_command_is_what_the_picker_draws_bold() {
        let settings = line("settings").command;
        assert_eq!(settings.typed_range(""), 0..0);
        assert_eq!(settings.typed_range("se"), 0..PREFIX.len() + 2);

        let item = CommandItem {
            typed: settings.typed_range("se"),
            ..line("settings")
        };
        let cells = item.cells();
        let bold = |span: &Span| span.style.add_modifier.contains(Modifier::BOLD);
        let (typed, rest): (Vec<_>, Vec<_>) = cells[0]
            .spans
            .iter()
            .filter(|span| !span.content.is_empty())
            .partition(|span| bold(span));
        assert_eq!(typed.len(), 1);
        assert_eq!(typed[0].content, format!("{PREFIX}se"));
        assert_eq!(rest[0].content, "ttings");
    }

    #[test]
    fn the_name_typed_is_coloured_whatever_follows_it() {
        assert_eq!(
            name_range(&format!("{PREFIX}sa")),
            Some(0..PREFIX.len() + 2)
        );
        assert_eq!(
            name_range(&format!("{PREFIX}save notes")),
            Some(0..PREFIX.len() + 4)
        );
        assert_eq!(name_range("x = 1"), None);
    }
}
