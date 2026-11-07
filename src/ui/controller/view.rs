//! Views of the session shown beside its shell: what each shows, the pane state
//! it scrolls with, and how the editor draws it.

use std::fs;
use std::path::PathBuf;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block as TuiBlock, Clear, Widget};
use ratatui::Frame;
use these_macros_should_be_illegal::discriminated_str;

use super::settings::theme::groups::{scheme, HLGroup};
use super::Editor;
use crate::app::session::CommittedCell;
use crate::code::highlight::Highlighter;
use crate::ui::action::Action;
use crate::ui::block::{Block, Element, TextSpan};
use crate::ui::block::{Reformatted, Rethemed};
use crate::ui::cell::{Cell, CodeBlock, TextBlock};
use crate::ui::config::UI;
use crate::ui::cycle::cycled;
use crate::ui::element::EventListener;
use crate::ui::feed::BlockView;
use crate::ui::feed::Feed;
use crate::ui::graphics::kitty::ImageId;
use crate::ui::statusline::{StatusLine, StatusLineContext};
use crate::ui::syntax_tree::SyntaxTreeBlock;

/// Content rendered by one workspace pane.
#[discriminated_str(label)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewKind {
    #[default]
    Shell = "shell",
    Code = "code",
    /// Returned values alone.
    Values = "values",
    /// Text the cells printed alone.
    StdOut = "stdout",
    /// Printed text and returned values together, without input cells.
    Output = "output",
    SyntaxTree = "syntax tree",
    /// Documentation and source files opened for reference.
    Reference = "reference",
}

impl ViewKind {
    /// The views shown beside the session, in the order `Left` and `Right` step
    /// through them.
    const PREVIEWS: [Self; 5] = [
        Self::Code,
        Self::Values,
        Self::StdOut,
        Self::Output,
        Self::SyntaxTree,
    ];

    /// The view `delta` steps away from this one, wrapping around the previews.
    pub fn cycle(self, delta: i8) -> Self {
        cycled(&Self::PREVIEWS, self, delta)
    }
}

/// Independent scrolling state for one pane.
#[derive(Default)]
pub struct Viewport {
    pub scroll: usize,
    preview: Option<PreviewCache>,
}

/// Immutable committed cells paired with the revision that changes them.
pub struct CommittedCells<'a> {
    /// Cells in evaluation order.
    pub cells: &'a [CommittedCell],
    /// Workspace revision for cache invalidation.
    pub revision: u64,
}

/// Parsed block model retained by one code or syntax-tree preview pane.
struct PreviewCache {
    revision: u64,
    highlight_revision: u64,
    view: ViewKind,
    blocks: Vec<Box<dyn Block>>,
}

/// A retained reference, including its viewport and source-file action.
pub struct ReferenceEntry {
    feed: Feed,
    file: Option<String>,
}

impl ReferenceEntry {
    /// The pictures this reference holds, though it is not showing.
    pub fn pictures(&self) -> Vec<ImageId> {
        self.feed.pictures()
    }

    pub fn new(blocks: Vec<Box<dyn Block>>, file: Option<String>) -> Self {
        let mut feed = Feed::new(Box::new(TextBlock::system(String::new())));
        for block in blocks {
            feed.push(block);
        }
        feed.scroll_to_top();
        Self { feed, file }
    }
}

impl std::fmt::Debug for ReferenceEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReferenceEntry")
            .field("file", &self.file)
            .field("scroll", &self.feed.offset())
            .finish_non_exhaustive()
    }
}

impl Viewport {
    /// A viewport that opens on the newest content, as every view does.
    pub fn at_bottom() -> Self {
        let mut viewport = Self::default();
        viewport.follow_bottom();
        viewport
    }

    /// Scroll to the newest content; rendering clamps this to the last full page.
    pub fn follow_bottom(&mut self) {
        self.scroll = usize::MAX;
    }

