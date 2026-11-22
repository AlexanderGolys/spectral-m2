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
    stopped: Receiver<u64>,
}

impl ExampleRunner {
    pub fn new() -> Self {
        let (pages, requested) = mpsc::channel();
        let (answered, computed) = mpsc::channel();
        let (stopped_short, stopped) = mpsc::channel();
        thread::spawn(move || compute(&requested, &answered, &stopped_short));
        Self {
            pages,
            computed,
            stopped,
        }
    }

    /// Compute `page`'s examples, allowing each `timeout` to answer.
    pub fn run(&self, page: ExamplePage, timeout: Duration) {
        let _ = self.pages.send((page, timeout));
    }

    /// Every example answered since last asked.
    pub fn answered(&self) -> Vec<ComputedExample> {
        self.computed.try_iter().collect()
    }

    /// Every page whose examples stopped short since last asked.
    pub fn stopped(&self) -> Vec<u64> {
        self.stopped.try_iter().collect()
    }
}

/// Run each page's examples in order in a fresh Macaulay2, so no page sees
/// another's state. A page stops at the first example that outlasts its
/// timeout, or that fails where its documentation succeeded or the other way
/// round: what follows was recorded after a different state, so it keeps its
/// recorded answer. A page stopped short is reported, so it can be asked for again.
fn compute(
    requested: &Receiver<(ExamplePage, Duration)>,
    answered: &Sender<ComputedExample>,
    stopped: &Sender<u64>,
) {
    while let Ok((page, timeout)) = requested.recv() {
        let id = page.page;
        if !compute_page(page, timeout, answered) && stopped.send(id).is_err() {
            return;
        }
    }
}

/// Send what each of `page`'s examples answers; whether every one did.
fn compute_page(page: ExamplePage, timeout: Duration, answered: &Sender<ComputedExample>) -> bool {
    let Ok(mut backend) = M2Backend::spawn_for_examples(page.package.as_deref()) else {
        return false;
    };
    page.examples.into_iter().all(|example| {
        backend
            .evaluate_within(&example.source, timeout)
            .ok()
            .filter(|response| response.is_error == example.failed)
            .is_some_and(|response| {
                answered
                    .send(ComputedExample {
                        page: page.page,
                        number: example.number,
                        response,
                    })
                    .is_ok()
            })
    })
}
