use ahash::RandomState;
use hashbrown::HashTable;
use hashbrown::hash_table::Entry;
use memchr::{memchr, memchr_iter};
use memmap2::{Mmap, MmapOptions};
use rayon::prelude::*;
use std::fs::{self, File};
use std::io;
use std::path::Path;

pub const SHARDS: usize = 256;
const CHUNK_BYTES: usize = 4 * 1024 * 1024;

pub fn csv<const N: usize>(line: &[u8]) -> Option<[&[u8]; N]> {
    let mut commas = memchr_iter(b',', line);
    let mut start = 0;
    let mut result = [&[][..]; N];
    for (index, field) in result.iter_mut().enumerate() {
        let end = if index + 1 == N {
            if commas.next().is_some() {
                return None;
            }
            line.len()
        } else {
            commas.next()?
        };
        let bytes = &line[start..end];
        let bytes = if let Some(quoted) = bytes.strip_prefix(b"\"") {
            quoted.strip_suffix(b"\"")?
        } else {
            bytes
        };
        if memchr(b'"', bytes).is_some() {
            return None;
        }
        *field = bytes;
        start = end + 1;
    }
    Some(result)
}

pub fn lines(data: &[u8]) -> impl Iterator<Item = (u32, &[u8])> {
    let mut start = 0;
    memchr_iter(b'\n', data)
        .chain(std::iter::once(data.len()))
        .filter_map(move |end| {
            let offset = start;
            start = end + 1;
            (offset < end).then(|| (u32::try_from(offset).unwrap(), &data[offset..end]))
        })
}

pub fn after_header(data: &[u8]) -> &[u8] {
    memchr(b'\n', data).map_or(&[], |end| &data[end + 1..])
}

pub fn read_parts(directory: &Path) -> io::Result<Vec<Mmap>> {
    let mut paths = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<io::Result<Vec<_>>>()?;
    paths.retain(|path| path.as_os_str().as_encoded_bytes().ends_with(b".csv"));
    paths.sort_unstable();

    let mut chunks = Vec::new();
    for path in paths {
        let file = File::open(path)?;
        if file.metadata()?.len() == 0 {
            continue;
        }
        // The input is immutable for the entire run. Each chunk owns a separate
        // read-only mapping so it can be unmapped as soon as its rows are parsed.
        let file_bytes = unsafe { Mmap::map(&file)? };
        let mut start = file_bytes.len() - after_header(&file_bytes).len();
        while start < file_bytes.len() {
            let target = (start + CHUNK_BYTES).min(file_bytes.len());
            let end = if target == file_bytes.len() {
                target
            } else {
                memchr(b'\n', &file_bytes[target - 1..])
                    .map_or(file_bytes.len(), |next| target + next)
            };
            let chunk = unsafe {
                MmapOptions::new()
                    .offset(start as u64)
                    .len(end - start)
                    .map(&file)?
            };
            chunks.push(chunk);
            start = end;
        }
    }
    Ok(chunks)
}

#[derive(Clone, Copy)]
pub struct Location {
    chunk: u32,
    offset: u32,
}

impl Location {
    pub fn new(chunk: usize, offset: u32) -> Self {
        Self {
            chunk: u32::try_from(chunk).unwrap(),
            offset,
        }
    }

    fn row(self, chunks: &[Mmap]) -> &[u8] {
        let tail = &chunks[self.chunk as usize][self.offset as usize..];
        &tail[..memchr(b'\n', tail).unwrap_or(tail.len())]
    }

    pub fn id(self, chunks: &[Mmap]) -> &[u8] {
        let tail = &chunks[self.chunk as usize][self.offset as usize..];
        // Called only for records that passed CSV parsing, so the first
        // comma is on this line and any opening quote has a closing quote.
        let field = &tail[..memchr(b',', tail).unwrap()];
        if field[0] == b'"' {
            &field[1..field.len() - 1]
        } else {
            field
        }
    }
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
