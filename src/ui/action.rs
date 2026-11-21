//! Operations emitted by keymaps and other UI event sources.

use serde::{Deserialize, Serialize};

/// A small by-value operation routed through the focused UI hierarchy.
#[discriminated_str(description)]
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Action {
    Quit = "quit spectral-m2",
    CancelExecution = "cancel execution",
    Submit = "evaluate the input",
    ScrollFeedUp = "scroll the feed up",
    ScrollFeedDown = "scroll the feed down",
    ScrollBlockStart = "scroll to the start of the block at the top, or of the one before",
    ScrollBlockEnd = "scroll to the end of the block at the bottom, or of the one after",
    OpenCommands = "open commands",
    OpenSettings = "open settings",
    OpenKeymaps = "open keymaps",
    ClearFeed = "clear the feed",
    OpenCode = "open code",
    OpenOutput = "open output",
    OpenNotes = "open the notes notebook",
    OpenSyntaxTree = "open syntax tree",
    RecentSession = "view the previous session without running it",
    BrowseSessions = "browse saved sessions",
    RunViewedFile = "restart and run the viewed file",
    ViewFile = "view a file without running it",
    SaveSelection = "save selected code to a file",
    SaveSession = "save the session's code to a file",
    LoadCode = "run the code in a file",
    ResumeSession = "run the previous session's code",
    Restart = "restart Macaulay2",
    #[serde(alias = "ClosePane")]
    CloseTab = "close the tab showing beside the session",
    #[serde(alias = "ReferenceBack")]
    HistoryBack = "go back to what the view beside the session showed before",
    HistoryForward = "go forward again through what the view beside the session showed",
    FocusPanel = "move between the session and the view beside it, showing the view",
    TogglePanel = "hide the view beside the session, or show it again",
    ToggleHighlightDebug = "toggle highlight groups",
    CopySelection = "copy the selection",
    CutSelection = "cut the input's selection, or its current line",
    SelectPreviousBlock = "select the previous cell or output",
    SelectNextBlock = "select the next cell or output",
    Describe = "tell what the name at the cursor is",
    DescribeBeside = "describe the name at the cursor beside the session",
    GoToDefinition = "go to where the name at the cursor is defined",
    GoToDocumentation = "open the documentation of the name at the cursor beside the session",
    FormatInput = "format the input",
    ToggleViewCursor = "move the cursor into the feed or the view beside it, and back",
}

/// Where a link opens in the view beside the session.
#[discriminated_str(description)]
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Opening {
    Here = "open the link in the tab showing",
    Beside = "open the link in a new tab, staying on this one",
    BesideAndJump = "open the link in a new tab and go to it",
}
