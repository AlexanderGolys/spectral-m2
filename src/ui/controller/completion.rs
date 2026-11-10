//! What the input completes, chosen by what the cursor is in the middle of: a
//! command's name, the file a command takes, a file named in a string, or the
//! name of something M2 knows. Names are the ones M2's own readline completes,
//! reported by M2 as it starts and as each cell binds more.

mod usage;

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use m2_syn::{lex_str_recovering, LexError, LexErrorKind, Spanned, TokenTree, TriviaKind};

use self::usage::Usage;
use super::commands::{self, Command, Operand};
use crate::code::file::{self, Entries};
use crate::input::Completion;

/// What opens a raw string literal, which a path typed inside one follows.
const RAW_STRING_OPENER: &str = "///";

/// Every name M2 has reported, those the session bound kept apart from those
/// it started with.
pub struct Names {
    started: BTreeSet<String>,
    bound: BTreeSet<String>,
    /// Names the language server reads as bound in the session's source,
    /// the input included, before M2 has run them.
    analysed: BTreeSet<String>,
    locals: BTreeSet<String>,
    usage: Usage,
    /// Frequencies for names introduced by this session, never saved to disk.
    session_usage: BTreeMap<String, u64>,
}

impl Default for Names {
    fn default() -> Self {
        Self {
            started: BTreeSet::new(),
            bound: BTreeSet::new(),
            analysed: BTreeSet::new(),
            locals: BTreeSet::new(),
            usage: Usage::load(),
            session_usage: BTreeMap::new(),
        }
    }
}

impl Names {
    /// Forget every name for those a freshly started M2 knows.
    pub fn reset(&mut self, names: Vec<String>) {
        self.started = names.into_iter().collect();
        self.bound.clear();
        self.locals.clear();
        self.session_usage.clear();
    }

    /// Add names the session has bound since.
    pub fn bind(&mut self, names: Vec<String>) {
        self.bound.extend(names);
    }

    /// Replace the names the language server last read as bound.
    pub fn analyse(&mut self, names: impl IntoIterator<Item = String>) {
        self.analysed = names.into_iter().collect();
    }

    /// Save library-symbol frequencies and retain session-name frequencies only in memory.
    pub fn record(&mut self, source: &str) {
        let used = usage::references(source, &mut self.locals);
        let mut library = Vec::new();
        for name in used {
            if self.started.contains(&name) {
                library.push(name);
            } else if self.bound.contains(&name) {
                let count = self.session_usage.entry(name).or_default();
                *count = count.saturating_add(1);
            }
        }
        self.usage.record(library);
    }

    /// The saved library count or the current session's count for a candidate.
    fn frequency(&self, name: &str) -> u64 {
        if self.started.contains(name) {
            self.usage.count(name)
        } else {
            self.session_usage.get(name).copied().unwrap_or_default()
        }
    }

    /// Whether M2 knows `name`, so there is nothing left to complete.
    fn knows(&self, name: &str) -> bool {
        self.started.contains(name) || self.bound.contains(name)
    }

