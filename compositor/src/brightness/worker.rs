//! The one task that talks to brightness hardware.
//!
//! Jobs run one at a time, in the order they came, so DDC/CI calls never
//! overlap on a bus. While one runs, newer writes for the same output replace
//! the one still waiting: a slider dragged across its whole range costs one
//! write per round trip, not one per sample.

use std::{collections::VecDeque, future::Future};

use tokio::sync::mpsc::UnboundedReceiver;

use super::{
    device::{Device, Found, ProbeTarget},
    error::DeviceError,
};
use crate::utils::OutputId;

#[derive(Debug, Clone)]
pub enum Job {
    Probe(ProbeTarget),
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used once the protocol and the keys land")
    )]
    Write {
        output: OutputId,
        device: Device,
        raw: u32,
    },
    /// `sequence` is the output's change count when the read was asked for,
    /// so a value read before a newer level was set is dropped.
    Read {
        output: OutputId,
        sequence: u64,
        device: Device,
    },
}

#[derive(Debug)]
pub enum Outcome {
    Probed {
        output: OutputId,
        found: Found,
    },
    Level {
        output: OutputId,
        sequence: u64,
        raw: u32,
    },
    Failed {
        output: OutputId,
        during: Operation,
        error: DeviceError,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Write,
    Read,
}

/// Something that runs jobs: the real devices, or a fake in tests. A write
/// that succeeds reports nothing; the level it was asked for is the level.
pub trait Hardware: Send + 'static {
    fn run(&mut self, job: Job) -> impl Future<Output = Option<Outcome>> + Send;
}

/// The jobs waiting their turn, at most one write per output.
#[derive(Debug, Default)]
pub struct JobQueue {
    waiting: VecDeque<Job>,
}

impl JobQueue {
    /// Queues `job`. A write takes the place of one still waiting for the
    /// same output.
    pub fn push(&mut self, job: Job) {
        if let Job::Write { output, .. } = &job
            && let Some(waiting) = self.waiting.iter_mut().find(
                |waiting| matches!(waiting, Job::Write { output: other, .. } if other == output),
            )
        {
            *waiting = job;
            return;
        }
        self.waiting.push_back(job);
    }

    pub fn next(&mut self) -> Option<Job> {
        self.waiting.pop_front()
    }
}

/// Runs jobs until every sender is gone, handing each outcome to `report`.
pub async fn work(
    mut requests: UnboundedReceiver<Job>,
    mut hardware: impl Hardware,
    report: impl Fn(Outcome) + Send,
) {
    let mut queue = JobQueue::default();
    loop {
        while let Ok(job) = requests.try_recv() {
            queue.push(job);
        }
        if let Some(job) = queue.next() {
            if let Some(outcome) = hardware.run(job).await {
                report(outcome);
            }
            continue;
        }
        match requests.recv().await {
            Some(job) => queue.push(job),
            None => return,
        }
    }
}

#[cfg(test)]
mod tests;
