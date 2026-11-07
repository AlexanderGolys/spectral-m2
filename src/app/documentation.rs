//! Documentation fetched in a separate interpreter without changing the session.

use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use crate::backend::{M2Backend, M2Response};

/// One requested page and the identity of the view that requested it.
pub struct DocumentationPage {
    pub request: u64,
    pub response: Result<M2Response, String>,
}

/// A lazily started interpreter used only to read documentation.
pub struct DocumentationBrowser {
    requests: Sender<(u64, String)>,
    pages: Receiver<DocumentationPage>,
    next_request: u64,
}

impl DocumentationBrowser {
    pub fn new() -> Self {
        let (requests, incoming) = mpsc::channel::<(u64, String)>();
        let (outgoing, pages) = mpsc::channel();
        thread::spawn(move || {
            let mut backend = None;
            for (request, target) in incoming {
                let response = (|| {
                    if backend.is_none() {
                        backend = Some(
                            M2Backend::spawn_for_examples(None)
                                .map_err(|error| error.to_string())?,
                        );
                    }
                    backend
                        .as_mut()
                        .expect("documentation interpreter started")
                        .evaluate_cancellable(&format!("help {target}"), &AtomicBool::new(false))
                        .map_err(|error| error.to_string())
                })();
                if response.is_err() {
                    backend = None;
                }
                if outgoing
                    .send(DocumentationPage { request, response })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            requests,
            pages,
            next_request: 0,
        }
    }

    /// Request a page, returning its identity for discarding superseded results.
    pub fn open(&mut self, target: String) -> u64 {
        self.next_request += 1;
        let _ = self.requests.send((self.next_request, target));
        self.next_request
    }

    pub fn answered(&self) -> impl Iterator<Item = DocumentationPage> + '_ {
        self.pages.try_iter()
    }
}