    /// Prefix matches ranked by usage, then by length and name for stable ties.
    fn starting_with(&self, prefix: &str) -> Vec<String> {
        let matching = |names: &BTreeSet<String>| {
            names
                .range(prefix.to_string()..)
                .take_while(|name| name.starts_with(prefix))
                .cloned()
                .collect::<Vec<String>>()
        };
        let mut found = matching(&self.bound);
        found.extend(
            matching(&self.analysed)
                .into_iter()
                .filter(|name| name != prefix && !self.knows(name)),
        );
        found.extend(
            matching(&self.started)
                .into_iter()
                .filter(|name| !self.bound.contains(name)),
        );
        found.sort_by(|left, right| {
            (Reverse(self.frequency(left)), left.len(), left).cmp(&(
                Reverse(self.frequency(right)),
                right.len(),
                right,
            ))
        });
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
        Err(error) => return unterminated_text(&error, end).unwrap_or(Typing::Other),
    };
    // A string left open is recovered, and reported among the diagnostics.
    if let Some(text) = recovered
        .diagnostics
        .iter()
        .find_map(|error| unterminated_text(error, end))
    {
        return text;
    }
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

/// The text typed into a string `error` reports left open, running to `end`.
fn unterminated_text(error: &LexError, end: usize) -> Option<Typing> {
    let opener = match error.kind() {
        LexErrorKind::UnterminatedString => "\"".len(),
        LexErrorKind::UnterminatedRawString => RAW_STRING_OPENER.len(),
        _ => return None,
    };
    let range = error.span().range()?;
    Some(Typing::Text(range.start + opener..end))
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
        let mut candidates = vec![commands::completed(command)];
        candidates.extend(
            commands::matching(text.strip_prefix(commands::PREFIX)?)
                .into_iter()
                .filter(|candidate| candidate.action != command.action)
                .map(commands::completed),
        );
        return Completion::new(text, 0..text.len(), candidates);
    }
    if text.starts_with(commands::PREFIX) {
        let (Operand::File, range) = commands::operand(text).filter(|_| cursor == text.len())?;
        let candidates = file::completions(&text[range.clone()], Entries::Code);
        return Completion::new(text, range, candidates);
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
    fn names_the_server_reads_as_bound_complete_before_m2_has_run_them() {
        let mut names = Names::default();
        names.analyse(["myFunction".to_string(), "my".to_string()]);
        assert_eq!(names.starting_with("my"), vec!["myFunction".to_string()]);
        names.analyse(Vec::new());
        assert!(names.starting_with("my").is_empty());
    }

    #[test]
    fn unused_names_are_ordered_by_length_then_name() {
        let mut names = Names::default();
        names.reset(["identity", "ideal", "image"].map(String::from).to_vec());
        names.bind(vec!["idealOfCurve".to_string()]);

        assert_eq!(
            names.starting_with("ide"),
            ["ideal", "identity", "idealOfCurve"]
        );
        names.reset(vec!["ideal".to_string()]);
        assert_eq!(names.starting_with("ide"), ["ideal"]);
    }

    #[test]
    fn global_usage_outweighs_name_length_but_local_usage_does_not() {
        let mut names = Names::default();
        names.reset(["identity", "ideal", "image"].map(String::from).to_vec());
        names.record("identity; ideal; identity; unknown");
        names.record("f = ideal -> (local image; ideal + image)");
        assert_eq!(names.starting_with("i"), ["identity", "ideal", "image"]);
        assert_eq!(names.usage.count("identity"), 2);
        assert_eq!(names.usage.count("ideal"), 1);
        assert_eq!(names.usage.count("image"), 0);
        assert_eq!(names.usage.count("unknown"), 0);

        names.record("ideal := 1");
        names.record("ideal; ideal");
        assert_eq!(names.usage.count("ideal"), 1);
        names.reset(["ideal"].map(String::from).to_vec());
        names.record("ideal");
        assert_eq!(names.usage.count("ideal"), 2);
    }

    #[test]
    fn session_names_gain_rank_without_persisting_their_counts() {
        let mut names = Names::default();
        names.reset(vec!["rank".into()]);
        names.bind(vec!["R".into(), "RingOfCurve".into()]);
        names.record("R = QQ[x]; RingOfCurve = R");
        for _ in 0..100 {
            names.record("RingOfCurve; rank RingOfCurve");
        }
        assert_eq!(names.starting_with("R"), ["RingOfCurve", "R"]);
        assert_eq!(names.frequency("RingOfCurve"), 200);
        assert_eq!(names.usage.count("R"), 0);
        assert_eq!(names.usage.count("RingOfCurve"), 0);
        assert_eq!(names.usage.count("rank"), 100);

        names.reset(vec!["rank".into()]);
        names.bind(vec!["R".into(), "RingOfCurve".into()]);
        assert_eq!(names.frequency("RingOfCurve"), 0);
        assert_eq!(names.starting_with("R"), ["R", "RingOfCurve"]);
        assert_eq!(names.frequency("rank"), 100);
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
    fn tab_reveals_a_list_before_inserting_when_the_threshold_is_reached() {
        let mut editor = typing_editor(&["ideal", "identity", "identityMap"], "identity");
        let mut settings = editor.input.settings();
        settings.hint_completion_below = 3;
        editor.input.set_settings(settings);
        let typed = editor.submission().to_string();

        editor.handle_input(KeyCode::Tab.into());
        assert_eq!(editor.submission(), typed);
        let completion = editor.input.offered().unwrap();
        assert!(!completion.hinted(settings.hint_completion_below));
        assert_eq!(completion.list_height(), 5);

        editor.handle_input(KeyCode::Tab.into());
        assert_eq!(editor.submission(), "I = ideal");
        editor.handle_input(KeyCode::Tab.into());
        assert_eq!(editor.submission(), "I = identity");
        editor.handle_input(
            crate::ui::key::KeyEvent::from(KeyCode::Tab) + crate::ui::key::KeyMod::Ctrl,
        );
        assert_eq!(editor.submission(), "I = ideal");

        editor.handle_input(KeyCode::Char('x').into());
        editor.refresh_completion();
        assert!(editor.input.offered().is_none());
    }

    #[test]
    fn zero_disables_hints_and_escape_dismisses_the_list_without_editing() {
        let mut editor = typing_editor(&["ideal"], "ideal");
        let mut settings = editor.input.settings();
        settings.hint_completion_below = 0;
        editor.input.set_settings(settings);
        let typed = editor.submission().to_string();
        editor.handle_input(KeyCode::Tab.into());
        assert_eq!(editor.submission(), typed);
        assert_eq!(editor.input.offered().unwrap().list_height(), 3);
        editor.handle_input(KeyCode::Esc.into());
        assert_eq!(editor.submission(), typed);
        assert!(editor.input.offered().is_none());
    }

    #[test]
    fn evaluated_usage_changes_the_first_completion_and_the_list_is_drawn_above_input() {
        use crate::code::highlight::SyntaxHighlighter;
        use crate::ui::cell::CellState;
        use ratatui::{backend::TestBackend, Terminal};

        let mut editor = Editor::new(String::new());
        editor.apply(EditorUpdate::KnownNames(vec![
            "ideal".into(),
            "identity".into(),
        ]));
        editor.apply(EditorUpdate::Pending {
            id: 1,
            cells: vec![("identity; identity".into(), Vec::new())],
        });
        assert_eq!(
            editor.names.usage.count("identity"),
            0,
            "queued text has not been used"
        );
        editor.apply(EditorUpdate::CellSucceeded {
            id: 1,
            index: 0,
            number: 1,
            state: CellState::Null,
            outputs: Vec::new(),
            printed: None,
            value: None,
        });
        assert_eq!(editor.names.usage.count("identity"), 2);
        let mut settings = editor.input.settings();
        settings.hint_completion_below = 2;
        editor.input.set_settings(settings);
        editor.paste("I = id");
        editor.handle_input(KeyCode::Tab.into());
        assert_eq!(editor.submission(), "I = id");

        for (width, height) in [(40, 12), (16, 7)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    editor.draw(frame, frame.area(), &mut SyntaxHighlighter::new(), false);
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            let rows: Vec<String> = (0..height)
                .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
                .collect();
            let candidate = rows
                .iter()
                .position(|row| row.contains("> identity"))
                .expect("ranked candidates are drawn");
            let input = rows
                .iter()
                .position(|row| row.contains("I = id"))
                .expect("input remains visible");
            assert!(candidate < input);
            let cursor = terminal.get_cursor_position().unwrap();
            assert!(cursor.x < width && cursor.y < height);
        }
        editor.handle_input(KeyCode::Tab.into());
        assert_eq!(editor.submission(), "I = identity");
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
