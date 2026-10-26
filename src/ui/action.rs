//! Operations emitted by keymaps and other UI event sources.

use serde::{Deserialize, Serialize};
use these_macros_should_be_illegal::discriminated_str;

/// A small by-value operation routed through the focused UI hierarchy.
#[discriminated_str(description)]
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Action {
    Quit = "quit spectral-m2",
    CancelExecution = "cancel execution",
    Submit = "evaluate the input",
    ScrollFeedUp = "scroll the feed up",
    ScrollFeedDown = "scroll the feed down",
    OpenCommands = "open commands",
    OpenSettings = "open settings",
    OpenKeymaps = "open keymaps",
    ClearFeed = "clear the feed",
    OpenCode = "open code",
    OpenValues = "open returned values",
    OpenStdOut = "open printed output",
    OpenOutput = "open all output",
    OpenSyntaxTree = "open syntax tree",
    SaveSession = "save the session's code to a file",
    LoadCode = "run the code in a file",
    ResumeSession = "run the previous session's code",
    Restart = "restart Macaulay2",
    ClosePane = "close the view",
    ToggleHighlightDebug = "toggle highlight groups",
    CopySelection = "copy the selection",
}
