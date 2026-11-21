use shopping_sessions::csv::{Location, csv, lines, read_chunks};
use ahash::RandomState;
use hashbrown::HashTable;
use hashbrown::hash_table::Entry;
use memmap2::Mmap;
use rayon::prelude::*;
use std::io;
use std::path::Path;

pub const SHARDS: usize = 256;
pub fn read_parts(directory: &Path) -> io::Result<Vec<Mmap>> {
    read_chunks(directory, 4 * 1024 * 1024)
}

#[derive(Clone, Copy)]
struct IndexedRow {
    hash: u64,
    location: Location,
}

pub fn shard(hash: u64) -> usize {
    (hash >> 32) as usize % SHARDS
}

pub fn batch_size(chunks: usize) -> usize {
    chunks.div_ceil(rayon::current_num_threads() * 4).max(1)
}

pub fn empty_shards<T>() -> Vec<Vec<T>> {
    (0..SHARDS).map(|_| Vec::new()).collect()
}

pub fn transpose<T>(producers: Vec<Vec<T>>) -> Vec<Vec<T>> {
    let mut shards: Vec<_> = (0..SHARDS)
        .map(|_| Vec::with_capacity(producers.len()))
        .collect();
    for producer in producers {
        for (shard, rows) in shards.iter_mut().zip(producer) {
            shard.push(rows);
        }
    }
    shards
}

// Only well-formed rows with an ID enter the index. Store their positions
// rather than their strings or full rows. Hashes choose the shard and probe
// the table, but all matching keys are compared byte for byte.
pub fn duplicate_rows<const N: usize>(chunks: &[Mmap], earliest_ingest: bool) -> Vec<Vec<u32>> {
    let hash = RandomState::new();
    let size = batch_size(chunks.len());
    let producers = chunks
        .par_chunks(size)
        .enumerate()
        .map(|(batch, chunks)| {
            let mut buckets = empty_shards();
            for (index, chunk) in chunks.iter().enumerate() {
                let chunk_index = batch * size + index;
                for (offset, line) in lines(chunk) {
                    if let Some(fields) = csv::<N>(line)
                        && !fields[0].is_empty()
                    {
                        let hash = hash.hash_one(fields[0]);
                        buckets[shard(hash)].push(IndexedRow {
                            hash,
                            location: Location::new(chunk_index, offset),
                        });
                    }
                }
            }
            buckets
        })
        .collect();

    let losers: Vec<Vec<Location>> = transpose(producers)
        .into_par_iter()
        .map(|parts| {
            let mut winners: HashTable<IndexedRow> =
                HashTable::with_capacity(parts.iter().map(Vec::len).sum());
            let mut losers = Vec::new();
            for row in parts.into_iter().flatten() {
                let equal = |other: &IndexedRow| {
                    row.hash == other.hash && row.location.id(chunks) == other.location.id(chunks)
                };
                match winners.entry(row.hash, equal, |entry| entry.hash) {
                    Entry::Occupied(mut entry) => {
                        let winner = entry.get_mut();
                        // Event ingest timestamps are guaranteed to be F1, so
                        // their byte order is chronological. Click copies agree.
                        let ingest = |location: Location| csv::<N>(location.row(chunks)).unwrap()[3];
                        if earliest_ingest && ingest(row.location) < ingest(winner.location) {
                            losers.push(winner.location);
                            *winner = row;
                        } else {
                            losers.push(row.location);
                        }
                    }
                    Entry::Vacant(entry) => {
                        entry.insert(row);
                    }
                }
            }
            losers
        })
        .collect();

    let mut by_chunk = vec![Vec::new(); chunks.len()];
    for location in losers.into_iter().flatten() {
        by_chunk[location.chunk as usize].push(location.offset);
    }
    by_chunk.par_iter_mut().for_each(|rows| rows.sort_unstable());
    by_chunk
}

pub fn parse_winners<const N: usize, P, C, F>(
    chunks: Vec<C>,
    losers: Vec<Vec<u32>>,
    process: F,
) -> Vec<P>
where
    P: Default + Send,
    C: AsRef<[u8]> + Send,
    F: Fn(&mut P, Location, [&[u8]; N]) + Sync,
{
    let size = batch_size(chunks.len());
    let mut batches = Vec::new();
    let mut input = chunks.into_iter().zip(losers).enumerate().peekable();
    while input.peek().is_some() {
        batches.push(input.by_ref().take(size).collect::<Vec<_>>());
    }
    batches
        .into_par_iter()
        .map(|batch| {
            let mut output = P::default();
            for (index, (chunk, losers)) in batch {
                let mut losers = losers.into_iter().peekable();
                for (offset, line) in lines(chunk.as_ref()) {
                    if losers.next_if_eq(&offset).is_some() {
                        continue;
                    }
                    if let Some(fields) = csv::<N>(line)
                        && !fields[0].is_empty()
                    {
                        process(&mut output, Location::new(index, offset), fields);
                    }
                }
            }
            output
        })
        .collect()
}
