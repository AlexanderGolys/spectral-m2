//! Views of the session shown beside its shell: what each shows, the pane state
//! it scrolls with, and how the editor draws it.

use crate::ui::settling::SettlingWidth;
use std::fs;
use std::path::PathBuf;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block as TuiBlock, Clear, Widget};
use ratatui::Frame;

use super::screen::CaretPane;
use super::Editor;
use crate::app::session::CommittedCell;
use crate::code::highlight::Highlighter;
use crate::ui::action::Action;
use crate::ui::block::{Block, Element, TextSpan};
use crate::ui::block::{Reformatted, Rethemed};
use crate::ui::cell::{CodeBlock, M2CodeCell, TextBlock};
use crate::ui::config::UI;
use crate::ui::cycle::cycled;
use crate::ui::element::EventListener;
use crate::ui::feed::BlockView;
use crate::ui::feed::Feed;
use crate::ui::graphics::kitty::ImageId;
use crate::ui::settings::rendering::OutputView;
use crate::ui::statusline::{StatusLine, StatusLineContext};
use crate::ui::syntax_tree::SyntaxTreeBlock;
use crate::ui::theme::groups::{scheme, HLGroup};

/// Content rendered by one workspace pane.
#[discriminated_str(label)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ViewKind {
    #[default]
    Shell = "shell",
    Code = "code",
    /// What the session answered, without input cells, as the *output view* setting
    /// selects it.
    Output = "output",
    SyntaxTree = "syntax tree",
    /// Documentation and source files opened for reference.
    Reference = "reference",
}

impl ViewKind {
    /// The views shown beside the session, in the order `Left` and `Right` step
    /// through them.
    pub const PREVIEWS: [Self; 3] = [Self::Code, Self::Output, Self::SyntaxTree];

    pub fn is_preview(self) -> bool {
        Self::PREVIEWS.contains(&self)
    }

    /// The view `delta` steps away from this one, wrapping around the previews;
    /// only a preview steps.
    pub fn cycle(self, delta: i8) -> Self {
        cycled(&Self::PREVIEWS, self, delta)
    }
}

/// Independent scrolling state for one pane.
#[derive(Default)]
pub struct Viewport {
    pub scroll: usize,
    preview: Option<PreviewCache>,
    /// The width the view's content is laid out at while its pane is resized.
    pub width: SettlingWidth,
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
                            M2CodeCell::highlighted(
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
                            M2CodeCell::highlighted(
                                cell.code.clone(),
                                None,
                                cell.source_id,
                                highlighter,
                            ),
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
    pub fn scroll_reference(&mut self, rows: isize) {
        self.reference.scroll(rows);
    }

    /// Scroll the reference pane by whole pages, positive toward its start.
    pub fn page_reference(&mut self, pages: isize) {
        self.reference
            .scroll(pages * self.page_rows(self.reference.height()));
    }

    pub fn reference_to_block_start(&mut self) {
        self.reference.scroll_to_block_start();
    }

    pub fn reference_to_block_end(&mut self) {
        self.reference.scroll_to_block_end();
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
            true if view.is_preview() => &ViewKind::PREVIEWS,
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
            .render_context(viewport.width.width(sections[0].width))
            .with_swapped_code_background(swapped);
        self.feed_area = sections[0];
        self.picker_area = sections[1];

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
                let Viewport {
                    scroll, preview, ..
                } = viewport;
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
            ViewKind::Output => {
                let context = context.full_output().with_printed_numbers();
                let blocks = match self.settings.rendering.output_view {
                    OutputView::Everything => self.feed.outputs(),
                    OutputView::Values => self.feed.values(),
                    OutputView::Printed => self.feed.printed(),
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
        self.place_caret(frame, CaretPane::View);
    }
}
