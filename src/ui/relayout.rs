//! Layouts computed beside the drawing thread from copies of what they lay out, so
//! the screen keeps its previous picture until the new one is ready to swap in.

use std::num::NonZero;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::thread;

type Job = Box<dyn FnOnce() + Send>;

static WORKERS: LazyLock<Mutex<Sender<Job>>> = LazyLock::new(start_workers);
/// Jobs sent and not yet finished or abandoned.
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
/// Results delivered so far; a change tells the drawing thread to look for them.
static DELIVERED: AtomicU64 = AtomicU64::new(0);

fn start_workers() -> Mutex<Sender<Job>> {
    let (sender, receiver) = channel::<Job>();
    let receiver = Arc::new(Mutex::new(receiver));
    let count = thread::available_parallelism().map_or(1, NonZero::get);
    for _ in 0..count {
        let receiver = receiver.clone();
        thread::spawn(move || loop {
            let job = receiver
                .lock()
                .expect("a layout worker never panics holding the queue")
                .recv();
            match job {
                Ok(job) => job(),
                Err(_) => break,
            }
        });
    }
    Mutex::new(sender)
}

/// A result being computed on a worker. Dropping it abandons the work, which is
/// skipped when no worker has started it yet.
///
/// ```ignore
/// let pending = Pending::spawn(|| 6 * 7);
/// // … later, on the drawing thread:
/// if let Some(answer) = pending.take() { assert_eq!(answer, 42) }
/// ```
#[derive(Debug)]
pub struct Pending<T> {
    slot: Arc<Mutex<Option<T>>>,
}

impl<T: Send + 'static> Pending<T> {
    pub fn spawn(job: impl FnOnce() -> T + Send + 'static) -> Self {
        let slot = Arc::new(Mutex::new(None));
        let target: Weak<Mutex<Option<T>>> = Arc::downgrade(&slot);
        IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
        let job: Job = Box::new(move || {
            if target.strong_count() > 0 {
                let result = job();
                if let Some(slot) = target.upgrade() {
                    *slot.lock().expect("a layout result is never set mid-panic") = Some(result);
                    DELIVERED.fetch_add(1, Ordering::SeqCst);
                }
            }
            IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        });
        WORKERS
            .lock()
            .expect("the layout queue is never held across a panic")
            .send(job)
            .expect("layout workers live as long as the program");
        Self { slot }
    }

    /// The result, once; `None` while it is still being computed.
    pub fn take(&self) -> Option<T> {
        self.slot
            .lock()
            .expect("a layout result is never set mid-panic")
            .take()
    }
}

/// Whether any worker still has a layout to deliver.
pub fn busy() -> bool {
    IN_FLIGHT.load(Ordering::SeqCst) > 0
}

/// How many results have been delivered; compare to notice new ones.
pub fn delivered() -> u64 {
    DELIVERED.load(Ordering::SeqCst)
}

