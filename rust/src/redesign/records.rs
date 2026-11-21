use crate::buckets::{self, IDS, Partitions, USERS, bucket};
use crate::calendar::Calendar;
use crate::key::PackedKey;
use crate::scan::rows;
use ahash::RandomState;
use hashbrown::{HashTable, hash_table::Entry};
use memmap2::Mmap;
use rayon::prelude::*;
use shopping_sessions::aggregate::{Click, Event};
use shopping_sessions::catalog::{Campaigns, Products};
use shopping_sessions::csv::Location;
use shopping_sessions::time::{DAY_MS, MAX_PRICE_CENTS, Window, bounded_uint};
use std::ops::Range;

// A structurally valid row keeps its place in the index even with invalid values.
const INVALID: i64 = i64::MIN;

pub struct Record<T> {
    id: PackedKey,
    pub user: PackedKey,
    pub payload: T,
}

#[derive(Clone, Copy)]
pub struct PendingEvent {
    ingest: i64,
    event: Event,
}

pub trait Candidate: Send + Sync + Copy {
    fn discard(&mut self);
    type Valid: Send;
    fn earlier(&self, other: &Self) -> bool;
    fn validate(self) -> Option<Self::Valid>;
}

impl Candidate for PendingEvent {
    type Valid = Event;
    fn discard(&mut self) {
        self.event.time = INVALID;
    }
    fn earlier(&self, other: &Self) -> bool {
        self.ingest < other.ingest
    }
    fn validate(self) -> Option<Event> {
        (self.event.time != INVALID && self.ingest - self.event.time <= DAY_MS)
            .then_some(self.event)
    }
}

impl Candidate for Click {
    type Valid = Click;
    fn discard(&mut self) {
        self.time = INVALID;
    }
    fn earlier(&self, _: &Self) -> bool {
        false
    }
    fn validate(self) -> Option<Self> {
        (self.time != INVALID).then_some(self)
    }
}

// Estimate allocation sizes from complete sampled rows. All buffers still
// grow normally when worker scheduling, malformed rows or skew change demand.
fn rows_per_worker<T>(chunks: &[Mmap], range: Range<usize>) -> usize {
    let workers = range.len().min(rayon::current_num_threads()).max(1);
    let mut estimated = 0u128;
    let mut bytes = 0u128;
    for i in range {
        let chunk = &chunks[i];
        bytes += chunk.len() as u128;
        let sample = &chunk[..chunk.len().min(4096)];
        if let Some(end) = memchr::memrchr(b'\n', sample) {
            let rows = memchr::memchr_iter(b'\n', &sample[..=end]).count();
            estimated += chunk.len() as u128 * rows as u128 / (end + 1) as u128;
        }
    }
    // Bound speculative capacity even if the sample consists of empty lines.
    let limit = bytes * 2 / (size_of::<Record<T>>() + size_of::<Ticket>()) as u128;
    ((estimated + estimated / 8).min(limit) / workers as u128) as usize
}

pub fn parse_events(
    chunks: &[Mmap],
    range: Range<usize>,
    hash: &RandomState,
    window: Window,
    products: &Products,
) -> Vec<Batch<PendingEvent>> {
    let expected = rows_per_worker::<PendingEvent>(chunks, range.clone());
    let batches = buckets::consume_chunks(
        range.start,
        range.len(),
        || vec![Batch::new(expected)],
        |out, i| {
            let chunk = &chunks[i];
            let mut calendar = Calendar::new();
            for (_, [id, user, ts, ingest, kind, product, price]) in rows::<7>(chunk) {
                if id.is_empty() {
                    continue;
                }
                let id = PackedKey::new(id, chunk, i);
                // The rules guarantee a valid UTC whole-second ingest timestamp.
                let ingest = calendar.read(ingest).expect("invalid ingest timestamp");
                let (price, category, valid_kind) = match kind {
                    b"view" | b"cart" => (0, 0, true),
                    b"purchase" => {
                        let price = bounded_uint(price, MAX_PRICE_CENTS).unwrap_or(0) as u32;
                        (
                            price,
                            products
                                .ids
                                .get(product)
                                .copied()
                                .unwrap_or(products.unknown),
                            price != 0,
                        )
                    }
                    _ => (0, 0, false),
                };
                let time = if valid_kind && !user.is_empty() && !product.is_empty() {
                    calendar.timestamp(window, ts).unwrap_or(INVALID)
                } else {
                    INVALID
                };
                let user = PackedKey::new(user, chunk, i);
                append(
                    out,
                    id.hash(hash, chunks),
                    user.hash(hash, chunks),
                    Record {
                        id,
                        user,
                        payload: PendingEvent {
                            ingest,
                            event: Event {
                                time,
                                price,
                                category,
                            },
                        },
                    },
                );
            }
        },
    );
    batches.into_iter().flatten().collect()
}

