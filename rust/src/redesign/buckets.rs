use memmap2::Mmap;
use rayon::prelude::*;
use std::ops::Range;
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

/// Each worker keeps its own scatter buffers across dynamically scheduled
/// chunks. Workers claim the largest remaining chunk first, so the last
/// chunks in flight are small and the parallel tail stays short.
pub fn consume_chunks<B: Send>(
    chunks: &[Mmap],
    range: Range<usize>,
    init: impl Fn() -> B + Sync,
    consume: impl Fn(&mut B, usize) + Sync,
) -> Vec<B> {
    // Whole mebibytes rank the chunks, so the many full-size chunks keep
    // their file order and arrive from a cold disk in the order they are read.
    let mut order: Vec<usize> = range.collect();
    order.sort_by_key(|&i| std::cmp::Reverse(chunks[i].len() >> 20));
    let next = AtomicUsize::new(0);
    (0..order.len().min(rayon::current_num_threads()))
        .into_par_iter()
        .map(|_| {
            let mut batch = init();
            while let Some(&i) = order.get(next.fetch_add(1, Ordering::Relaxed)) {
                consume(&mut batch, i);
            }
            batch
        })
        .collect()
}