    /// Rebuild the cached blocks of a code or syntax-tree view when the committed
    /// cells have changed since they were built.
    fn refresh_preview(
        &mut self,
        committed: CommittedCells<'_>,
        view: ViewKind,
        highlighter: &mut dyn Highlighter,
    ) {
        if self.preview.as_ref().is_none_or(|cache| {
            cache.revision != committed.revision
                || cache.view != view
                || cache.highlight_revision != highlighter.revision()
        }) {
            let blocks: Vec<Box<dyn Block>> = match view {
                ViewKind::Code => (!committed.cells.is_empty())
                    .then(|| {
                        Box::new(CodeBlock::new(committed.cells.iter().map(|cell| {
                            Cell::highlighted(
                                cell.code.clone(),
                                Some((cell.number, cell.state)),
                                cell.source_id,
                                highlighter,
                            )
                        }))) as Box<dyn Block>
                    })
                    .into_iter()
                    .collect(),
                ViewKind::SyntaxTree => committed
                    .cells
                    .iter()
                    .map(|cell| {
                        Box::new(SyntaxTreeBlock::new(
                            Cell::highlighted(cell.code.clone(), None, cell.source_id, highlighter),
                            cell.tree.clone(),
                        )) as Box<dyn Block>
                    })
                    .collect(),
                _ => unreachable!("only committed previews are cached"),
            };
            self.preview = Some(PreviewCache {
                revision: committed.revision,
                highlight_revision: highlighter.revision(),
                view,
                blocks,
            });
        }
    }
}

impl Editor {
    /// Open reference content in the active view.
    pub fn open_reference(&mut self, blocks: Vec<Box<dyn Block>>, file: Option<String>) {
        self.reference_file = file;
        self.set_reference(blocks);
    }

    /// Retain the active reference, including its scroll and fold state.
    pub fn take_reference(&mut self) -> ReferenceEntry {
        ReferenceEntry {
            feed: std::mem::replace(
                &mut self.reference,
                Feed::new(Box::new(TextBlock::system(String::new()))),
            ),
            file: self.reference_file.take(),
        }
    }

    /// Restore a retained reference under the current rendering settings.
    pub fn restore_reference(&mut self, entry: ReferenceEntry) {
        self.reference = entry.feed;
        self.reference_file = entry.file;
        self.reference.on_event(&Rethemed);
        self.reference.on_event(&Reformatted);
    }

    /// Source file available to the reference pane's run action.
    pub fn reference_file(&self) -> Option<&str> {
        self.reference_file.as_deref()
    }

    /// Replace the reference pane's content.
    pub fn set_reference(&mut self, blocks: Vec<Box<dyn Block>>) {
        self.reference.clear();
        for block in blocks {
            self.reference.push(block);
        }
        self.reference.scroll_to_top();
        self.reference.focus_target();
    }

    /// Scroll the reference pane independently of the transcript.
    pub fn scroll_reference(&mut self, rows: i8) {
        self.reference.scroll(rows);
    }

