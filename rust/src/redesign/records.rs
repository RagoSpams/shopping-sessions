use crate::buckets::{self, IDS, USERS, bucket};
use crate::calendar::Calendar;
use crate::chunks::Span;
use crate::dims::Dimension;
use crate::key::PackedKey;
use crate::scan::rows;
use ahash::RandomState;
use hashbrown::{HashTable, hash_table::Entry};
use memmap2::Mmap;
use rayon::prelude::*;
use shopping_sessions::aggregate::{Click, Event};
use shopping_sessions::csv::Location;
use shopping_sessions::time::{DAY_MS, MAX_PRICE_CENTS, Window, bounded_uint};
use std::ops::Range;

// A structurally valid row keeps its place in the index even with invalid values.
const INVALID: i64 = i64::MIN;

/// A row sits in the partition of its user's hash. It has the size and
/// alignment of the person row it becomes, so resolution rewrites each
/// partition run in place.
#[derive(Clone, Copy)]
pub struct Record<T> {
    pub user: PackedKey,
    pub payload: T,
}

pub trait Candidate: Send + Sync + Copy {
    fn discard(&mut self);
    fn valid(&self) -> bool;
}

impl Candidate for Event {
    fn discard(&mut self) {
        self.time = INVALID;
    }
    fn valid(&self) -> bool {
        self.time != INVALID
    }
}

impl Candidate for Click {
    fn discard(&mut self) {
        self.time = INVALID;
    }
    fn valid(&self) -> bool {
        self.time != INVALID
    }
}

/// A click as one sortable word, the click-window-relative millisecond above
/// the campaign, plus the row location that supplies its ID for ties.
/// Campaign 0 is organic and never belongs to a valid click, so a zero
/// campaign field marks a row that cannot survive.
#[derive(Clone, Copy)]
pub struct PackedClick {
    pub word: u64,
    pub location: Location,
}

impl PackedClick {
    /// True when every valid click of this window packs losslessly.
    pub fn fits(window: Window) -> bool {
        window.end - window.start <= i64::from(u32::MAX) + 1
    }

    #[inline]
    pub fn decode(self, window: Window) -> Click {
        Click {
            time: window.start + (self.word >> 32) as i64,
            campaign: self.word as u32,
            location: self.location,
        }
    }
}

impl Candidate for PackedClick {
    fn discard(&mut self) {
        self.word &= !u64::from(u32::MAX);
    }
    fn valid(&self) -> bool {
        self.word as u32 != 0
    }
}

/// The payload a parsed click row carries until aggregation.
pub trait ClickCodec: Candidate {
    fn encode(window: Window, time: i64, campaign: u32, location: Location) -> Self;
}

impl ClickCodec for Click {
    #[inline]
    fn encode(_: Window, time: i64, campaign: u32, location: Location) -> Self {
        Click {
            time,
            campaign,
            location,
        }
    }
}

impl ClickCodec for PackedClick {
    #[inline]
    fn encode(window: Window, time: i64, campaign: u32, location: Location) -> Self {
        if time == INVALID {
            return Self { word: 0, location };
        }
        let offset = (time - window.start) as u64;
        Self {
            word: (offset << 32) | u64::from(campaign),
            location,
        }
    }
}

pub const PRICE_BITS: u32 = (MAX_PRICE_CENTS as u32).ilog2() + 1;
pub const PRICE_MASK: u32 = (1 << PRICE_BITS) - 1;

/// An event as one sortable word: the window-relative millisecond in the high
/// half, then the category and price. No valid price fills the price field,
/// so that value marks a row that cannot survive.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct PackedEvent(pub u64);

impl PackedEvent {
    /// True when every valid event of this input packs losslessly.
    pub fn fits(window: Window, categories: usize) -> bool {
        window.end - window.start <= i64::from(u32::MAX) + 1 && categories <= 1 << (32 - PRICE_BITS)
    }

    #[inline]
    pub fn decode(self, window: Window) -> Event {
        Event {
            time: window.start + (self.0 >> 32) as i64,
            price: self.0 as u32 & PRICE_MASK,
            category: self.0 as u32 >> PRICE_BITS,
        }
    }
}

impl Candidate for PackedEvent {
    fn discard(&mut self) {
        self.0 |= u64::from(PRICE_MASK);
    }
    fn valid(&self) -> bool {
        self.0 as u32 & PRICE_MASK != PRICE_MASK
    }
}

/// The payload a parsed event row carries until aggregation.
pub trait EventCodec: Candidate {
    fn encode(window: Window, time: i64, price: u32, category: u32) -> Self;
}

