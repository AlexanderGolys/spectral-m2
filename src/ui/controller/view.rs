//! Views of the session shown beside its shell: what each shows, the pane state
//! it scrolls with, and how the editor draws it.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block as TuiBlock, Clear, Widget};
use ratatui::Frame;
use these_macros_should_be_illegal::discriminated_str;

use super::settings::theme::groups::{scheme, HLGroup};
use super::{cycled, Editor};
use crate::app::session::CommittedCell;
use crate::code::highlight::Highlighter;
use crate::ui::action::Action;
use crate::ui::block::{Block, Element, TextSpan};
use crate::ui::cell::{Cell, CodeBlock};
use crate::ui::config::UI;
use crate::ui::feed::BlockView;
use crate::ui::render::RenderContext;
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
    view: ViewKind,
    blocks: Vec<Box<dyn Block>>,
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
        if self
            .preview
            .as_ref()
            .is_none_or(|cache| cache.revision != committed.revision || cache.view != view)
        {
            let blocks: Vec<Box<dyn Block>> = match view {
                ViewKind::Code => (!committed.cells.is_empty())
                    .then(|| {
                        Box::new(CodeBlock::new(committed.cells.iter().map(|cell| {
                            Cell::resolved(
                                cell.code.clone(),
                                cell.number,
                                cell.state,
                                highlighter.highlight(&cell.code),
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
                            cell.code.clone(),
                            highlighter.highlight(&cell.code),
                            cell.tree.clone(),
                        )) as Box<dyn Block>
                    })
                    .collect(),
                _ => unreachable!("only committed previews are cached"),
            };
            self.preview = Some(PreviewCache {
                revision: committed.revision,
                view,
                blocks,
            });
        }
    }
}

impl Editor {
    /// A view's status: the session's state on the left, the views in the
    /// centre, and the keys it answers to on the right. While the view has focus
    /// the centre lists every view `Left` and `Right` step through, its own
    /// highlighted; otherwise it names the view alone.
    fn preview_status(&self, view: ViewKind, focused: bool) -> StatusLineContext {
        let shown: &[ViewKind] = match focused {
            true => &ViewKind::PREVIEWS,
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
                        true => HLGroup::Documentation,
                        false => HLGroup::Muted,
                    },
                )
            })
            .collect();
        let action = if self.split { "close" } else { "leave" };
        let right = self
            .keymaps
            .key_for(Action::ClosePane)
            .map(|key| {
                vec![
                    TextSpan::new(format!("{key} "), HLGroup::KeymapHint),
                    TextSpan::new(format!("{action} view"), HLGroup::Muted),
                ]
            })
            .unwrap_or_default();
        StatusLineContext {
            idle_left: self.shell_state(),
            center: Some(center),
            right,
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
        match view_focused && self.pickers.is_empty() {
            true => StatusLine::default().render(&status, area, buffer),
            false => self.status_line.render(&status, area, buffer),
        }
    }

    /// The column between a split's panes, a faint frame line over the canvas of
    /// the view on its right.
    pub fn draw_divider(&self, frame: &mut Frame, area: Rect) {
        let style = self.view_canvas().patch(scheme().border());
        let buffer = frame.buffer_mut();
        for y in area.top()..area.bottom() {
            buffer[(area.x, y)]
                .set_symbol(UI.split_divider)
                .set_style(style);
        }
    }

    /// Fill `area`, the margin around the views, with their background.
    pub fn draw_margin(&self, frame: &mut Frame, area: Rect) {
        frame.render_widget(TuiBlock::default().style(self.view_canvas()), area);
    }

    /// The one background of every view: the code block's, under code and output
    /// alike, so nothing reads as a seam between blocks.
    fn view_canvas(&self) -> Style {
        self.appearance.canvas(scheme().code())
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
        self.selection_exclusions.clear();
        self.input.hide();
        let theme = *scheme();
        frame.render_widget(Clear, area);
        let canvas = self.view_canvas();
        TuiBlock::default()
            .style(canvas)
            .render(area, frame.buffer_mut());
        let picker_height = self
            .pickers
            .last()
            .map_or(0, |controller| controller.picker.height())
            .min(area.height.saturating_sub(1));
        let status_height = u16::from(!self.split).min(area.height);
        let sections = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(picker_height),
            Constraint::Length(status_height),
        ])
        .split(area);
        let context = RenderContext::new(sections[0].width, 0);
        self.feed_area = sections[0];

        match view {
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
                self.selection_exclusions
                    .extend(BlockView::new(&blocks, empty).render(
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
                self.selection_exclusions.extend(
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
        for position in sections[0].positions() {
            frame.buffer_mut()[position].set_bg(background);
        }
        self.feed_top = viewport.scroll as i64;
        self.selection_exclusions.extend([sections[1], sections[2]]);
        if let Some(controller) = self.pickers.last() {
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
                false => self.status_line.render(
                    &StatusLineContext {
                        center: status.center,
                        ..StatusLineContext::default()
                    },
                    sections[2],
                    frame.buffer_mut(),
                ),
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
                true => HLGroup::Documentation,
                false => HLGroup::Muted,
            };
            assert_eq!(group, expected, "{label}");
        }
        assert_eq!(
            listed(false),
            [(view.label().to_string(), HLGroup::Documentation)]
        );
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
}
