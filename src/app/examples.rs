//! Documentation examples computed again, each page in a Macaulay2 of its own,
//! so their values are drawn from their structure as the session's are.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use crate::backend::{M2Backend, M2Response};
use crate::ui::render::markup::ArchivedExample;

/// One page's examples to compute, with the package they document.
pub struct ExamplePage {
    pub page: u64,
    pub package: Option<String>,
    pub examples: Vec<ArchivedExample>,
}

/// What one example answered.
pub struct ComputedExample {
    pub page: u64,
    pub number: u32,
    pub response: M2Response,
}

/// The thread computing examples, page by page in the order they were asked for.
pub struct ExampleRunner {
    pages: Sender<(ExamplePage, Duration)>,
    computed: Receiver<ComputedExample>,
}

impl ExampleRunner {
    pub fn new() -> Self {
        let (pages, requested) = mpsc::channel();
        let (answered, computed) = mpsc::channel();
        thread::spawn(move || compute(&requested, &answered));
        Self { pages, computed }
    }

    /// Compute `page`'s examples, allowing each `timeout` to answer.
    pub fn run(&self, page: ExamplePage, timeout: Duration) {
        let _ = self.pages.send((page, timeout));
    }

    /// Every example answered since last asked.
    pub fn answered(&self) -> Vec<ComputedExample> {
        self.computed.try_iter().collect()
    }
}

/// Run each page's examples in order in a fresh Macaulay2, so no page sees
/// another's state. A page stops at the first example that outlasts its
/// timeout, or that fails where its documentation succeeded or the other way
/// round: what follows was recorded after a different state, so it keeps its
/// recorded answer.
fn compute(requested: &Receiver<(ExamplePage, Duration)>, answered: &Sender<ComputedExample>) {
    while let Ok((page, timeout)) = requested.recv() {
        let Ok(mut backend) = M2Backend::spawn_for_examples(page.package.as_deref()) else {
            continue;
        };
        for example in page.examples {
            let Ok(response) = backend.evaluate_within(&example.source, timeout) else {
                break;
            };
            if response.is_error != example.failed {
                break;
            }
            let computed = ComputedExample {
                page: page.page,
                number: example.number,
                response,
            };
            if answered.send(computed).is_err() {
                return;
            }
        }
    }
}

