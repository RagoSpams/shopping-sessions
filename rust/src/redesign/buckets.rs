use rayon::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};

pub const IDS: usize = 128;
pub const USERS: usize = 128;
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
