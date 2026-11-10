use url::Url;
#[test]
fn preview_tabs_retain_arbitrary_views_and_source_actions() {
    use super::previews::PreviewTab;
    use crate::ui::cell::TextBlock;
    use crate::ui::controller::ViewKind;
    let mut app = super::App::new(crate::backend::M2Backend::spawn().unwrap());
    app.open_preview(ViewKind::Code, "code".into());
    let code_tab = app.previews.active.unwrap();
    app.panes.list[super::Panes::SPLIT].viewport.scroll = 7;
    app.open_preview(ViewKind::Reference, "source".into());
    app.workspace.editor.open_reference(
        vec![Box::new(TextBlock::system("source text"))],
        Some("source.m2".into()),
    );
    let source_tab = app.previews.active.unwrap();
    app.previews.pin();
    app.open_preview(ViewKind::Output, "output".into());
    app.select_preview(code_tab);
    assert_eq!(app.panes.list[super::Panes::SPLIT].view, ViewKind::Code);
    assert_eq!(app.panes.list[super::Panes::SPLIT].viewport.scroll, 7);
    app.select_preview(PreviewTab::Bookmark);
    assert_eq!(app.workspace.editor.reference_file(), Some("source.m2"));
    assert!(app.previews.label(source_tab).is_none());
    app.handle_reaction(crate::ui::controller::Reaction::ClosePane);
    app.select_preview(PreviewTab::Bookmark);
    assert!(app.panes.split());
    assert_eq!(app.workspace.editor.reference_file(), Some("source.m2"));
}

#[test]
fn loading_comments_does_not_display_them_or_leave_a_continuation() {
    use ratatui::{backend::TestBackend, Terminal};
    let mut app = super::App::new(crate::backend::M2Backend::spawn().unwrap());
    app.run_code("-- hidden heading\n-* hidden block *-");
    assert!(app.workspace.editor.analysis_input().1.is_none());
    assert!(!app.workspace.session.busy());
    app.run_code("-- hidden heading\nloadedCommentProbe = \"-- literal\" -- hidden tail\n\u{c}\n-- hidden saved block\n1 + 1;");
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(screen.contains("loadedCommentProbe"), "{screen}");
    assert!(screen.contains("-- literal"), "{screen}");
    assert!(!screen.contains("hidden"), "{screen}");
    assert!(app.workspace.editor.analysis_input().1.is_none());
}

#[test]
fn incomplete_file_and_live_input_keep_uniform_padding() {
    use ratatui::{backend::TestBackend, Terminal};
    let mut app = super::App::new(crate::backend::M2Backend::spawn().unwrap());
    app.run_code("f(\n    x");
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let input_row = terminal.get_cursor_position().unwrap().y;
    let buffer = terminal.backend().buffer();
    let source_row = (0..24)
        .find(|&y| {
            (0..80)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .contains("f(")
        })
        .expect("incomplete source is visible");
    let code_bg = crate::ui::render::RenderContext::new(80, 0)
        .groups()
        .code()
        .bg
        .unwrap();
    for y in [
        source_row - 1,
        source_row,
        input_row - 1,
        input_row,
        input_row + 1,
    ] {
        for x in 0..80 {
            assert_eq!(buffer[(x, y)].bg, code_bg, "background at ({x}, {y})");
        }
    }
}

