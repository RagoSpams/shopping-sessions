use crate::buckets::{self, Partitions, USERS, bucket};
use crate::key::{Key, PackedKey};
use crate::records::{Batch, Candidate, Record};
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

impl Directory {
    /// Rewrite one partition run into person rows within its own allocation.
    /// Rows whose person another partition owns move to that owner's list.
    fn convert<T: Candidate>(
        &mut self,
        shard: usize,
        run: Vec<Record<T>>,
        forwards: &mut [Vec<PersonRow<T>>],
        state: &RandomState,
        chunks: &[Mmap],
    ) -> Vec<PersonRow<T>> {
        const {
            assert!(size_of::<Record<T>>() == size_of::<PersonRow<T>>());
            assert!(align_of::<Record<T>>() == align_of::<PersonRow<T>>());
        }
        let mut run = std::mem::ManuallyDrop::new(run);
        let (records, len, capacity) = (run.as_mut_ptr(), run.len(), run.capacity());
        let people = records.cast::<PersonRow<T>>();
        let mut kept = 0;
        let mut start = 0;
        while start < len {
            let end = (start + 32).min(len);
            let mut hashes = [0u64; 32];
            for (i, hash) in (start..end).zip(&mut hashes) {
                // Every index below len holds an initialized record.
                *hash = unsafe { &*records.add(i) }.user.hash(state, chunks);
                #[cfg(target_arch = "x86_64")]
                if !self.slots.is_empty() {
                    let slot = *hash as usize & (self.slots.len() - 1);
                    // The slot belongs to the current table. A later grow can
                    // invalidate this hint but never the lookup below.
                    unsafe {
                        std::arch::x86_64::_mm_prefetch::<{ std::arch::x86_64::_MM_HINT_T0 }>(
                            self.slots.as_ptr().add(slot).cast(),
                        );
                    }
                }
            }
            for (i, &hash) in (start..end).zip(&hashes) {
                // Record i is read before output slot kept <= i is written.
                // Both types are plain data of equal size, so the write never
                // reaches an unread record and the allocation keeps its layout.
                let record = unsafe { records.add(i).read() };
                if !record.payload.valid() {
                    continue;
                }
                let root = self.intern(shard, record.user, hash, chunks);
                let row = PersonRow {
                    person: local(root) as u32,
                    data: record.payload,
                };
                if owner(root) == shard {
                    unsafe { people.add(kept).write(row) };
                    kept += 1;
                } else {
                    forwards[owner(root)].push(row);
                }
            }
            start = end;
        }
        unsafe { Vec::from_raw_parts(people, kept, capacity) }
    }

    fn convert_runs<T: Candidate>(
        &mut self,
        shard: usize,
        runs: Vec<Vec<Record<T>>>,
        state: &RandomState,
        chunks: &[Mmap],
    ) -> (Vec<Vec<PersonRow<T>>>, Vec<Vec<PersonRow<T>>>) {
        let mut forwards = buckets::empty(USERS);
        let local = runs
            .into_iter()
            .filter(|run| !run.is_empty())
            .map(|run| self.convert(shard, run, &mut forwards, state, chunks))
            .collect();
        (local, forwards)
    }
}

fn take_runs<T: Candidate>(batches: &mut [Batch<T>]) -> Partitions<Record<T>> {
    let mut parts = buckets::empty(USERS);
    for batch in batches {
        for (dst, run) in parts.iter_mut().zip(std::mem::take(&mut batch.rows)) {
            dst.push(run);
        }
    }
    parts
}

// Append the rows each partition forwarded to the partitions that own them.
fn deliver<T>(parts: &mut Partitions<PersonRow<T>>, forwards: Vec<Vec<Vec<PersonRow<T>>>>) {
    for runs in forwards {
        for (dst, run) in parts.iter_mut().zip(runs) {
            if !run.is_empty() {
                dst.push(run);
            }
        }
    }
}

pub fn resolve(
    dirs: Vec<Directory>,
    mut events: Vec<Batch<Event>>,
    mut clicks: Vec<Batch<Click>>,
    state: &RandomState,
    chunks: &[Mmap],
) -> PeopleRows {
    let event_runs = take_runs(&mut events);
    let click_runs = take_runs(&mut clicks);
    drop((events, clicks));
    let resolved: Vec<_> = dirs
        .into_par_iter()
        .zip(event_runs)
        .zip(click_runs)
        .enumerate()
        .map(|(shard, ((mut dir, events), clicks))| {
            let (events, event_forwards) = dir.convert_runs(shard, events, state, chunks);
            let (clicks, click_forwards) = dir.convert_runs(shard, clicks, state, chunks);
            (dir.names, events, clicks, event_forwards, click_forwards)
        })
        .collect();
    let mut names = Vec::with_capacity(USERS);
    let mut event_parts = Vec::with_capacity(USERS);
    let mut click_parts = Vec::with_capacity(USERS);
    let mut event_forwards = Vec::with_capacity(USERS);
    let mut click_forwards = Vec::with_capacity(USERS);
    for (partition, events, clicks, ef, cf) in resolved {
        names.push(partition);
        event_parts.push(events);
        click_parts.push(clicks);
        event_forwards.push(ef);
        click_forwards.push(cf);
    }
    deliver(&mut event_parts, event_forwards);
    deliver(&mut click_parts, click_forwards);
    PeopleRows {
        names,
        events: event_parts,
        clicks: click_parts,
    }
}
