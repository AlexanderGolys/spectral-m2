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

use super::picker::{Picker, PickerInput, PickerItem};
use super::EditorEvent;
use crate::ui::action::Action;
use crate::ui::key::KeyEvent;
use crate::ui::settings::keymap::Keymaps;
use crate::ui::theme::groups::scheme;

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
            Span::styled(
                self.command.action.description(),
                groups.command_action_description(),
            )
            .into(),
        ];
        if let Some(shortcut) = self.shortcut {
            cells.push(Span::styled(shortcut.to_string(), groups.command_shortcut()).into());
        }
        cells
    }

    fn handle(&mut self, input: PickerInput) -> Option<EditorEvent> {
        matches!(input, PickerInput::Action(Action::Submit))
            .then_some(EditorEvent::RunCommand(self.command))
    }
}

/// Every command, in the order the picker lists them.
const COMMANDS: [Command; 14] = [
    Command::new("settings", Action::OpenSettings),
    Command::new("keymaps", Action::OpenKeymaps),
    Command::new("clear", Action::ClearFeed),
    Command::new("code", Action::OpenCode),
    Command::new("output", Action::OpenOutput),
    Command::new("notes", Action::OpenNotes),
    Command::new("ast", Action::OpenSyntaxTree),
    Command::taking(
        "save",
        Action::SaveSession,
        Argument::Optional(Operand::File),
    ),
    Command::taking("load", Action::LoadCode, Argument::Required(Operand::File)),
    Command::new("resume", Action::ResumeSession),
    Command::new("recent", Action::RecentSession),
    Command::new("sessions", Action::BrowseSessions),
    Command::taking("view", Action::ViewFile, Argument::Required(Operand::File)),
    Command::taking(
        "save-selection",
        Action::SaveSelection,
        Argument::Optional(Operand::File),
    ),
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
