//! Runs DND changes off the UI thread.
//!
//! Applying a change restarts `WpnUserService_*` and then waits for the new
//! state to be published, which takes on the order of a second. Doing that on
//! the message-pump thread would freeze the tray icon and the settings window,
//! so all of it happens here and results are reported back asynchronously.

use super::{DndController, DndOutcome};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::JoinHandle;

enum Request {
    Engage,
    Release,
    Shutdown,
}

/// A completed DND change, for the UI to surface.
#[derive(Clone, Debug)]
pub struct DndReport {
    pub engaging: bool,
    pub outcome: DndOutcome,
}

/// Handle to the background DND thread.
pub struct DndWorker {
    tx: Sender<Request>,
    rx: Receiver<DndReport>,
    thread: Option<JoinHandle<()>>,
}

impl DndWorker {
    /// `on_report` is called on the worker thread after every change; use it to
    /// nudge the UI thread (e.g. `PostMessage`) so it drains [`Self::reports`].
    pub fn spawn(on_report: impl Fn() + Send + 'static) -> Self {
        let (tx, req_rx) = channel::<Request>();
        let (rep_tx, rx) = channel::<DndReport>();
        let thread = std::thread::spawn(move || {
            let mut ctl = DndController::new();
            while let Ok(req) = req_rx.recv() {
                let report = match req {
                    Request::Engage => DndReport {
                        engaging: true,
                        outcome: ctl.engage(),
                    },
                    Request::Release => DndReport {
                        engaging: false,
                        outcome: ctl.release(),
                    },
                    Request::Shutdown => break,
                };
                let _ = rep_tx.send(report);
                on_report();
            }

            let outcome = ctl.release();
            if outcome != DndOutcome::AlreadyCorrect {
                crate::audit::log_dnd(false, &format!("{outcome:?} (on exit)"));
            }
        });
        DndWorker {
            tx,
            rx,
            thread: Some(thread),
        }
    }

    pub fn engage(&self) {
        let _ = self.tx.send(Request::Engage);
    }

    pub fn release(&self) {
        let _ = self.tx.send(Request::Release);
    }

    /// Non-blocking drain of finished changes.
    pub fn reports(&self) -> impl Iterator<Item = DndReport> + '_ {
        self.rx.try_iter()
    }
}

impl Drop for DndWorker {
    fn drop(&mut self) {
        let _ = self.tx.send(Request::Shutdown);

        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
