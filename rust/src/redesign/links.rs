use crate::buckets::{self, Partitions, Runs, USERS, bucket};
use crate::key::{Key, PackedKey};
use crate::records::Named;
use crate::scan::rows;
use ahash::RandomState;
use hashbrown::HashTable;
use memmap2::Mmap;
use rayon::prelude::*;
use shopping_sessions::aggregate::{Click, Event, PersonRow};
use std::ops::Range;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

struct Endpoint {
    name: PackedKey,
    hash: u64,
    slot: usize,
}

pub struct LinkBatch {
    ends: Vec<Vec<Endpoint>>,
    count: usize,
}

pub fn parse(chunks: &[Mmap], range: Range<usize>, state: &RandomState) -> Vec<LinkBatch> {
    buckets::consume_chunks(
        range.start,
        range.len(),
        || LinkBatch {
            ends: buckets::empty(USERS),
            count: 0,
        },
        |batch, i| {
            let chunk = &chunks[i];
            for (_, [left, right]) in rows::<2>(chunk) {
                if left.is_empty() || right.is_empty() {
                    continue;
                }
                for field in [left, right] {
                    let name = PackedKey::new(field, chunk, i);
                    let hash = name.hash(state, chunks);
                    batch.ends[bucket(hash, USERS)].push(Endpoint {
                        name,
                        hash,
                        slot: batch.count,
                    });
                    batch.count += 1;
                }
            }
        },
    )
}

// Encoding the owning partition in the low bits makes every component name a
// valid destination for both linked and unlinked users, without a name merge.
fn person(shard: usize, local: usize) -> u32 {
    u32::try_from(local * USERS + shard).expect("too many names in one partition")
}
fn owner(id: u32) -> usize {
    id as usize % USERS
}
fn local(id: u32) -> usize {
    id as usize / USERS
}

#[derive(Default)]
pub struct Directory {
    names: Vec<Key>,
    table: HashTable<(PackedKey, u32)>,
    roots: Vec<u32>,
}

impl Directory {
    fn intern(&mut self, name: PackedKey, hash: u64, state: &RandomState, chunks: &[Mmap]) -> u32 {
        let names = &mut self.names;
        self.table
            .entry(
                hash,
                |&(stored, _)| name.matches(stored, chunks),
                |&(stored, _)| stored.hash(state, chunks),
            )
            .or_insert_with(|| {
                let id = u32::try_from(names.len()).unwrap();
                names.push(name.name(chunks));
                (name, id)
            })
            .get()
            .1
    }

    fn route<T>(
        &mut self,
        shard: usize,
        rows: Runs<Named<T>>,
        state: &RandomState,
        chunks: &[Mmap],
    ) -> Vec<Vec<PersonRow<T>>> {
        let mut output = buckets::empty(USERS);
        for row in rows.into_iter().flatten() {
            let index = self.intern(row.user, row.hash, state, chunks) as usize;
            // Unlinked names are appended after the link-only root table.
            let root = self
                .roots
                .get(index)
                .copied()
                .unwrap_or_else(|| person(shard, index));
            output[owner(root)].push(PersonRow {
                person: local(root) as u32,
                data: row.data,
            });
        }
        output
    }
}

fn parent(parents: &[Vec<AtomicU32>], id: u32) -> &AtomicU32 {
    &parents[owner(id)][local(id)]
}
fn root(parents: &[Vec<AtomicU32>], mut id: u32) -> u32 {
    loop {
        let slot = parent(parents, id);
        let next = slot.load(Relaxed);
        if next == id {
            return id;
        }
        let grandparent = parent(parents, next).load(Relaxed);
        if next != grandparent {
            let _ = slot.compare_exchange(next, grandparent, Relaxed, Relaxed);
        }
        id = grandparent;
    }
}

pub fn build(batches: Vec<LinkBatch>, chunks: &[Mmap], state: &RandomState) -> Vec<Directory> {
    let slots: Vec<_> = (0..batches.iter().map(|b| b.count).sum())
        .map(|_| AtomicU32::new(0))
        .collect();
    let mut partitions = buckets::empty(USERS);
    let mut base = 0;
    for batch in batches {
        for (partition, endpoints) in partitions.iter_mut().zip(batch.ends) {
            partition.push((base, endpoints));
        }
        base += batch.count;
    }
    let mut dirs: Vec<_> = partitions
        .into_par_iter()
        .enumerate()
        .map(|(shard, parts)| {
            let mut dir = Directory::default();
            for (base, ends) in parts {
                for endpoint in ends {
                    let i = dir.intern(endpoint.name, endpoint.hash, state, chunks);
                    slots[base + endpoint.slot].store(person(shard, i as usize), Relaxed);
                }
            }
            dir
        })
        .collect();
    let parents: Vec<Vec<_>> = dirs
        .par_iter()
        .enumerate()
        .map(|(shard, dir)| {
            (0..dir.names.len())
                .map(|i| AtomicU32::new(person(shard, i)))
                .collect()
        })
        .collect();
    let name = |id| dirs[owner(id)].names[local(id)].bytes(chunks);
    // Every successful edge points toward a strictly smaller byte string.
    // A stale target is safe because its own path only goes to smaller names.
    slots.par_chunks_exact(2).for_each(|edge| {
        let (mut a, mut b) = (edge[0].load(Relaxed), edge[1].load(Relaxed));
        loop {
            a = root(&parents, a);
            b = root(&parents, b);
            if a == b {
                break;
            }
            if name(a) < name(b) {
                std::mem::swap(&mut a, &mut b);
            }
            if parent(&parents, a)
                .compare_exchange(a, b, Relaxed, Relaxed)
                .is_ok()
            {
                break;
            }
        }
    });
    dirs.par_iter_mut().enumerate().for_each(|(shard, dir)| {
        dir.roots = (0..dir.names.len())
            .map(|i| root(&parents, person(shard, i)))
            .collect();
    });
    dirs
}

pub struct PeopleRows {
    pub names: Vec<Vec<Key>>,
    pub events: Partitions<PersonRow<Event>>,
    pub clicks: Partitions<PersonRow<Click>>,
}

pub fn resolve(
    dirs: Vec<Directory>,
    events: Partitions<Named<Event>>,
    clicks: Partitions<Named<Click>>,
    state: &RandomState,
    chunks: &[Mmap],
) -> PeopleRows {
    let resolved: Vec<_> = dirs
        .into_par_iter()
        .zip(events)
        .zip(clicks)
        .enumerate()
        .map(|(shard, ((mut dir, events), clicks))| {
            let events = dir.route(shard, events, state, chunks);
            let clicks = dir.route(shard, clicks, state, chunks);
            (dir.names, events, clicks)
        })
        .collect();
    let mut names = Vec::with_capacity(USERS);
    let mut event_batches = Vec::with_capacity(USERS);
    let mut click_batches = Vec::with_capacity(USERS);
    for (partition, events, clicks) in resolved {
        names.push(partition);
        event_batches.push(events);
        click_batches.push(clicks);
    }
    PeopleRows {
        names,
        events: buckets::transpose(event_batches, USERS),
        clicks: buckets::transpose(click_batches, USERS),
    }
}