    /// Offer archived session files to preview.
    pub fn browse_sessions(&mut self, paths: Vec<PathBuf>) {
        use super::picker::{Picker, PickerController, PickerMode, Row};
        use super::EditorEvent;
        let items = paths
            .into_iter()
            .map(|path| {
                let label = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let summary = fs::read_to_string(&path)
                    .ok()
                    .and_then(|source| {
                        source
                            .lines()
                            .find(|line| !line.trim().is_empty())
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| "empty session".into());
                let path = path.to_string_lossy().into_owned();
                Row::enter(vec![summary.into(), label.into()], move || {
                    EditorEvent::ViewFile(path.clone())
                })
                .into()
            })
            .collect();
        self.pickers.push(PickerController {
            mode: PickerMode::Commands,
            picker: Picker::new("saved sessions", items),
        });
        self.refresh_status_line();
    }

    /// A view's status: the views in the centre and, while it has focus, the
    /// keys it answers to on the right. While the view has focus the centre lists every view `Left` and `Right` step through, its own
    /// highlighted; otherwise it names the view alone.
    fn preview_status(&self, view: ViewKind, focused: bool) -> StatusLineContext {
        let shown: &[ViewKind] = match focused {
            true if view != ViewKind::Reference => &ViewKind::PREVIEWS,
            true => &[view],
            false => &[view],
        };
        let center = shown
            .iter()
            .enumerate()
            .map(|(index, &preview)| {
                let gap = if index == 0 { "" } else { " " };
                TextSpan::new(
                    format!("{gap}{}", preview.label()),
                    match preview == view {
                        true => HLGroup::ViewLabelActive,
                        false => HLGroup::ViewLabelInactive,
                    },
                )
            })
            .collect();
        let action = if self.split { "close" } else { "leave" };
        let right = self
            .keymaps
            .key_for(Action::ClosePane)
            .filter(|_| focused)
            .map(|key| {
                vec![
                    TextSpan::new(format!("{key} "), HLGroup::KeymapHint),
                    TextSpan::new(format!("{action} view"), HLGroup::StatuslineHintLabel),
                ]
            })
            .unwrap_or_default();
        StatusLineContext {
            center: Some(center),
            right,
            ..self.status_context()
        }
    }

    /// One status line across both panes of a split: the session's state on the
    /// left, the view beside it in the centre, and the focused pane's keys on the
    /// right.
    pub fn draw_shared_status(
        &self,
        frame: &mut Frame,
        area: Rect,
        view: ViewKind,
        view_focused: bool,
    ) {
        let buffer = frame.buffer_mut();
        TuiBlock::default()
            .style(scheme().statusline_bg())
            .render(area, buffer);
        let status = self.preview_status(view, view_focused);
        match view_focused && self.pickers.is_empty() && !self.cancel_hint_shown {
            true => StatusLine::default().render(&status, area, buffer),
            false => self.status_line.render(&status, area, buffer),
        }
    }

    /// The column between a split's panes, a faint frame line over the canvas of
    /// the view on its right.
    pub fn draw_divider(&self, frame: &mut Frame, area: Rect) {
        let style = self.view_canvas().patch(scheme().view_divider());
        let buffer = frame.buffer_mut();
        for y in area.top()..area.bottom() {
            buffer[(area.x, y)]
                .set_symbol(UI.glyphs.split_divider.current())
                .set_style(style);
        }
    }

    /// Match each margin row to the view beside it after that view is drawn.
    pub fn draw_margin(&self, frame: &mut Frame, area: Rect) {
        if area.right() >= frame.area().right() {
            return;
        }
        for y in area.top()..area.bottom() {
            let bg = frame.buffer_mut()[(area.right(), y)].bg;
            for x in area.left()..area.right() {
                frame.buffer_mut()[(x, y)].set_bg(bg);
            }
        }
    }

    /// The one background of every view: the code block's, under code and output
    /// alike, so nothing reads as a seam between blocks.
    fn view_canvas(&self) -> Style {
        self.settings.appearance.canvas(scheme().code())
    }

    pub fn draw_preview(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        highlighter: &mut dyn Highlighter,
        committed: CommittedCells<'_>,
        view: ViewKind,
        viewport: &mut Viewport,
    ) {
        if self.pane_focused {
            self.frame_stats.tick();
        }
        self.report_picture_distances();
        self.selection_inclusions.clear();
        self.input.hide();
        let theme = *scheme();
        frame.render_widget(Clear, area);
        let reference = view == ViewKind::Reference;
        let swapped = reference && self.settings.rendering.reference_code_background;
        let canvas = self.view_canvas();
        TuiBlock::default()
            .style(canvas)
            .render(area, frame.buffer_mut());
        let picker_height = self
            .pickers
            .last()
            .filter(|_| self.pane_focused)
            .map_or(0, |controller| controller.picker.height(area.width))
            .min(area.height.saturating_sub(1));
        let status_height = u16::from(!self.split).min(area.height);
        let sections = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(picker_height),
            Constraint::Length(status_height),
        ])
        .split(area);
        let context = self
            .render_context(sections[0].width)
            .with_swapped_code_background(swapped);
        self.feed_area = sections[0];

