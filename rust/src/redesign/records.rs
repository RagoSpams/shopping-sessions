use crate::buckets::{self, IDS, Partitions, Runs, USERS, bucket};
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
    user: PackedKey,
    payload: T,
}

pub struct PendingEvent {
    ingest: i64,
    event: Event,
}

pub trait Candidate: Send {
    type Valid: Send;
    fn earlier(&self, other: &Self) -> bool;
    fn validate(self) -> Option<Self::Valid>;
}

impl Candidate for PendingEvent {
    type Valid = Event;
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
    fn earlier(&self, _: &Self) -> bool {
        false
    }
    fn validate(self) -> Option<Self> {
        (self.time != INVALID).then_some(self)
    }
}

pub fn parse_events(
    chunks: &[Mmap],
    range: Range<usize>,
    hash: &RandomState,
    window: Window,
    products: &Products,
) -> Partitions<Record<PendingEvent>> {
    let batches = buckets::consume_chunks(
        range.start,
        range.len(),
        || buckets::empty(IDS),
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
                let slot = bucket(id.hash(hash, chunks), IDS);
                out[slot].push(Record {
                    id,
                    user: PackedKey::new(user, chunk, i),
                    payload: PendingEvent {
                        ingest,
                        event: Event {
                            time,
                            price,
                            category,
                        },
                    },
                });
            }
        },
    );
    buckets::transpose(batches, IDS)
}

pub fn parse_clicks(
    chunks: &[Mmap],
    range: Range<usize>,
    hash: &RandomState,
    window: Window,
    campaigns: &Campaigns,
) -> Partitions<Record<Click>> {
    let batches = buckets::consume_chunks(
        range.start,
        range.len(),
        || buckets::empty(IDS),
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
                out[bucket(id.hash(hash, chunks), IDS)].push(Record {
                    id,
                    user: PackedKey::new(user, chunk, i),
                    payload: Click {
                        time,
                        campaign,
                        location: Location::new(i, offset),
                    },
                });
            }
        },
    );
    buckets::transpose(batches, IDS)
}

pub struct Named<T> {
    pub user: PackedKey,
    pub hash: u64,
    pub data: T,
}

fn winners<T: Candidate>(
    runs: Runs<Record<T>>,
    chunks: &[Mmap],
    hash: &RandomState,
    output: &mut [Vec<Named<T::Valid>>],
) {
    // A bit seen twice is only a candidate. Distinct IDs sharing that bit
    // take the same exact table path as actual duplicate IDs.
    let count: usize = runs.iter().map(Vec::len).sum();
    let words = count.div_ceil(8).next_power_of_two();
    let mut seen = vec![0u64; words];
    let mut repeated = vec![0u64; words];
    let position = |hash: u64| ((hash as usize >> 6) & (words - 1), 1u64 << (hash & 63));
    let mut hits = 0;
    for row in runs.iter().flatten() {
        let (word, bit) = position(row.id.hash(hash, chunks));
        let old = seen[word] & bit;
        repeated[word] |= old;
        seen[word] |= bit;
        hits += usize::from(old != 0);
    }
    let mut emit = |row: Record<T>| {
        if let Some(data) = row.payload.validate() {
            let hash = row.user.hash(hash, chunks);
            output[bucket(hash, USERS)].push(Named {
                user: row.user,
                hash,
                data,
            });
        }
    };
    let mut table: HashTable<Record<T>> = HashTable::with_capacity((2 * hits).min(count));
    for row in runs.into_iter().flatten() {
        let id_hash = row.id.hash(hash, chunks);
        let (word, bit) = position(id_hash);
        if repeated[word] & bit == 0 {
            emit(row);
            continue;
        }
        match table.entry(
            id_hash,
            |r| row.id.matches(r.id, chunks),
            |r| r.id.hash(hash, chunks),
        ) {
            Entry::Vacant(entry) => {
                entry.insert(row);
            }
            Entry::Occupied(mut entry) => {
                if row.payload.earlier(&entry.get().payload) {
                    *entry.get_mut() = row;
                }
            }
        }
    }
    for row in table {
        emit(row);
    }
}

pub fn dedup<T: Candidate>(
    index: Partitions<Record<T>>,
    chunks: &[Mmap],
    hash: &RandomState,
) -> Partitions<Named<T::Valid>> {
    // Reuse one set of destination buffers across several cache-sized ID tables.
    let batch = index
        .len()
        .div_ceil(rayon::current_num_threads() * 2)
        .max(1);
    let groups = index
        .into_par_iter()
        .chunks(batch)
        .map(|partitions| {
            let mut out = buckets::empty(USERS);
            for runs in partitions {
                winners(runs, chunks, hash, &mut out);
            }
            out
        })
        .collect();
    buckets::transpose(groups, USERS)
}