pub fn parse_clicks(
    chunks: &[Mmap],
    range: Range<usize>,
    hash: &RandomState,
    window: Window,
    campaigns: &Campaigns,
) -> Vec<Batch<Click>> {
    let expected = rows_per_worker::<Click>(chunks, range.clone());
    let batches = buckets::consume_chunks(
        range.start,
        range.len(),
        || vec![Batch::new(expected)],
        |out, i| {
            let chunk = &chunks[i];
            let mut calendar = Calendar::new();
            for (offset, [id, user, campaign, ts]) in rows::<4>(chunk) {
                if id.is_empty() {
                    continue;
                }
                let id = PackedKey::new(id, chunk, i);
                let campaign = campaigns.ids.get(campaign).copied().unwrap_or(0);
                let time = if !user.is_empty() && campaign != 0 {
                    calendar.timestamp(window, ts).unwrap_or(INVALID)
                } else {
                    INVALID
                };
                let user = PackedKey::new(user, chunk, i);
                append(
                    out,
                    id.hash(hash, chunks),
                    user.hash(hash, chunks),
                    Record {
                        id,
                        user,
                        payload: Click {
                            time,
                            campaign,
                            location: Location::new(i, offset),
                        },
                    },
                );
            }
        },
    );
    batches.into_iter().flatten().collect()
}

// Only these eight bytes per row move through the ID partitions. Payloads
// already belong to the user partition that will consume them.
struct Ticket {
    projection: u32,
    slot: u32,
}

pub struct Batch<T> {
    rows: Vec<Vec<Record<T>>>,
    tickets: Vec<Vec<Ticket>>,
}

impl<T> Batch<T> {
    fn new(expected: usize) -> Self {
        Self {
            rows: (0..USERS)
                .map(|_| Vec::with_capacity(expected / USERS))
                .collect(),
            tickets: (0..IDS)
                .map(|_| Vec::with_capacity(expected / IDS))
                .collect(),
        }
    }

    fn get(&self, slot: u32) -> &Record<T> {
        &self.rows[bucket_of_slot(slot)][index_of_slot(slot)]
    }
}

fn bucket_of_slot(slot: u32) -> usize {
    slot as usize % USERS
}
fn index_of_slot(slot: u32) -> usize {
    slot as usize / USERS
}

fn append<T>(batches: &mut Vec<Batch<T>>, id_hash: u64, user_hash: u64, row: Record<T>) {
    let user = bucket(user_hash, USERS);
    let max_index = u32::MAX as usize / USERS;
    if batches.last().unwrap().rows[user].len() > max_index {
        batches.push(Batch::new(0));
    }
    let batch = batches.last_mut().unwrap();
    let slot = (batch.rows[user].len() * USERS + user) as u32;
    batch.rows[user].push(row);
    batch.tickets[bucket(id_hash, IDS)].push(Ticket {
        projection: id_hash as u32,
        slot,
    });
}

#[derive(Clone, Copy)]
struct Place {
    batch: usize,
    slot: u32,
    projection: u32,
}

fn losers<T: Candidate>(
    runs: Vec<(usize, Vec<Ticket>)>,
    batches: &[Batch<T>],
    chunks: &[Mmap],
) -> Vec<Place> {
    let count: usize = runs.iter().map(|(_, run)| run.len()).sum();
    let words = count.div_ceil(8).next_power_of_two();
    let mut seen = vec![0u64; words];
    let mut repeated = vec![0u64; words];
    let bit = |h: u32| ((h as usize >> 6) & (words - 1), 1u64 << (h & 63));
    let mut hits = 0;
    for (_, run) in &runs {
        for ticket in run {
            let (i, mask) = bit(ticket.projection);
            let old = seen[i] & mask;
            repeated[i] |= old;
            seen[i] |= mask;
            hits += usize::from(old != 0);
        }
    }
    let mut table: HashTable<Place> = HashTable::with_capacity((2 * hits).min(count));
    let mut lost = Vec::new();
    let hash = |h: u32| u64::from(h).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    for (batch, run) in runs {
        for Ticket { projection, slot } in run {
            let (word, mask) = bit(projection);
            if repeated[word] & mask == 0 {
                continue;
            }
            let place = Place {
                batch,
                slot,
                projection,
            };
            let row = || batches[batch].get(slot);
            match table.entry(
                hash(projection),
                |stored| {
                    stored.projection == projection
                        && row()
                            .id
                            .matches(batches[stored.batch].get(stored.slot).id, chunks)
                },
                |stored| hash(stored.projection),
            ) {
                Entry::Vacant(entry) => {
                    entry.insert(place);
                }
                Entry::Occupied(mut entry) => {
                    let stored = entry.get_mut();
                    let earlier = row()
                        .payload
                        .earlier(&batches[stored.batch].get(stored.slot).payload);
                    lost.push(if earlier {
                        std::mem::replace(stored, place)
                    } else {
                        place
                    });
                }
            }
        }
    }
    lost
}

pub fn dedup<T: Candidate>(mut batches: Vec<Batch<T>>, chunks: &[Mmap]) -> Partitions<Record<T>> {
    let mut partitions = buckets::empty(IDS);
    for (i, batch) in batches.iter_mut().enumerate() {
        for (shard, tickets) in partitions
            .iter_mut()
            .zip(std::mem::take(&mut batch.tickets))
        {
            if !tickets.is_empty() {
                shard.push((i, tickets));
            }
        }
    }
    let lost: Vec<_> = partitions
        .into_par_iter()
        .map(|runs| losers(runs, &batches, chunks))
        .collect();
    let mut discard = buckets::empty(batches.len());
    for place in lost.into_iter().flatten() {
        discard[place.batch].push(place.slot);
    }
    let groups = batches
        .into_par_iter()
        .zip(discard)
        .map(|(mut batch, slots)| {
            for slot in slots {
                batch.rows[bucket_of_slot(slot)][index_of_slot(slot)]
                    .payload
                    .discard();
            }
            batch.rows
        })
        .collect();
    buckets::transpose(groups, USERS)
}
