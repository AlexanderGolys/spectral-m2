//! Undo and redo for the input field: whole-buffer snapshots, with a run of like
//! edits made in one place undone as a single step.

/// Input text and cursor as they stood before an undoable edit.
pub struct Snapshot {
    pub text: String,
    pub cursor: usize,
}

/// What an edit did, which decides whether it extends the step before it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    DeletingBack,
    DeletingForward,
    Completing,
    Recalling,
    /// Pasting, clearing, or restoring source: always a step of its own.
    Replacing,
}

/// Steps to undo, steps undone, and the run the latest edit belongs to.
#[derive(Default)]
pub struct Undo {
    undoable: Vec<Snapshot>,
    redoable: Vec<Snapshot>,
    /// The latest edit's kind and the cursor it left, which the next edit must
    /// start from to join its step.
    run: Option<(EditKind, usize)>,
}

impl Undo {
    /// Record an edit that turned `before` into text with the cursor at
    /// `cursor`. It joins the previous step when it carries on that run.
    pub fn record(&mut self, kind: EditKind, before: Snapshot, cursor: usize) {
        let continues = kind != EditKind::Replacing && self.run == Some((kind, before.cursor));
        if !continues {
            self.undoable.push(before);
        }
        self.redoable.clear();
        self.run = Some((kind, cursor));
    }

    /// Make the next edit a step of its own.
    pub fn break_run(&mut self) {
        self.run = None;
    }

    /// The state before the latest step, with `current` kept for redoing it.
    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let previous = self.undoable.pop()?;
        self.redoable.push(current);
        self.run = None;
        Some(previous)
    }

    /// The state the latest undo left, with `current` kept for undoing again.
    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let next = self.redoable.pop()?;
        self.undoable.push(current);
        self.run = None;
        Some(next)
    }
}
