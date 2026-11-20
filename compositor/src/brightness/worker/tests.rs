use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::mpsc::{self, UnboundedSender};

use super::*;

const LEFT: OutputId = OutputId::from_raw(1);
const RIGHT: OutputId = OutputId::from_raw(2);

fn write(output: OutputId, raw: u32) -> Job {
    Job::Write {
        output,
        device: Device::Ddc { bus: 3, max: 100 },
        raw,
    }
}

fn read(output: OutputId, sequence: u64) -> Job {
    Job::Read {
        output,
        sequence,
        device: Device::Ddc { bus: 3, max: 100 },
    }
}

/// Raw values in the order they reached the device.
type Written = Arc<Mutex<Vec<(OutputId, u32)>>>;

/// A device that records writes, answers reads with 42, and the first time
/// it writes, has more requests arrive while that write is in flight.
struct FakeMonitor {
    written: Written,
    in_flight: Arc<Mutex<(usize, usize)>>,
    arriving_meanwhile: Option<(UnboundedSender<Job>, Vec<Job>)>,
}

impl Hardware for FakeMonitor {
    async fn run(&mut self, job: Job) -> Option<Outcome> {
        {
            let mut in_flight = self.in_flight.lock();
            in_flight.0 += 1;
            in_flight.1 = in_flight.1.max(in_flight.0);
        }
        if let Some((requests, jobs)) = self.arriving_meanwhile.take() {
            for job in jobs {
                let _ = requests.send(job);
            }
        }
        tokio::task::yield_now().await;
        self.in_flight.lock().0 -= 1;
        match job {
            Job::Write { output, raw, .. } => {
                self.written.lock().push((output, raw));
                None
            }
            Job::Read {
                output, sequence, ..
            } => Some(Outcome::Level {
                output,
                sequence,
                raw: 42,
            }),
            Job::Probe(_) => None,
        }
    }
}

struct Run {
    written: Vec<(OutputId, u32)>,
    most_in_flight: usize,
    outcomes: Vec<(OutputId, u64, u32)>,
}

fn run_worker(first: Vec<Job>, meanwhile: Vec<Job>) -> std::io::Result<Run> {
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let (requests, receiver) = mpsc::unbounded_channel();
    for job in first {
        let _ = requests.send(job);
    }
    let written = Written::default();
    let in_flight = Arc::new(Mutex::new((0, 0)));
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let monitor = FakeMonitor {
        written: written.clone(),
        in_flight: in_flight.clone(),
        arriving_meanwhile: Some((requests, meanwhile)),
    };
    let reported = outcomes.clone();
    runtime.block_on(work(receiver, monitor, move |outcome| {
        if let Outcome::Level {
            output,
            sequence,
            raw,
        } = outcome
        {
            reported.lock().push((output, sequence, raw));
        }
    }));
    let run = Run {
        written: written.lock().clone(),
        most_in_flight: in_flight.lock().1,
        outcomes: outcomes.lock().clone(),
    };
    Ok(run)
}

#[test]
fn a_write_waiting_for_its_turn_is_replaced_by_a_newer_one() {
    let mut queue = JobQueue::default();
    queue.push(write(LEFT, 10));
    queue.push(write(RIGHT, 5));
    queue.push(write(LEFT, 20));
    queue.push(write(LEFT, 30));

    let order: Vec<_> = std::iter::from_fn(|| queue.next())
        .filter_map(|job| match job {
            Job::Write { output, raw, .. } => Some((output, raw)),
            _ => None,
        })
        .collect();
    assert_eq!(order, [(LEFT, 30), (RIGHT, 5)]);
}

#[test]
fn reads_and_probes_are_never_merged() {
    let mut queue = JobQueue::default();
    queue.push(read(LEFT, 1));
    queue.push(read(LEFT, 1));
    assert!(queue.next().is_some());
    assert!(queue.next().is_some());
    assert!(queue.next().is_none());
}

#[test]
fn writes_that_arrive_during_a_write_collapse_to_the_newest() -> std::io::Result<()> {
    let run = run_worker(
        vec![write(LEFT, 10)],
        vec![write(LEFT, 20), write(RIGHT, 5), write(LEFT, 30)],
    )?;
    assert_eq!(run.written, [(LEFT, 10), (LEFT, 30), (RIGHT, 5)]);
    assert_eq!(run.most_in_flight, 1);
    Ok(())
}

#[test]
fn a_burst_queued_before_the_worker_runs_costs_one_write() -> std::io::Result<()> {
    let run = run_worker(
        vec![write(LEFT, 10), write(LEFT, 20), write(LEFT, 30)],
        Vec::new(),
    )?;
    assert_eq!(run.written, [(LEFT, 30)]);
    Ok(())
}

#[test]
fn a_read_reports_its_value_with_the_sequence_it_was_asked_with() -> std::io::Result<()> {
    let run = run_worker(vec![read(RIGHT, 7)], Vec::new())?;
    assert_eq!(run.outcomes, [(RIGHT, 7, 42)]);
    assert!(run.written.is_empty());
    Ok(())
}
