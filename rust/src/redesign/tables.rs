use crate::buckets::{Partitions, USERS};
use crate::key::Key;
use crate::records::PackedEvent;
use memmap2::Mmap;
use parquet::errors::Result;
use rayon::prelude::*;
use shopping_sessions::aggregate::{
    Attribution, CampaignDays, CampaignTotals, Click, Event, PersonRow, UserDay,
};
use shopping_sessions::catalog::{Campaigns, Products};
use shopping_sessions::fastpq::{CampaignDaily, NullableStrings, Strings, UserDaily};
use std::io;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use shopping_sessions::output::CampaignDay;
use shopping_sessions::time::{DAY_MS, Window};

// Limit scratch per worker for long windows or large campaign catalogs.
enum Totals {
    Dense {
        first: i32,
        campaigns: usize,
        rows: Vec<CampaignDay>,
        // Assist counts are the most frequent update. A separate day-major
        // matrix keeps each session's increments within one compact row.
        assists: Vec<i64>,
    },
    Sparse(CampaignDays),
}

impl Totals {
    fn new(window: Window, campaigns: usize) -> Self {
        let first = window.start.div_euclid(DAY_MS) as i32;
        let last = (window.end - 1).div_euclid(DAY_MS) as i32;
        let days = (i64::from(last) - i64::from(first) + 1).max(0) as usize;
        if let Some(size) = days
            .checked_mul(campaigns)
            .filter(|&n| n <= (8 << 20) / size_of::<CampaignDay>())
        {
            Self::Dense {
                first,
                campaigns,
                rows: std::iter::repeat_with(CampaignDay::default)
                    .take(size)
                    .collect(),
                assists: vec![0; size],
            }
        } else {
            Self::Sparse(CampaignDays::new())
        }
    }

    /// Visit every campaign day with a session or an assist.
    fn cells(self, mut visit: impl FnMut((u32, i32), CampaignDay)) {
        match self {
            Self::Dense {
                first,
                campaigns,
                rows,
                assists,
            } => {
                for (index, (mut totals, assists)) in rows.into_iter().zip(assists).enumerate() {
                    totals.assists += assists;
                    if totals.sessions != 0 || totals.assists != 0 {
                        let key = (
                            (index % campaigns) as u32,
                            first + (index / campaigns) as i32,
                        );
                        visit(key, totals);
                    }
                }
            }
            Self::Sparse(rows) => {
                for (key, totals) in rows {
                    visit(key, totals);
                }
            }
        }
    }

    /// Combine the workers' totals. Equal-shaped dense matrices are summed
    /// cell by cell; any other mix goes through a map.
    fn combine(parts: Vec<Self>, mut visit: impl FnMut((u32, i32), CampaignDay)) {
        let mut parts = parts.into_iter();
        let Some(mut total) = parts.next() else {
            return;
        };
        let mut map = CampaignDays::new();
        for part in parts {
            match (&mut total, part) {
                (
                    Self::Dense {
                        first,
                        campaigns,
                        rows,
                        assists,
                    },
                    Self::Dense {
                        first: part_first,
                        campaigns: part_campaigns,
                        rows: part_rows,
                        assists: part_assists,
                    },
                ) if *first == part_first && *campaigns == part_campaigns => {
                    for (cell, part) in rows.iter_mut().zip(part_rows) {
                        cell.merge(part);
                    }
                    for (cell, part) in assists.iter_mut().zip(part_assists) {
                        *cell += part;
                    }
                }
                (_, part) => part.cells(|key, totals| map.entry(key).or_default().merge(totals)),
            }
        }
        if map.is_empty() {
            total.cells(visit);
        } else {
            total.cells(|key, totals| map.entry(key).or_default().merge(totals));
            for (key, totals) in map {
                visit(key, totals);
            }
        }
    }
}

impl CampaignTotals for Totals {
    #[inline]
    fn daily(&mut self, campaign: u32, day: i32) -> &mut CampaignDay {
        match self {
            Self::Dense {
                first,
                campaigns,
                rows,
                ..
            } => &mut rows[(day - *first) as usize * *campaigns + campaign as usize],
            Self::Sparse(rows) => rows.daily(campaign, day),
        }
    }

    #[inline]
    fn assist(&mut self, ids: &[u32], day: i32) {
        match self {
            Self::Dense {
                first,
                campaigns,
                assists,
                ..
            } => {
                let start = (day - *first) as usize * *campaigns;
                let row = &mut assists[start..start + *campaigns];
                for &id in ids {
                    row[id as usize] += 1;
                }
            }
            Self::Sparse(rows) => rows.assist(ids, day),
        }
    }
}

struct TextColumn {
    offsets: Vec<usize>,
    bytes: Vec<u8>,
}

impl Default for TextColumn {
    fn default() -> Self {
        Self {
            offsets: vec![0],
            bytes: Vec::new(),
        }
    }
}

impl TextColumn {
    fn push(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
        self.offsets.push(self.bytes.len());
    }

    fn strings(&self) -> io::Result<Strings<'_>> {
        let data = std::str::from_utf8(&self.bytes)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        Ok(Strings {
            data,
            offsets: &self.offsets,
        })
    }
}