impl EventCodec for Event {
    #[inline]
    fn encode(_: Window, time: i64, price: u32, category: u32) -> Self {
        Event {
            time,
            price,
            category,
        }
    }
}

impl EventCodec for PackedEvent {
    #[inline]
    fn encode(window: Window, time: i64, price: u32, category: u32) -> Self {
        if time == INVALID {
            return Self(u64::from(PRICE_MASK));
        }
        let offset = (time - window.start) as u64;
        Self((offset << 32) | u64::from((category << PRICE_BITS) | price))
    }
}

// A ticket key holds a row's ordering value above its slot. The slot names a
// row by index and user partition within one worker batch.
const SLOT_BITS: u32 = 25;
const SLOT_MASK: u64 = (1 << SLOT_BITS) - 1;
const MAX_INDEX: usize = (1 << SLOT_BITS) / USERS;
// The earliest F1 instant, 0001-01-01T00:00:00Z, in Unix seconds. Every
// ingest second after it, through 9999-12-31T23:59:59Z, fits 39 bits.
const FIRST_SECOND: i64 = -62_135_596_800;
const _: () = assert!((1 << SLOT_BITS) % USERS == 0);
const _: () = assert!((253_402_300_799 - FIRST_SECOND) >> (64 - SLOT_BITS) == 0);

// Estimate allocation sizes from complete sampled rows. All buffers still
// grow normally when worker scheduling, malformed rows or skew change demand.
fn rows_per_worker<T>(chunks: &[Mmap], spans: &[Span], range: Range<usize>) -> usize {
    let workers = range.len().min(rayon::current_num_threads()).max(1);
    let bytes: u128 = range.clone().map(|i| spans[i].nominal as u128).sum();
    // One page of complete rows from the first chunk with rows sets the rate.
    let mut estimated = 0u128;
    for i in range {
        let (from, until) = spans[i].rows(&chunks[i]);
        let sample = &chunks[i][from.min(until)..until.min(from + 4096)];
        if let Some(end) = memchr::memrchr(b'\n', sample) {
            let rows = memchr::memchr_iter(b'\n', &sample[..=end]).count();
            estimated = bytes * rows as u128 / (end + 1) as u128;
            break;
        }
    }
    // Bound speculative capacity even if the sample consists of empty lines.
    let row = size_of::<Record<T>>() + size_of::<Ticket>();
    let limit = bytes * 2 / row as u128;
    ((estimated + estimated / 8).min(limit) / workers as u128) as usize
}

pub fn parse_events<E: EventCodec>(
    chunks: &[Mmap],
    spans: &[Span],
    range: Range<usize>,
    hash: &RandomState,
    window: Window,
    products: &Dimension,
) -> Vec<Batch<E>> {
    let expected = rows_per_worker::<E>(chunks, spans, range.clone());
    let batches = buckets::consume_chunks(
        spans,
        range,
        || vec![Batch::new(expected)],
        |out, i| {
            let chunk = &chunks[i];
            let mut calendar = Calendar::new();
            let (from, until) = spans[i].rows(chunk);
            for (_, [id, user, ts, ingest, kind, product, price]) in rows::<7>(chunk, from, until) {
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
                        (price, products.get(product), price != 0)
                    }
                    _ => (0, 0, false),
                };
                let mut time = if valid_kind && !user.is_empty() && !product.is_empty() {
                    calendar.timestamp(window, ts).unwrap_or(INVALID)
                } else {
                    INVALID
                };
                // A late copy can still win deduplication, so lateness is part
                // of this row's own validity.
                if time != INVALID && ingest - time > DAY_MS {
                    time = INVALID;
                }
                let user = PackedKey::new(user, chunk, i);
                append(
                    out,
                    id,
                    id.hash(hash, chunks),
                    // Ingest is whole seconds, so the division is exact.
                    (ingest / 1000 - FIRST_SECOND) as u64,
                    bucket(user.hash(hash, chunks), USERS),
                    Record {
                        user,
                        payload: E::encode(window, time, price, category),
                    },
                );
            }
        },
    );
    batches.into_iter().flatten().collect()
}

