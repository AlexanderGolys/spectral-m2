//! What the input completes, chosen by what the cursor is in the middle of: a
//! command's name, the file a command takes, a file named in a string, or the
//! name of something M2 knows. Names are the ones M2's own readline completes,
//! reported by M2 as it starts and as each cell binds more.

use std::collections::BTreeSet;
use std::ops::Range;

use m2_syn::{lex_str_recovering, LexErrorKind, Spanned, TokenTree, TriviaKind};

use super::commands::{self, Command, Operand};
use crate::code::file::{self, Entries};
use crate::input::Completion;

/// What opens a raw string literal, which a path typed inside one follows.
const RAW_STRING_OPENER: &str = "///";

/// Every name M2 has reported, those the session bound kept apart from those
/// it started with.
#[derive(Default)]
pub struct Names {
    started: BTreeSet<String>,
    bound: BTreeSet<String>,
}

impl Names {
    /// Forget every name for those a freshly started M2 knows.
    pub fn reset(&mut self, names: Vec<String>) {
        self.started = names.into_iter().collect();
        self.bound.clear();
    }

    /// Add names the session has bound since.
    pub fn bind(&mut self, names: Vec<String>) {
        self.bound.extend(names);
    }

    /// Whether M2 knows `name`, so there is nothing left to complete.
    fn knows(&self, name: &str) -> bool {
        self.started.contains(name) || self.bound.contains(name)
    }

    /// The names starting with `prefix`: the session's own first, as the likelier
    /// to be meant, then each group shortest first.
    fn starting_with(&self, prefix: &str) -> Vec<String> {
        let matching = |names: &BTreeSet<String>| {
            let mut found: Vec<String> = names
                .range(prefix.to_string()..)
                .take_while(|name| name.starts_with(prefix))
                .cloned()
                .collect();
            found.sort_by_key(String::len);
            found
        };
        let mut found = matching(&self.bound);
        found.extend(
            matching(&self.started)
                .into_iter()
                .filter(|name| !self.bound.contains(name)),
        );
        found
    }
}

/// What the source before the cursor ends in the middle of, as M2 reads it.
#[derive(Debug, PartialEq, Eq)]
enum Typing {
    /// An identifier, spanning this range.
    Name(Range<usize>),
    /// A string literal, whose text so far spans this range.
    Text(Range<usize>),
    /// Anything else, a comment included.
    Other,
}

/// What `source` ends in the middle of.
fn typing(source: &str) -> Typing {
    let end = source.len();
    let recovered = match lex_str_recovering(source) {
        Ok(recovered) => recovered,
        Err(error) => {
            let opener = match error.kind() {
                LexErrorKind::UnterminatedString => "\"".len(),
                LexErrorKind::UnterminatedRawString => RAW_STRING_OPENER.len(),
                _ => return Typing::Other,
            };
            return error.span().range().map_or(Typing::Other, |range| {
                Typing::Text(range.start + opener..end)
            });
        }
    };
    // The innermost last token: an unclosed group's last token is inside it.
    let mut chain: Vec<&TokenTree> = Vec::new();
    let mut stream = &recovered.value;
    while let Some(last) = stream.iter().next_back() {
        chain.push(last);
        match last {
            TokenTree::Group(group) => stream = group.stream(),
            _ => break,
        }
    }
    let commented = chain.iter().any(|tree| {
        tree.trailing_trivia().as_slice().iter().any(|trivia| {
            trivia.kind() == TriviaKind::LineComment
                && trivia.span().range().is_some_and(|range| range.end == end)
        })
    });
    match chain.last() {
        Some(TokenTree::Ident(ident)) if !commented => ident
            .span()
            .range()
            .filter(|range| range.end == end)
            .map_or(Typing::Other, Typing::Name),
        _ => Typing::Other,
    }
}

/// Whether `character` can continue an M2 identifier.
fn continues_name(character: char) -> bool {
    character.is_alphanumeric() || character == '\''
}

/// What the input needs to know to complete its text.
pub struct Request<'a> {
    pub text: &'a str,
    pub cursor: usize,
    /// Incomplete source the input continues.
    pub pending: Option<&'a str>,
    /// The command picked while its name is being typed.
    pub picked: Option<Command>,
    pub names: &'a Names,
    /// Characters of a name or a string typed before it is completed.
    pub complete_after: usize,
}

