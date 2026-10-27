//! The single backend worker (SAFETY.md S3 step 1). One thread per open
//! warehouse: it confines itself to the root with Landlock (S1), opens the
//! warehouse, and then runs every request in order. Nothing else in the
//! process ever holds the `Warehouse`.

use crate::confine;
use crate::warehouse::{OpenError, Warehouse};
use std::os::fd::AsFd;
use std::path::Path;
use std::sync::mpsc;
use std::thread::JoinHandle;

type Job = Box<dyn FnOnce(&mut Warehouse) + Send>;

pub struct Worker {
    tx: Option<mpsc::Sender<Job>>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn open(path: &Path, sample: bool) -> Result<Worker, OpenError> {
        // One handle for everything: policy, the Landlock rule, and the warehouse.
        let (canonical, root) = Warehouse::bind(path)?;
        let (tx, rx) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), OpenError>>();
        let thread = std::thread::Builder::new()
            .name("pallet-warehouse".into())
            .spawn(move || {
                if let Err(message) = confine::confine_current_thread(root.as_fd()) {
                    let _ = ready_tx.send(Err(OpenError {
                        code: "NO_CONFINEMENT",
                        message,
                    }));
                    return;
                }
                let mut w = match Warehouse::open_at(canonical, root, sample) {
                    Ok(w) => w,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                for job in rx {
                    job(&mut w);
                }
            })
            .map_err(|e| OpenError {
                code: "IO",
                message: e.to_string(),
            })?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Worker {
                tx: Some(tx),
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err(OpenError {
                    code: "IO",
                    message: "The warehouse worker stopped while opening.".into(),
                })
            }
        }
    }

    /// Runs `f` on the confined worker thread and waits for its result.
    pub fn run<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Warehouse) -> R + Send + 'static,
    ) -> Result<R, String> {
        let (rtx, rrx) = mpsc::channel();
        let job: Job = Box::new(move |w| {
            let _ = rtx.send(f(w));
        });
        self.tx
            .as_ref()
            .ok_or("worker closed")?
            .send(job)
            .map_err(|_| "The warehouse worker stopped.".to_string())?;
        rrx.recv()
            .map_err(|_| "The warehouse worker stopped while handling a request.".to_string())
    }
}

impl Drop for Worker {
    /// Closes the warehouse and waits, so its lock is released before another opens.
    fn drop(&mut self) {
        self.tx.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