        match view {
            ViewKind::Reference => {
                let (_, inclusions) = self.reference.render(
                    &context.full_output(),
                    sections[0],
                    frame.buffer_mut(),
                    canvas,
                );
                self.selection_inclusions.merge(inclusions);
            }
            ViewKind::Shell => unreachable!("shells use Editor::draw"),
            ViewKind::Code | ViewKind::SyntaxTree => {
                viewport.refresh_preview(committed, view, highlighter);
                let Viewport { scroll, preview } = viewport;
                let blocks = preview
                    .iter()
                    .flat_map(|cache| cache.blocks.iter().map(Box::as_ref))
                    .collect::<Vec<_>>();
                let empty = match view {
                    ViewKind::Code => "no committed code",
                    _ => "no committed syntax trees",
                };
                self.selection_inclusions
                    .merge(BlockView::new(&blocks, empty).render(
                        &context,
                        sections[0],
                        frame.buffer_mut(),
                        scroll,
                        canvas,
                    ));
            }
            ViewKind::Values | ViewKind::StdOut | ViewKind::Output => {
                let context = context.full_output().with_printed_numbers();
                let blocks = match view {
                    ViewKind::Values => self.feed.values(),
                    ViewKind::StdOut => self.feed.printed(),
                    _ => self.feed.outputs(),
                };
                self.selection_inclusions.merge(
                    BlockView::new(&blocks, "no matching output").render(
                        &context,
                        sections[0],
                        frame.buffer_mut(),
                        &mut viewport.scroll,
                        canvas,
                    ),
                );
            }
        }
        // Blocks such as printed text paint a band of their own; a view keeps one
        // background under all of them.
        let background = canvas.bg.unwrap_or(Color::Reset);
        if !reference {
            for position in sections[0].positions() {
                frame.buffer_mut()[position].set_bg(background);
            }
        }
        if reference {
            self.paint_link_hover(
                frame,
                self.pointer
                    .and_then(|pointer| self.reference.link_at(pointer)),
            );
        }
        self.feed_top = if view == ViewKind::Reference {
            -(self.reference.offset() as i64)
        } else {
            viewport.scroll as i64
        };
        if let Some(controller) = self.pickers.last().filter(|_| self.pane_focused) {
            controller
                .picker
                .render(&(), sections[1], frame.buffer_mut());
        }
        if status_height > 0 {
            TuiBlock::default()
                .style(theme.statusline_bg())
                .render(sections[2], frame.buffer_mut());
            let status = self.preview_status(view, self.pane_focused);
            match self.pickers.is_empty() {
                true => StatusLine::default().render(&status, sections[2], frame.buffer_mut()),
                false => self
                    .status_line
                    .render(&status, sections[2], frame.buffer_mut()),
            }
        }
        self.draw_highlight_debug(frame);
        self.capture_screen(frame);
        self.paint_screen_selection(frame);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;
    use crate::code::highlight::SyntaxHighlighter;
    use crate::ui::cell::{CellState, TextBlock};
    use crate::ui::controller::EditorUpdate;

    #[test]
    fn retained_reference_restores_content_scroll_and_source_action() {
        let mut editor = Editor::new(String::new());
        let source = (0..80)
            .map(|line| format!("source {line}\n"))
            .collect::<String>();
        editor.open_reference(
            vec![Box::new(TextBlock::system(source))],
            Some("first.m2".into()),
        );
        drawn_rows(&mut editor, ViewKind::Reference);
        editor.scroll_reference(-5);
        drawn_rows(&mut editor, ViewKind::Reference);
        let offset = editor.reference.offset();
        assert!(offset > 0);
        let retained = editor.take_reference();
        editor.open_reference(vec![Box::new(TextBlock::system("another reference"))], None);
        assert!(editor.reference_file().is_none());
        editor.restore_reference(retained);
        assert_eq!(editor.reference.offset(), offset);
        assert_eq!(editor.reference_file(), Some("first.m2"));
        let rows = drawn_rows(&mut editor, ViewKind::Reference);
        assert!(rows.iter().any(|row| row.contains("source ")));
        assert!(!rows.iter().any(|row| row.contains("another reference")));
    }

    #[test]
    fn reference_values_obey_the_same_selected_math_renderer_as_the_feed() {
        use crate::backend::M2Value;
        use crate::ui::cell::OutputBlock;
        use crate::ui::render::maths::MathDisplay;
        let mut editor = Editor::new(String::new());
        editor.settings.rendering.maths = MathDisplay::Ascii;
        editor.set_reference(vec![Box::new(
            OutputBlock::rendered(
                M2Value::Tex {
                    source: "$\\frac{1}{2}$".into(),
                    fallback: "1/2".into(),
                    code: "1/2".into(),
                },
                None,
            )
            .without_frame(),
        )]);
        let rows = drawn_rows(&mut editor, ViewKind::Reference);
        assert!(rows.iter().any(|row| row.contains("1/2")), "{rows:?}");
    }