#[derive(Default)]
pub struct UserColumns {
    user_id: TextColumn,
    day: Vec<i32>,
    sessions: Vec<i64>,
    events: Vec<i64>,
    revenue_cents: Vec<i64>,
    top_category: TextColumn,
    category_validity: Vec<u8>,
}

#[derive(Default)]
pub struct CampaignColumns {
    campaign_id: TextColumn,
    channel: TextColumn,
    day: Vec<i32>,
    sessions: Vec<i64>,
    purchasing_sessions: Vec<i64>,
    revenue_cents: Vec<i64>,
    assisted_sessions: Vec<i64>,
}

struct Groups<T> {
    rows: Vec<T>,
    ends: Vec<usize>,
}

impl<T> Groups<T> {
    fn groups(&self) -> impl Iterator<Item = &[T]> {
        let mut start = 0;
        self.ends.iter().map(move |&end| {
            let rows = &self.rows[start..end];
            start = end;
            rows
        })
    }
}

// For long event histories, sort only the four timestamp bytes. Equal-time
// payloads belong to the same session, so their order has no effect on sums.
fn sort_packed_events(rows: &mut [u64], scratch: &mut Vec<u64>) {
    if rows.len() < 4096 {
        rows.sort_unstable();
        return;
    }
    let mut counts = [[0usize; 256]; 4];
    for &row in rows.iter() {
        let bytes = ((row >> 32) as u32).to_le_bytes();
        for (hist, byte) in counts.iter_mut().zip(bytes) {
            hist[byte as usize] += 1;
        }
    }
    scratch.resize(rows.len(), 0);
    let mut in_scratch = false;
    for (pass, hist) in counts.iter_mut().enumerate() {
        if hist.contains(&rows.len()) {
            continue;
        }
        let mut start = 0;
        for count in hist.iter_mut() {
            let len = *count;
            *count = start;
            start += len;
        }
        let (src, dst) = if in_scratch {
            (&scratch[..], &mut rows[..])
        } else {
            (&rows[..], &mut scratch[..])
        };
        let shift = 32 + pass * 8;
        for &row in src {
            let cursor = &mut hist[((row >> shift) & 255) as usize];
            dst[*cursor] = row;
            *cursor += 1;
        }
        in_scratch = !in_scratch;
    }
    if in_scratch {
        rows.copy_from_slice(scratch);
    }
}

/// Event payloads from resolution and the form each person's history is
/// sorted in. Packed words sort by their timestamp bytes; full rows by time.
pub trait EventRows: Copy + Send + Sync {
    type Sorted: Copy + Send;
    fn sorted(self) -> Self::Sorted;
    fn sort(rows: &mut [Self::Sorted], scratch: &mut Vec<u64>);
    fn event(row: &Self::Sorted, window: Window) -> Event;
}

impl EventRows for PackedEvent {
    type Sorted = u64;
    #[inline]
    fn sorted(self) -> u64 {
        self.0
    }
    fn sort(rows: &mut [u64], scratch: &mut Vec<u64>) {
        sort_packed_events(rows, scratch);
    }
    #[inline]
    fn event(row: &u64, window: Window) -> Event {
        PackedEvent(*row).decode(window)
    }
}

impl EventRows for Event {
    type Sorted = Event;
    #[inline]
    fn sorted(self) -> Event {
        self
    }
    fn sort(rows: &mut [Event], _: &mut Vec<u64>) {
        rows.sort_unstable_by_key(|event| event.time);
    }
    #[inline]
    fn event(row: &Event, _: Window) -> Event {
        *row
    }
}

// Person IDs are dense indices into this shard's name vector. Count their
// rows first so each person has a contiguous range for the timestamp sort.
fn order_people<T, U: Copy>(
    runs: Vec<Vec<PersonRow<T>>>,
    people: usize,
    encode: impl Fn(T) -> U,
    mut sort: impl FnMut(&mut [U]),
) -> Groups<U> {
    let mut ends = vec![0; people];
    for run in &runs {
        for row in run {
            ends[row.person as usize] += 1;
        }
    }
    let mut total = 0;
    for cursor in &mut ends {
        let count = *cursor;
        *cursor = total;
        total += count;
    }
    let mut rows: Vec<U> = Vec::with_capacity(total);
    let dest = rows.spare_capacity_mut();
    for run in runs {
        for row in run {
            let cursor = &mut ends[row.person as usize];
            dest[*cursor].write(encode(row.data));
            *cursor += 1;
        }
    }
    // Each cursor started at a distinct range of the counted length.
    // The second pass wrote exactly those rows and initialized the full vector.
    unsafe { rows.set_len(total) };
    let mut start = 0;
    for &end in &ends {
        sort(&mut rows[start..end]);
        start = end;
    }
    Groups { rows, ends }
}