#[test]
fn first_session_displays_initialization_output() {
    use ratatui::{backend::TestBackend, Terminal};
    let directory = ENV
        .temp_dir()
        .join(format!("spectral-startup-output-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let init = directory.join("init.m2");
    std::fs::write(&init, "print \"startup output probe\"\n").unwrap();
    let backend = crate::backend::M2Backend::spawn_with_init_file(Some(init)).unwrap();
    let mut app = super::App::new(backend);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    std::fs::remove_dir_all(directory).unwrap();
    assert!(screen.contains("startup output probe"), "{screen}");
}

#[test]
fn a_submitted_comment_is_visible_on_the_next_frame() {
    use ratatui::{backend::TestBackend, Terminal};
    let mut app = super::App::new(crate::backend::M2Backend::spawn().expect("spawn M2"));
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    for character in "--a".chars() {
        app.handle_key(crossterm::event::KeyCode::Char(character).into());
    }
    assert!(app.handle_key(crossterm::event::KeyCode::Enter.into()));
    app.poll();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(screen.contains("--a"), "{screen}");
}

use super::*;
use crate::env::ENV;
use std::time::Instant;

#[test]
fn ctrl_c_interrupts_while_busy_and_quits_when_idle() {
    let mut app = App::new(M2Backend::spawn().expect("spawn M2"));
    let update = app
        .workspace
        .session
        .submit("while true do null", &mut app.workspace.highlighter);
    app.workspace.editor.apply(update);
    std::thread::sleep(Duration::from_millis(100));
    assert!(app.handle_key(crate::key!(ctrl + c)));
    assert!(app.handle_key(crate::key!(ctrl + c)));
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.workspace.session.busy() && Instant::now() < deadline {
        for update in app.workspace.session.poll() {
            app.workspace.editor.apply(update);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!app.workspace.session.busy());
    assert!(!app.handle_key(crate::key!(ctrl + c)));
}

#[test]
fn cancelling_an_accidental_continuation_preserves_the_live_session() {
    use crate::ui::action::Action;
    let mut app = super::App::new(crate::backend::M2Backend::spawn().unwrap());
    app.run_code("(");
    assert!(app.workspace.editor.analysis_input().1.is_some());
    app.workspace.editor.paste("accidental continuation");
    assert!(app.handle_action(Action::CancelExecution));
    assert!(app.workspace.editor.analysis_input().1.is_none());
    assert!(app.workspace.editor.submission().is_empty());
    app.run_code("continuationProbe = 42;");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.workspace.session.busy() && std::time::Instant::now() < deadline {
        app.poll();
        std::thread::yield_now();
    }
    assert!(app.workspace.session.code().contains("continuationProbe"));
}

#[test]
fn viewing_a_file_preserves_the_draft_and_evaluates_nothing() {
    use crate::ui::controller::{Reaction, ViewKind};
    let directory = ENV
        .temp_dir()
        .join(format!("spectral-file-preview-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("preview.m2");
    std::fs::write(&path, "error \"must not execute\"").unwrap();
    let mut app = super::App::new(crate::backend::M2Backend::spawn().unwrap());
    app.workspace.editor.paste("unfinished draft");
    assert!(app.handle_reaction(Reaction::ViewFile(path.to_string_lossy().into_owned())));
    assert_eq!(app.workspace.editor.submission(), "unfinished draft");
    assert!(app.workspace.session.code().is_empty());
    assert!(!app.workspace.session.busy());
    assert_eq!(
        app.panes.list[super::Panes::SPLIT].view,
        ViewKind::Reference
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn clicked_documentation_is_rendered_outside_the_session_feed() {
    use crate::ui::controller::Reaction;
    use ratatui::{backend::TestBackend, Terminal};
    let mut app = super::App::new(crate::backend::M2Backend::spawn().unwrap());
    app.workspace.editor.paste("documentation draft");
    app.handle_reaction(Reaction::Documentation("\"isPrime\"".into()));
    let documentation_tab = app.previews.active.unwrap();
    app.open_preview(ViewKind::Code, "code".into());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut terminal = Terminal::new(TestBackend::new(160, 35)).unwrap();
    loop {
        app.poll();
        if !app.documentation_requests.is_empty() {
            assert_eq!(app.panes.list[super::Panes::SPLIT].view, ViewKind::Code);
        } else {
            app.select_preview(documentation_tab);
        }
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let split = app.pane_areas[super::Panes::SPLIT];
        let reference = split
            .positions()
            .filter(|position| position.y > split.y)
            .map(|position| buffer[position].symbol())
            .collect::<String>();
        if reference.contains("isPrime") && app.documentation_requests.is_empty() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "documentation did not arrive: {reference}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(app.workspace.editor.submission(), "documentation draft");
    assert!(app.workspace.session.code().is_empty());
    assert!(!app.workspace.session.busy());
}

#[test]
fn source_file_links_preview_encoded_paths_without_evaluating_them() {
    use ratatui::{backend::TestBackend, Terminal};
    let directory = ENV
        .temp_dir()
        .join(format!("spectral-source-link-{}", std::process::id(),));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("source file.m2");
    let source = "error \"sourcePreviewMustNotExecute\"";
    std::fs::write(&path, source).unwrap();
    let mut target = Url::from_file_path(&path).unwrap();
    target.set_fragment(Some("L1:C1"));
    for in_feed in [false, true] {
        let mut app = App::new(M2Backend::spawn().unwrap());
        app.workspace.editor.paste("preserved draft");
        let reaction = if in_feed {
            Reaction::DocumentationInFeed(target.to_string())
        } else {
            Reaction::Documentation(target.to_string())
        };
        assert!(app.handle_reaction(reaction));
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.workspace.session.busy() {
            assert!(Instant::now() < deadline);
            app.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(app.panes.split(), !in_feed);
        let mut terminal = Terminal::new(TestBackend::new(180, 35)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(screen.contains("sourcePreviewMustNotExecute"), "{screen}");
        assert_eq!(app.workspace.editor.submission(), "preserved draft");
        if in_feed {
            assert!(app
                .workspace
                .session
                .code()
                .contains("code new FilePosition"));
            assert!(app.documentation.is_none());
        } else {
            assert!(app.workspace.session.code().is_empty());
        }
        assert!(!app.workspace.session.busy());
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_documentation_link_can_open_in_the_feed_without_opening_a_split() {
    use ratatui::{backend::TestBackend, Terminal};
    let mut app = App::new(M2Backend::spawn().unwrap());
    app.workspace.editor.paste("preserved documentation draft");
    app.handle_reaction(Reaction::DocumentationInFeed("\"isPrime\"".into()));
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut terminal = Terminal::new(TestBackend::new(120, 35)).unwrap();
    loop {
        app.poll();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        if screen.contains("isPrime") && !app.workspace.session.busy() {
            break;
        }
        assert!(Instant::now() < deadline, "page did not arrive: {screen}");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!app.panes.split());
    assert_eq!(
        app.workspace.editor.submission(),
        "preserved documentation draft"
    );
    assert!(app.workspace.session.code().contains("help \"isPrime\""));
    assert!(app.documentation.is_none());
    assert!(!app.workspace.session.busy());
}

#[test]
fn control_arrows_move_the_inputs_cursor_by_words_with_a_preview_open() {
    use crate::ui::controller::ViewKind;
    use crate::ui::key::{KeyEvent, KeyMod};
    use crossterm::event::KeyCode;
    let mut app = super::App::new(crate::backend::M2Backend::spawn().unwrap());
    app.open_preview(ViewKind::Code, "code".into());
    app.open_preview(ViewKind::Output, "output".into());
    let shown = app.panes.list[super::Panes::SPLIT].view;
    assert!(app.panes.split());
    app.workspace.editor.paste("one two");

    app.handle_key(KeyCode::Left + KeyMod::Ctrl);
    assert_eq!(
        app.panes.list[super::Panes::SPLIT].view,
        shown,
        "the preview stays put"
    );
    app.workspace.editor.paste("X");
    assert_eq!(app.workspace.editor.analysis_input().0, "one Xtwo");

    app.handle_key(KeyEvent::new(KeyCode::PageUp).with(KeyMod::Ctrl));
    assert_ne!(
        app.panes.list[super::Panes::SPLIT].view,
        shown,
        "Ctrl+PageUp steps the preview"
    );
}
