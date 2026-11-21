use rayon::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};

pub const IDS: usize = 1024;
pub const USERS: usize = 256;
pub type Runs<T> = Vec<Vec<T>>;
pub type Partitions<T> = Vec<Runs<T>>;

pub fn empty<T>(count: usize) -> Vec<Vec<T>> {
    (0..count).map(|_| Vec::new()).collect()
}

#[inline]
pub fn bucket(hash: u64, count: usize) -> usize {
    (hash >> 32) as usize & (count - 1)
}

/// Each worker keeps its own scatter buffers across dynamically scheduled chunks.
pub fn consume_chunks<B: Send>(
    start: usize,
    count: usize,
    init: impl Fn() -> B + Sync,
    consume: impl Fn(&mut B, usize) + Sync,
) -> Vec<B> {
    let next = AtomicUsize::new(start);
    (0..count.min(rayon::current_num_threads()))
        .into_par_iter()
        .map(|_| {
            let mut batch = init();
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= start + count {
                    break;
                }
                consume(&mut batch, i);
            }
            batch
        })
        .collect()
}

pub fn transpose<T>(batches: Vec<Vec<Vec<T>>>, count: usize) -> Partitions<T> {
    let mut out: Vec<_> = (0..count)
        .map(|_| Vec::with_capacity(batches.len()))
        .collect();
    for batch in batches {
        for (dst, run) in out.iter_mut().zip(batch) {
            if !run.is_empty() {
                dst.push(run);
            }
        }
    }
    out
}

pub fn flatten<T>(runs: Runs<T>) -> Vec<T> {
    let mut out = Vec::with_capacity(runs.iter().map(Vec::len).sum());
    for run in runs {
        out.extend(run);
    }
    out
}