pub fn parse_clicks<C: ClickCodec>(
    chunks: &[Mmap],
    spans: &[Span],
    range: Range<usize>,
    hash: &RandomState,
    window: Window,
    campaigns: &Dimension,
) -> Vec<Batch<C>> {
    let expected = rows_per_worker::<C>(chunks, spans, range.clone());
    let batches = buckets::consume_chunks(
        spans,
        range,
        || vec![Batch::new(expected)],
        |out, i| {
            let chunk = &chunks[i];
            let mut calendar = Calendar::new();
            let (from, until) = spans[i].rows(chunk);
            for (offset, [id, user, campaign, ts]) in rows::<4>(chunk, from, until) {
                if id.is_empty() {
                    continue;
                }
                let id = PackedKey::new(id, chunk, i);
                let campaign = campaigns.get(campaign);
                let time = if !user.is_empty() && campaign != 0 {
                    calendar.timestamp(window, ts).unwrap_or(INVALID)
                } else {
                    INVALID
                };
                let user = PackedKey::new(user, chunk, i);
                // Copies of a click are identical, so every copy ranks equally.
                append(
                    out,
                    id,
                    id.hash(hash, chunks),
                    0,
                    bucket(user.hash(hash, chunks), USERS),
                    Record {
                        user,
                        payload: C::encode(window, time, campaign, Location::new(i, offset)),
                    },
                );
            }
        },
    );
    batches.into_iter().flatten().collect()
}

// A ticket carries the exact record ID and its ordering key into its ID
// partition. Payloads already belong to the partition that will consume them.
#[derive(Clone, Copy)]
struct Ticket {
    id: PackedKey,
    key: u64,
}

pub struct Batch<T> {
    pub rows: Vec<Vec<Record<T>>>,
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
}

#[inline]
fn append<T>(
    batches: &mut Vec<Batch<T>>,
    id: PackedKey,
    id_hash: u64,
    order: u64,
    owner: usize,
    row: Record<T>,
) {
    if batches.last().unwrap().rows[owner].len() >= MAX_INDEX {
        batches.push(Batch::new(0));
    }
    let batch = batches.last_mut().unwrap();
    let slot = (batch.rows[owner].len() * USERS + owner) as u64;
    batch.rows[owner].push(row);
    batch.tickets[bucket(id_hash, IDS)].push(Ticket {
        id,
        key: (order << SLOT_BITS) | slot,
    });
}

#[derive(Clone, Copy)]
struct Place {
    batch: usize,
    projection: u32,
    ticket: Ticket,
}

fn losers(
    runs: Vec<(usize, Vec<Ticket>)>,
    state: &RandomState,
    chunks: &[Mmap],
) -> Vec<(usize, u32)> {
    let count: usize = runs.iter().map(|(_, run)| run.len()).sum();
    let words = count.div_ceil(8).next_power_of_two();
    let mut seen = vec![0u64; words];
    let mut repeated = vec![0u64; words];
    let bit = |h: u32| ((h as usize >> 6) & (words - 1), 1u64 << (h & 63));
    let mut hits = 0;
    for (_, run) in &runs {
        for ticket in run {
            let (i, mask) = bit(ticket.id.hash(state, chunks) as u32);
            let old = seen[i] & mask;
            repeated[i] |= old;
            seen[i] |= mask;
            hits += usize::from(old != 0);
        }
    }
    drop(seen);
    let mut table: HashTable<Place> = HashTable::with_capacity((2 * hits).min(count));
    let mut lost = Vec::new();
    let hash = |h: u32| u64::from(h).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    for (batch, run) in runs {
        for ticket in run {
            let projection = ticket.id.hash(state, chunks) as u32;
            let (word, mask) = bit(projection);
            if repeated[word] & mask == 0 {
                continue;
            }
            let place = Place {
                batch,
                projection,
                ticket,
            };
            match table.entry(
                hash(projection),
                |stored| {
                    stored.projection == projection && stored.ticket.id.matches(ticket.id, chunks)
                },
                |stored| hash(stored.projection),
            ) {
                Entry::Vacant(entry) => {
                    entry.insert(place);
                }
                Entry::Occupied(mut entry) => {
                    // Equal ordering values belong to identical copies.
                    let stored = entry.get_mut();
                    let loser = if ticket.key >> SLOT_BITS < stored.ticket.key >> SLOT_BITS {
                        std::mem::replace(stored, place)
                    } else {
                        place
                    };
                    lost.push((loser.batch, (loser.ticket.key & SLOT_MASK) as u32));
                }
            }
        }
    }
    lost
}

/// Marks every copy except the one with the earliest ingest (any copy for clicks).
pub fn dedup<T: Candidate>(
    mut batches: Vec<Batch<T>>,
    state: &RandomState,
    chunks: &[Mmap],
) -> Vec<Batch<T>> {
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
        .map(|runs| losers(runs, state, chunks))
        .collect();
    let mut discard = buckets::empty(batches.len());
    for (batch, slot) in lost.into_iter().flatten() {
        discard[batch].push(slot);
    }
    batches
        .par_iter_mut()
        .zip(discard)
        .for_each(|(batch, slots)| {
            for slot in slots {
                let slot = slot as usize;
                batch.rows[slot % USERS][slot / USERS].payload.discard();
            }
        });
    batches
}
