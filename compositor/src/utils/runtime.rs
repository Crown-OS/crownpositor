//! The bridge between the compositor thread and async work.
//!
//! The rendering thread runs calloop and must never block — a frame missed
//! because something `await`ed inline is a visible stutter. So async work
//! (IPC, config watching, portals, anything network- or disk-shaped) runs on
//! a small Tokio pool, and its *results* come back as closures over `&mut
//! State`, delivered through a calloop channel that wakes the loop like any
//! other event source.
//!
//! ```text
//!  compositor thread                     tokio workers
//!  ──────────────────                    ─────────────
//!  tasks.run(future, on_done) ───────▶ future.await
//!        ▲                                   │
//!        │  calloop channel (wakes loop)     │
//!  on_done(&mut State, value) ◀────── send(Box<closure>)
//! ```
//!
//! Nothing here is graphics-specific; the render loop only interacts with it
//! by *not being blocked*.

use std::{future::Future, sync::Arc};

use anyhow::Context as _;
use calloop::{LoopHandle, channel};

use crate::state::State;

/// A completion that runs on the compositor thread with full state access.
type StateTask = Box<dyn FnOnce(&mut State) + Send>;

/// Handle for spawning async work. Cheap to clone; hand it to anything that
/// needs to do slow work off the compositor thread.
#[derive(Clone)]
pub struct TaskSender {
    runtime: Arc<tokio::runtime::Runtime>,
    completions: channel::Sender<StateTask>,
}

impl TaskSender {
    /// Builds the Tokio pool and plugs its completion channel into the event
    /// loop. Called once, from state construction.
    pub fn init(handle: &LoopHandle<'static, State>) -> anyhow::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            // Two is plenty: these workers host waiting, not computation.
            .worker_threads(2)
            .thread_name("crown-async")
            .enable_all()
            .build()
            .with_context(|| "failed to build the tokio runtime")?;

        let (sender, receiver) = channel::channel::<StateTask>();
        handle
            .insert_source(receiver, |event, _, state| {
                if let channel::Event::Msg(task) = event {
                    task(state);
                }
            })
            .map_err(|err| {
                anyhow::anyhow!("failed to insert the async completion source: {err}")
            })?;

        Ok(Self {
            runtime: Arc::new(runtime),
            completions: sender,
        })
    }

    /// Runs `future` on the async pool; when it resolves, `on_done` runs on
    /// the compositor thread with the result and `&mut State`.
    ///
    /// The completion is delivered through the event loop, so it obeys the
    /// same ordering as every other event — it can queue redraws, touch the
    /// shell, anything.
    pub fn run<F, T, C>(&self, future: F, on_done: C)
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
        C: FnOnce(&mut State, T) + Send + 'static,
    {
        let completions = self.completions.clone();
        self.runtime.spawn(async move {
            let value = future.await;
            // A send failure means the compositor is shutting down; the
            // result has nowhere meaningful to go.
            if completions
                .send(Box::new(move |state| on_done(state, value)))
                .is_err()
            {
                tracing::debug!("async completion dropped: event loop is gone");
            }
        });
    }

    /// Fire-and-forget async work that never needs the compositor state.
    pub fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.runtime.spawn(future);
    }
}
