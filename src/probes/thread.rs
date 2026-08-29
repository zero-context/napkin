//! Round trip between two threads over a rendezvous channel.
//!
//! Not labelled "context switch latency": a round trip is two switches plus the
//! channel's own bookkeeping, and on a multi-core machine the scheduler may
//! spin rather than switch at all. It is still the number that matters when you
//! are costing a hand-off between threads.

use super::{bench, time_ns, Probe};
use std::sync::mpsc::sync_channel;

const OPS: u64 = 20_000;
const REPS: usize = 9;

pub fn run() -> Probe {
    // Zero-capacity channels are rendezvous points: each send blocks until the
    // far side receives, which is what forces the hand-off.
    let (to_worker, worker_rx) = sync_channel::<u8>(0);
    let (to_main, main_rx) = sync_channel::<u8>(0);

    let worker = std::thread::spawn(move || {
        while let Ok(v) = worker_rx.recv() {
            if to_main.send(v).is_err() {
                break;
            }
        }
    });

    let summary = bench(REPS, OPS, || {
        time_ns(|| {
            for _ in 0..OPS {
                to_worker.send(1).expect("worker thread died mid-probe");
                main_rx.recv().expect("worker thread died mid-probe");
            }
        })
    });

    drop(to_worker);
    let _ = worker.join();

    Probe {
        name: "thread_pingpong_rtt",
        unit: "ns",
        what: WHAT,
        summary: Some(summary),
        points: Vec::new(),
        note: None,
    }
}

const WHAT: &str =
    "nanoseconds for one send-and-receive round trip between two threads over a zero-capacity \
     rendezvous channel. includes two hand-offs plus channel overhead, not a bare context switch.";