/// The completion for the text before the cursor, if anything completes it.
pub fn complete(request: Request<'_>) -> Option<Completion> {
    let Request {
        text,
        cursor,
        pending,
        picked,
        names,
        complete_after,
    } = request;
    if let Some(command) = picked.filter(|_| cursor == text.len()) {
        return Completion::new(text, 0..text.len(), vec![commands::completed(command)])
            .map(Completion::hinted);
    }
    if text.starts_with(commands::PREFIX) {
        let (Operand::File, range) = commands::operand(text).filter(|_| cursor == text.len())?;
        let candidates = file::completions(&text[range.clone()], Entries::Code);
        return Completion::new(text, range, candidates).map(Completion::hinted);
    }
    let pending = pending.map_or(String::new(), |pending| format!("{pending}\n"));
    let in_text = |range: Range<usize>| range.start.max(pending.len()) - pending.len()..cursor;
    match typing(&format!("{pending}{}", &text[..cursor])) {
        Typing::Name(range) if !text[cursor..].starts_with(continues_name) => {
            let range = in_text(range);
            let typed = &text[range.clone()];
            (typed.chars().count() >= complete_after && !names.knows(typed))
                .then(|| Completion::new(text, range, names.starting_with(typed)))?
        }
        Typing::Text(range) => {
            let range = in_text(range);
            let typed = &text[range.clone()];
            (typed.chars().count() >= complete_after && !typed.contains(char::is_whitespace))
                .then(|| Completion::new(text, range, file::completions(typed, Entries::All)))?
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::action::Action;
    use crate::ui::controller::{Editor, EditorUpdate, Reaction};
    use crossterm::event::KeyCode;

    #[test]
    fn the_cursor_is_read_as_m2_reads_the_source_before_it() {
        assert_eq!(typing("x = ide"), Typing::Name(4..7));
        assert_eq!(typing("f(a, ide"), Typing::Name(5..8));
        assert_eq!(typing("load \"rin"), Typing::Text(6..9));
        assert_eq!(typing("get ///rin"), Typing::Text(7..10));
        assert_eq!(typing("x -- ide"), Typing::Other);
        assert_eq!(typing("f(x -- ide"), Typing::Other);
        assert_eq!(typing("-* ide"), Typing::Other);
        assert_eq!(typing("x + "), Typing::Other);
    }

    #[test]
    fn the_sessions_own_names_come_first_then_shorter_ones() {
        let mut names = Names::default();
        names.reset(["identity", "ideal", "image"].map(String::from).to_vec());
        names.bind(vec!["idealOfCurve".to_string()]);

        assert_eq!(
            names.starting_with("ide"),
            ["idealOfCurve", "ideal", "identity"]
        );
        names.reset(vec!["ideal".to_string()]);
        assert_eq!(names.starting_with("ide"), ["ideal"]);
    }

    fn request<'a>(text: &'a str, names: &'a Names) -> Request<'a> {
        Request {
            text,
            cursor: text.len(),
            pending: None,
            picked: None,
            names,
            complete_after: 1,
        }
    }

    #[test]
    fn a_name_completes_only_at_its_end_and_never_in_a_comment() {
        let mut names = Names::default();
        names.reset(vec!["ideal".to_string()]);

        let completion = complete(request("I = ide", &names)).expect("ideal completes ide");
        assert_eq!(completion.ghost(), "al");
        assert!(complete(Request {
            cursor: 2,
            ..request("idx", &names)
        })
        .is_none());
        assert!(complete(request("-- ide", &names)).is_none());
        assert!(complete(Request {
            complete_after: 4,
            ..request("ide", &names)
        })
        .is_none());
    }

    #[test]
    fn a_name_continues_incomplete_source() {
        let mut names = Names::default();
        names.reset(vec!["ideal".to_string()]);
        let completion = complete(Request {
            pending: Some("f = x -> ("),
            ..request("ide", &names)
        })
        .expect("the live line continues the pending source");

        assert_eq!(completion.ghost(), "al");
    }

    /// An editor that knows `names`, with the first `complete_after` characters
    /// of `typed` typed, so a name completes whatever the setting is.
    fn typing_editor(names: &[&str], typed: &str) -> Editor {
        let mut editor = Editor::new(String::new());
        editor.apply(EditorUpdate::KnownNames(
            names.iter().map(|name| name.to_string()).collect(),
        ));
        let typed: String = typed
            .chars()
            .take(usize::from(editor.input.settings().complete_after))
            .collect();
        editor.paste(&format!("I = {typed}"));
        editor
    }

    #[test]
    fn tab_takes_the_name_shown_and_cycles_the_rest_instead_of_completing_further() {
        let names = ["identityMap", "identity", "identityMapOf"];
        let mut editor = typing_editor(&names, "identityMap");
        for expected in ["identity", "identityMap", "identityMapOf", "identity"] {
            editor.handle_input(KeyCode::Tab.into());
            assert_eq!(editor.submission(), format!("I = {expected}"));
        }
        editor.handle_input(KeyCode::BackTab.into());
        assert_eq!(editor.submission(), "I = identityMapOf");
    }

    #[test]
    fn a_name_m2_knows_is_not_completed_and_one_letter_names_are() {
        let mut names = Names::default();
        names.reset(vec!["ideal".to_string()]);
        let one_letter = Request {
            complete_after: 1,
            ..request("I = i", &names)
        };
        assert_eq!(complete(one_letter).unwrap().ghost(), "deal");
        assert!(complete(request("I = ideal", &names)).is_none());
    }

    #[test]
    fn tab_does_nothing_when_nothing_completes() {
        let mut editor = typing_editor(&[], "identity");
        let typed = editor.submission().to_string();
        editor.handle_input(KeyCode::Tab.into());

        assert_eq!(editor.submission(), typed);
    }

    /// An editor with `typed` typed key by key, as the command picker sees it.
    fn typed_editor(typed: &str) -> Editor {
        let mut editor = Editor::new(String::new());
        for character in typed.chars() {
            editor.handle_input(KeyCode::Char(character).into());
        }
        editor
    }

    #[test]
    fn tab_completes_the_picked_command_and_the_space_before_its_file() {
        let mut editor = typed_editor(&format!("{}sa", commands::PREFIX));
        editor.handle_input(KeyCode::Tab.into());

        assert_eq!(editor.submission(), format!("{}save ", commands::PREFIX));
        assert!(
            !editor.picker_open(),
            "the file follows, not another command"
        );
    }

    #[test]
    fn tab_completes_a_whole_command_and_puts_the_picker_away() {
        let mut editor = typed_editor(&format!("{}se", commands::PREFIX));
        assert!(editor.picker_open());
        editor.handle_input(KeyCode::Tab.into());

        assert_eq!(editor.submission(), format!("{}settings", commands::PREFIX));
        assert!(!editor.picker_open());
    }

    #[test]
    fn the_cursor_moves_as_in_any_editor_and_a_command_completes_only_at_the_end() {
        let mut editor = typed_editor(&format!("{}se", commands::PREFIX));
        let completes = |editor: &mut Editor| {
            editor.refresh_completion();
            editor.input.offered().is_some()
        };
        assert!(completes(&mut editor));

        editor.handle_input(KeyCode::Left.into());
        assert!(
            editor.picker_open(),
            "the list stays while the name is edited"
        );
        assert!(!completes(&mut editor));
        assert_eq!(editor.input.field().cursor(), editor.submission().len() - 1);

        editor.handle_input(KeyCode::Right.into());
        assert!(completes(&mut editor));
    }

    #[test]
    fn escape_puts_the_picker_away_and_keeps_the_command_typed() {
        let mut editor = typed_editor(&format!("{}sa", commands::PREFIX));
        assert!(editor.picker_open());
        editor.handle_input(KeyCode::Esc.into());

        assert!(!editor.picker_open());
        assert_eq!(editor.submission(), format!("{}sa", commands::PREFIX));
    }

    #[test]
    fn a_command_is_completed_as_a_hint_and_a_name_as_text() {
        let mut names = Names::default();
        names.reset(vec!["ideal".to_string()]);
        assert!(!complete(request("ide", &names)).unwrap().is_hint());

        let (save, _) = commands::exact(&format!("{}save", commands::PREFIX)).unwrap();
        let typed = format!("{}sa", commands::PREFIX);
        let picked = Request {
            picked: Some(save),
            ..request(&typed, &names)
        };
        assert!(complete(picked).unwrap().is_hint());
    }

    #[test]
    fn a_command_picked_by_part_of_its_name_is_remembered_whole() {
        let mut editor = typed_editor(&format!("{}res", commands::PREFIX));
        assert!(matches!(
            editor.handle_action(Action::Submit),
            Reaction::Resume
        ));
        editor.handle_input(KeyCode::Up.into());

        assert_eq!(editor.submission(), format!("{}resume", commands::PREFIX));
    }

    #[test]
    fn a_picked_command_completes_to_itself_and_its_arguments_space() {
        let names = Names::default();
        let (save, _) = commands::exact(&format!("{}save", commands::PREFIX)).unwrap();
        let typed = format!("{}sa", commands::PREFIX);
        let completion = complete(Request {
            picked: Some(save),
            ..request(&typed, &names)
        })
        .unwrap();

        assert_eq!(completion.chosen(), format!("{}save ", commands::PREFIX));
    }
}