    #[test]
    fn reference_code_background_keeps_the_panel_canvas_and_text_colours() {
        use crate::backend::M2Value;
        use crate::ui::cell::OutputBlock;
        let mut editor = Editor::new(String::new());
        editor.set_context(true, true, true);
        editor.set_reference(vec![Box::new(
            OutputBlock::rendered(
                M2Value::Markup {
                    html: "<p>CanvasProbe</p><pre>codeProbe = 1</pre>".into(),
                    fallback: String::new(),
                    example_width: 60,
                },
                None,
            )
            .without_frame(),
        )]);
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let mut colours = Vec::new();
        for swapped in [false, true, false] {
            editor.handle_action(Action::OpenSettings);
            let mut settings = editor.settings;
            settings.rendering.reference_code_background = swapped;
            editor.set_settings(settings);
            editor.pickers.clear();
            terminal
                .draw(|frame| {
                    editor.draw_preview(
                        frame,
                        frame.area(),
                        &mut SyntaxHighlighter::new(),
                        CommittedCells {
                            cells: &[],
                            revision: 0,
                        },
                        ViewKind::Reference,
                        &mut Viewport::default(),
                    )
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            let cell_at = |text: &str| {
                for y in 0..buffer.area.height {
                    let row: String = (0..buffer.area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect();
                    if let Some(x) = row.find(text) {
                        return buffer[(x as u16, y)].clone();
                    }
                }
                panic!("missing rendered text: {text}");
            };
            let canvas = cell_at("CanvasProbe");
            let code = cell_at("codeProbe");
            colours.push((canvas.bg, code.bg, code.fg));
        }
        let (canvas, _, foreground) = colours[0];
        assert_eq!(colours[1].0, canvas);
        assert_eq!(colours[1].2, foreground);
        assert_eq!(colours[2], colours[0]);
    }

    /// The rows `view` draws, as plain text.
    fn drawn_rows(editor: &mut Editor, view: ViewKind) -> Vec<String> {
        editor.set_context(true, true, true);
        let mut terminal = Terminal::new(TestBackend::new(30, 16)).expect("test terminal");
        terminal
            .draw(|frame| {
                editor.draw_preview(
                    frame,
                    frame.area(),
                    &mut SyntaxHighlighter::new(),
                    CommittedCells {
                        cells: &[],
                        revision: 0,
                    },
                    view,
                    &mut Viewport::default(),
                )
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn outer_margin_follows_each_adjacent_row_background() {
        let editor = Editor::new(String::new());
        let mut terminal = Terminal::new(TestBackend::new(5, 2)).unwrap();
        terminal
            .draw(|frame| {
                frame.buffer_mut()[(1, 0)].set_bg(Color::Rgb(10, 20, 30));
                frame.buffer_mut()[(1, 1)].set_bg(Color::Rgb(40, 50, 60));
                editor.draw_margin(frame, Rect::new(0, 0, 1, 2));
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        for y in 0..2 {
            assert_eq!(buffer[(0, y)].bg, buffer[(1, y)].bg);
        }
    }

    #[test]
    fn each_cells_printed_text_carries_its_number_and_no_rule() {
        let mut editor = Editor::new(String::new());
        editor.post(EditorUpdate::Pending {
            id: 1,
            cells: vec![("a".to_string(), Vec::new()), ("b".to_string(), Vec::new())],
        });
        for (index, (number, text)) in [(1, "first"), (2, "second")].into_iter().enumerate() {
            editor.apply(EditorUpdate::CellSucceeded {
                id: 1,
                index,
                number,
                state: CellState::Null,
                outputs: Vec::new(),
                printed: Some(TextBlock::printed(text)),
                value: None,
            });
        }

        let rows = drawn_rows(&mut editor, ViewKind::StdOut);
        let row_of = |needle: &str| {
            rows.iter()
                .position(|row| row.contains(needle))
                .unwrap_or_else(|| panic!("{needle:?} is drawn in {rows:#?}"))
        };
        assert!(row_of("first") < row_of("second"));
        for (number, text) in [(1, "first"), (2, "second")] {
            let row = rows[row_of(text)].trim_start();
            assert!(
                row.starts_with(&number.to_string()),
                "{row:?} is numbered {number}"
            );
        }
        assert!(
            rows.iter().all(|row| !row.contains('\u{2500}')),
            "no rule is drawn"
        );
    }

    #[test]
    fn printed_output_sits_on_the_code_background_like_everything_else() {
        let mut editor = Editor::new(String::new());
        editor.apply(EditorUpdate::Message(Box::new(TextBlock::printed(
            "printed\ntext",
        ))));
        editor.set_context(true, true, true);
        let mut terminal = Terminal::new(TestBackend::new(30, 8)).expect("test terminal");
        terminal
            .draw(|frame| {
                editor.draw_preview(
                    frame,
                    frame.area(),
                    &mut SyntaxHighlighter::new(),
                    CommittedCells {
                        cells: &[],
                        revision: 0,
                    },
                    ViewKind::Output,
                    &mut Viewport::at_bottom(),
                )
            })
            .expect("draw");

        let code = editor.view_canvas().bg.unwrap_or(Color::Reset);
        let buffer = terminal.backend().buffer();
        assert!(buffer.content().iter().any(|cell| cell.symbol() == "p"));
        assert!(buffer.content().iter().all(|cell| cell.bg == code));
    }

    #[test]
    fn a_focused_view_lists_every_view_and_highlights_its_own() {
        let editor = Editor::new(String::new());
        let view = ViewKind::Values;
        let listed = |focused| -> Vec<(String, HLGroup)> {
            editor
                .preview_status(view, focused)
                .center
                .expect("a view's status has a centre")
                .into_iter()
                .map(|span| (span.content.trim().to_string(), span.kind))
                .collect()
        };

        let focused = listed(true);
        assert_eq!(focused.len(), ViewKind::PREVIEWS.len());
        for (label, group) in focused {
            let expected = match label == view.label() {
                true => HLGroup::ViewLabelActive,
                false => HLGroup::ViewLabelInactive,
            };
            assert_eq!(group, expected, "{label}");
        }
        assert_eq!(
            listed(false),
            [(view.label().to_string(), HLGroup::ViewLabelActive)]
        );
    }

    #[test]
    fn only_a_focused_view_claims_the_right_side_of_the_status_line() {
        let editor = Editor::new(String::new());
        assert!(!editor
            .preview_status(ViewKind::Values, true)
            .right
            .is_empty());
        assert!(editor
            .preview_status(ViewKind::Values, false)
            .right
            .is_empty());
    }

    #[test]
    fn stepping_through_the_previews_visits_each_once_and_returns() {
        let mut view = ViewKind::Code;
        let visited: Vec<_> = (0..ViewKind::PREVIEWS.len())
            .map(|_| {
                view = view.cycle(1);
                view
            })
            .collect();

        assert_eq!(view, ViewKind::Code);
        assert!(ViewKind::PREVIEWS
            .iter()
            .all(|preview| visited.contains(preview)));
        assert!(!visited.contains(&ViewKind::Shell));
        assert_eq!(view.cycle(1).cycle(-1), view);
    }
    #[test]
    fn semantic_revision_refreshes_a_preview_without_new_committed_source() {
        use crate::code::semantic::SourceId;
        use crate::ui::block::HLSpan;
        use crate::ui::cell::CellState;
        use m2_syn::{NativeCellParser, PrettyTree};
        struct ChangingHighlighter {
            revision: u64,
            projections: usize,
        }
        impl Highlighter for ChangingHighlighter {
            fn revision(&self) -> u64 {
                self.revision
            }
            fn highlight(&mut self, _: &str) -> Vec<HLSpan> {
                Vec::new()
            }
            fn highlight_source(&mut self, _: SourceId, _: &str) -> Vec<HLSpan> {
                self.projections += 1;
                Vec::new()
            }
        }
        let mut parser = NativeCellParser::new();
        parser.push_line("f = 1");
        let tree = parser.take_completed()[0].syntax().pretty_tree();
        let cells = [CommittedCell {
            source_id: SourceId::Cell {
                generation: 0,
                submission: 1,
                index: 0,
            },
            number: 1,
            state: CellState::Value,
            code: "f = 1".to_string(),
            tree,
            block: 1,
        }];
        let mut viewport = Viewport {
            scroll: 3,
            ..Viewport::default()
        };
        let mut highlighter = ChangingHighlighter {
            revision: 0,
            projections: 0,
        };
        for revision in [0, 0, 1] {
            highlighter.revision = revision;
            viewport.refresh_preview(
                CommittedCells {
                    cells: &cells,
                    revision: 1,
                },
                ViewKind::Code,
                &mut highlighter,
            );
        }
        assert_eq!(highlighter.projections, 2);
        assert_eq!(viewport.scroll, 3);
    }
}
