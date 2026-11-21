use crate::buckets::{self, Partitions, Runs, USERS, bucket};
use crate::key::{Key, PackedKey};
use crate::records::{Candidate, PendingEvent, Record};
use crate::scan::rows;
use ahash::RandomState;
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

// Empty IDs are invalid in links and in every routed row, so the zero packed
// key is an available empty slot. Store the canonical person alongside the
// exact key: a successful linear probe reads both on the same cache line.
#[derive(Clone, Copy, Default)]
struct AliasSlot {
    name: PackedKey,
    hash: u32,
    person: u32,
}

#[derive(Default)]
pub struct Directory {
    names: Vec<Key>,
    slots: Vec<AliasSlot>,
}

impl Directory {
    #[inline]
    fn probe(&self, name: PackedKey, hash: u32, chunks: &[Mmap]) -> usize {
        let mask = self.slots.len() - 1;
        let mut i = hash as usize & mask;
        loop {
            let slot = &self.slots[i];
            if slot.name == PackedKey::default()
                || (slot.hash == hash && name.matches(slot.name, chunks))
            {
                return i;
            }
            i = (i + 1) & mask;
        }
    }

    fn grow(&mut self) {
        let capacity = (2 * self.slots.len()).max(16);
        let old = std::mem::replace(&mut self.slots, vec![AliasSlot::default(); capacity]);
        for slot in old
            .into_iter()
            .filter(|slot| slot.name != PackedKey::default())
        {
            let mut i = slot.hash as usize & (capacity - 1);
            while self.slots[i].name != PackedKey::default() {
                i = (i + 1) & (capacity - 1);
            }
            self.slots[i] = slot;
        }
    }

    #[inline]
    fn intern(&mut self, shard: usize, name: PackedKey, hash: u64, chunks: &[Mmap]) -> u32 {
        let hash = hash as u32;
        if self.slots.is_empty() {
            self.grow();
        }
        let mut i = self.probe(name, hash, chunks);
        if self.slots[i].name != PackedKey::default() {
            return self.slots[i].person;
        }
        if (self.names.len() + 1) * 3 > self.slots.len() * 2 {
            self.grow();
            i = self.probe(name, hash, chunks);
        }
        let id = person(shard, self.names.len());
        self.names.push(name.name(chunks));
        self.slots[i] = AliasSlot {
            name,
            hash,
            person: id,
        };
        id
    }

    fn route<T: Candidate>(
        &mut self,
        shard: usize,
        rows: Runs<Record<T>>,
        state: &RandomState,
        chunks: &[Mmap],
    ) -> Vec<Vec<PersonRow<T::Valid>>> {
        let mut output = buckets::empty(USERS);
        for run in rows {
            for block in run.chunks(32) {
                let mut hashes = [0; 32];
                for (hash, row) in hashes.iter_mut().zip(block) {
                    *hash = row.user.hash(state, chunks);
                    #[cfg(target_arch = "x86_64")]
                    if !self.slots.is_empty() {
                        let i = *hash as usize & (self.slots.len() - 1);
                        // The slot belongs to the current table. A later grow
                        // can invalidate this hint but never the lookup below.
                        unsafe {
                            std::arch::x86_64::_mm_prefetch::<{ std::arch::x86_64::_MM_HINT_T0 }>(
                                self.slots.as_ptr().add(i).cast(),
                            );
                        }
                    }
                }
                for (&hash, row) in hashes.iter().zip(block) {
                    let Some(data) = row.payload.validate() else {
                        continue;
                    };
                    let root = self.intern(shard, row.user, hash, chunks);
                    output[owner(root)].push(PersonRow {
                        person: local(root) as u32,
                        data,
                    });
                }
            }
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

pub fn build(batches: Vec<LinkBatch>, chunks: &[Mmap]) -> Vec<Directory> {
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
                    let id = dir.intern(shard, endpoint.name, endpoint.hash, chunks);
                    slots[base + endpoint.slot].store(id, Relaxed);
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
    dirs.par_iter_mut().for_each(|dir| {
        for slot in &mut dir.slots {
            if slot.name != PackedKey::default() {
                slot.person = root(&parents, slot.person);
            }
        }
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
    events: Partitions<Record<PendingEvent>>,
    clicks: Partitions<Record<Click>>,
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
