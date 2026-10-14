//! Operations emitted by keymaps and other UI event sources.

use serde::{Deserialize, Serialize};
use these_macros_should_be_illegal::enum_fn;

/// A small by-value operation routed through the focused UI hierarchy.
#[enum_fn(description: &'static str)]
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    FocusPaneLeft = "focus the left pane",
    FocusPaneRight = "focus the right pane",
    ResizePaneLeft = "move the pane divider left",
    ResizePaneRight = "move the pane divider right",
    Quit = "quit spectral-m2",
    Submit = "evaluate the input",
    ScrollFeedUp = "scroll the feed up",
    ScrollFeedDown = "scroll the feed down",
    OpenCommands = "open commands",
    OpenSettings = "open settings",
    OpenKeymaps = "open keymaps",
    ClearFeed = "clear the feed",
    OpenCode = "open code",
    OpenOutput = "open output",
    OpenSyntaxTree = "open syntax tree",
    Restart = "restart Macaulay2",
    ClosePane = "close the view",
    ToggleHighlightDebug = "toggle highlight groups",
    CopySelection = "copy the selection",
}

impl Action {
    /// Actions whose user binding may be replaced.
    pub const REMAPPABLE: &'static [Self] = &[
        Self::OpenCommands,
        Self::OpenSettings,
        Self::OpenKeymaps,
        Self::ClearFeed,
        Self::OpenCode,
        Self::OpenOutput,
        Self::OpenSyntaxTree,
        Self::Restart,
        Self::ClosePane,
        Self::Quit,
        Self::ToggleHighlightDebug,
        Self::CopySelection,
    ];

    pub fn is_remappable(self) -> bool {
        Self::REMAPPABLE.contains(&self)
    }
}
