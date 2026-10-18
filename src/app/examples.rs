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

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    /// Every example of `page` the runner answers, waiting until it falls silent.
    fn answers(page: ExamplePage, timeout: Duration) -> Vec<ComputedExample> {
        let runner = ExampleRunner::new();
        runner.run(page, timeout);
        let mut answered = Vec::new();
        let mut quiet_since = Instant::now();
        while quiet_since.elapsed() < timeout * 4 {
            let more = runner.answered();
            if !more.is_empty() {
                quiet_since = Instant::now();
            }
            answered.extend(more);
            thread::sleep(Duration::from_millis(50));
        }
        answered
    }

    fn example(number: u32, source: &str, failed: bool) -> ArchivedExample {
        ArchivedExample {
            number,
            source: source.to_string(),
            failed,
        }
    }

    #[test]
    fn examples_run_in_order_under_the_numbers_they_were_recorded_with() {
        let page = ExamplePage {
            page: 1,
            package: Some("Graphs".to_string()),
            examples: vec![
                example(1, "G = graph {{1,2},{2,3}}", false),
                example(2, "#edges o1", false),
                example(3, "error \"recorded as failing\"", true),
            ],
        };
        let answered = answers(page, Duration::from_secs(10));
        let numbers: Vec<_> = answered.iter().map(|example| example.number).collect();
        assert_eq!(numbers, [1, 2, 3]);
        assert_eq!(
            answered[1].response.rendered_value().as_deref(),
            Some("2"),
            "`o1` is the first example's value, as when it was recorded"
        );
    }

    #[test]
    fn a_page_stops_where_an_example_outlasts_its_timeout_or_answers_differently() {
        let slow = ExamplePage {
            page: 2,
            package: None,
            examples: vec![
                example(1, "1+1", false),
                example(2, "sleep 30", false),
                example(3, "2+2", false),
            ],
        };
        let numbers = |answered: Vec<ComputedExample>| {
            answered
                .iter()
                .map(|example| example.number)
                .collect::<Vec<_>>()
        };
        assert_eq!(numbers(answers(slow, Duration::from_secs(1))), [1]);

        let diverging = ExamplePage {
            page: 3,
            package: None,
            examples: vec![
                example(1, "x = 1", false),
                example(2, "undefinedFunction 3", false),
                example(3, "x", false),
            ],
        };
        assert_eq!(numbers(answers(diverging, Duration::from_secs(5))), [1]);
    }
}