fn process_groups<T>(
    events: &Groups<T>,
    clicks: &Groups<Click>,
    decode: impl Fn(&T) -> Event,
    user_day: &mut UserDay,
    attribution: &mut Attribution<'_>,
    totals: &mut Totals,
    emit: &mut impl FnMut(u32, i32, i64, i64, i64, Option<u32>) -> Result<()>,
) -> Result<()> {
    for (person, (events, clicks)) in events.groups().zip(clicks.groups()).enumerate() {
        if !events.is_empty() {
            user_day.process_person(
                person as u32,
                events,
                &decode,
                clicks,
                attribution,
                totals,
                emit,
            )?;
        }
    }
    Ok(())
}

/// Each worker writes its own user file as soon as its shards finish, then
/// calls `release` while other workers may still be aggregating.
#[allow(clippy::too_many_arguments)]
pub fn aggregate<E: EventRows>(
    names: &[Vec<Key>],
    events: Partitions<PersonRow<E>>,
    clicks: Partitions<PersonRow<Click>>,
    chunks: &[Mmap],
    products: &Products,
    campaigns: &Campaigns,
    window: Window,
    user_dir: &Path,
    release: &(impl Fn() + Sync),
) -> Result<CampaignColumns> {
    let parallelism = rayon::current_num_threads().min(USERS);
    // Workers claim the largest remaining shard first to balance the finish.
    let mut order: Vec<_> = (0..events.len()).collect();
    order
        .sort_by_key(|&shard| std::cmp::Reverse(events[shard].iter().map(Vec::len).sum::<usize>()));
    let shards: Vec<_> = events
        .into_iter()
        .zip(clicks)
        .map(|shard| Mutex::new(Some(shard)))
        .collect();
    let next = AtomicUsize::new(0);
    let parts: Vec<_> = (0..parallelism)
        .into_par_iter()
        .map(|part| -> Result<_> {
            let parts = std::iter::from_fn(|| {
                let shard = *order.get(next.fetch_add(1, Ordering::Relaxed))?;
                let taken = shards[shard].lock().unwrap().take();
                Some((shard, taken.unwrap()))
            });
            let mut daily = UserColumns::default();
            let mut user_day = UserDay::default();
            let mut sort_scratch = Vec::new();
            let mut totals = Totals::new(window, campaigns.values.len());
            let mut attribution = Attribution::new(chunks, campaigns.values.len());
            for (shard, (events, clicks)) in parts {
                if events.is_empty() {
                    continue;
                }
                let people = names[shard].len();
                let clicks = order_people(
                    clicks,
                    people,
                    |c| c,
                    |rows| rows.sort_unstable_by_key(|c| c.time),
                );
                let mut emit = |id: u32, day, sessions, count, revenue, top: Option<u32>| {
                    daily.user_id.push(names[shard][id as usize].bytes(chunks));
                    daily.day.push(day);
                    daily.sessions.push(sessions);
                    daily.events.push(count);
                    daily.revenue_cents.push(revenue);
                    let row = daily.day.len() - 1;
                    if row % 8 == 0 {
                        daily.category_validity.push(0);
                    }
                    if let Some(id) = top {
                        daily.category_validity[row / 8] |= 1 << (row % 8);
                        daily.top_category.push(&products.categories[id as usize]);
                    } else {
                        daily.top_category.push(&[]);
                    }
                    Ok(())
                };
                let events = order_people(events, people, E::sorted, |rows| {
                    E::sort(rows, &mut sort_scratch)
                });
                process_groups(
                    &events,
                    &clicks,
                    |row| E::event(row, window),
                    &mut user_day,
                    &mut attribution,
                    &mut totals,
                    &mut emit,
                )?;
            }
            write_users(&user_dir.join(format!("part-{part}.parquet")), &daily)?;
            drop(daily);
            release();
            Ok(totals)
        })
        .collect::<Result<_>>()?;
    let mut campaign = CampaignColumns::default();
    Totals::combine(parts, |(id, day), x| {
        let (name, channel) = &campaigns.values[id as usize];
        campaign.campaign_id.push(name);
        campaign.channel.push(channel);
        campaign.day.push(day);
        campaign.sessions.push(x.sessions);
        campaign.purchasing_sessions.push(x.purchasing);
        campaign.revenue_cents.push(x.revenue);
        campaign.assisted_sessions.push(x.assists);
    });
    Ok(campaign)
}

fn write_users(path: &Path, batch: &UserColumns) -> io::Result<()> {
    UserDaily {
        user_id: batch.user_id.strings()?,
        day: &batch.day,
        sessions: &batch.sessions,
        events: &batch.events,
        revenue_cents: &batch.revenue_cents,
        top_category: NullableStrings {
            values: batch.top_category.strings()?,
            validity: &batch.category_validity,
        },
    }
    .write(path)
}

pub fn write_campaigns(path: &Path, campaigns: &CampaignColumns) -> io::Result<()> {
    CampaignDaily {
        campaign_id: campaigns.campaign_id.strings()?,
        channel: campaigns.channel.strings()?,
        day: &campaigns.day,
        sessions: &campaigns.sessions,
        purchasing_sessions: &campaigns.purchasing_sessions,
        revenue_cents: &campaigns.revenue_cents,
        assisted_sessions: &campaigns.assisted_sessions,
    }
    .write(path)
}
