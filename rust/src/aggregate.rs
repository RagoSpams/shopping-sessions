use crate::csv::Location;
use crate::output::CampaignDay;
use crate::time::{DAY_MS, LOOKBACK_MS, SESSION_GAP_MS};
use ahash::AHashMap;
use memmap2::Mmap;
use parquet::errors::Result;
use std::borrow::Borrow;

// Four-byte packing removes padding from the routed rows. Fields are copied,
// never borrowed, so unaligned eight-byte fields are read by value.
#[repr(C, packed(4))]
pub struct PersonRow<T> {
    pub person: u32,
    pub data: T,
}

#[derive(Clone, Copy)]
#[repr(C, packed(4))]
pub struct Event {
    pub time: i64,
    pub price: u32,
    pub category: u32,
}

#[derive(Clone, Copy)]
#[repr(C, packed(4))]
pub struct Click {
    pub time: i64,
    pub campaign: u32,
    pub location: Location,
}

impl Borrow<Click> for PersonRow<Click> {
    fn borrow(&self) -> &Click {
        &self.data
    }
}

pub type CampaignDays = AHashMap<(u32, i32), CampaignDay>;

pub trait CampaignTotals {
    fn daily(&mut self, campaign: u32, day: i32) -> &mut CampaignDay;

    /// Count one assisted session for each listed campaign on `day`.
    fn assist(&mut self, campaigns: &[u32], day: i32) {
        for &id in campaigns {
            self.daily(id, day).assists += 1;
        }
    }
}

impl CampaignTotals for CampaignDays {
    #[inline]
    fn daily(&mut self, campaign: u32, day: i32) -> &mut CampaignDay {
        self.entry((campaign, day)).or_default()
    }
}

// Timestamps are validated against the manifest's calendar window before
// attribution. This sentinel is outside the representable calendar range.
const INACTIVE: i64 = i64::MIN;

pub struct Attribution<'a> {
    chunks: &'a [Mmap],
    last_by_campaign: Vec<i64>,
    active: Vec<u32>,
    right: usize,
    latest: Option<Click>,
}

impl<'a> Attribution<'a> {
    pub fn new(chunks: &'a [Mmap], campaign_count: usize) -> Self {
        Self {
            chunks,
            last_by_campaign: vec![INACTIVE; campaign_count],
            active: Vec::new(),
            right: 0,
            latest: None,
        }
    }

    fn reset(&mut self) {
        for campaign in self.active.drain(..) {
            self.last_by_campaign[campaign as usize] = INACTIVE;
        }
        self.right = 0;
        self.latest = None;
    }

    fn credit(
        &mut self,
        clicks: &[impl Borrow<Click>],
        start: i64,
        day: i32,
        revenue: i64,
        campaign_days: &mut impl CampaignTotals,
    ) {
        let cutoff = start - LOOKBACK_MS;
        let first = self.right;
        while let Some(row) = clicks.get(self.right) {
            let click = row.borrow();
            if click.time > start {
                break;
            }
            if click.time >= cutoff {
                let last = &mut self.last_by_campaign[click.campaign as usize];
                if *last == INACTIVE {
                    self.active.push(click.campaign);
                }
                *last = click.time;
            }
            self.right += 1;
        }
        // Equal-time clicks enter the window together. Only ties at the
        // latest timestamp can affect the winning campaign.
        if self.right != first {
            let mut winner = *clicks[self.right - 1].borrow();
            if winner.time >= cutoff {
                for row in clicks[first..self.right - 1].iter().rev() {
                    let click = row.borrow();
                    if click.time != winner.time {
                        break;
                    }
                    if click.location.id(self.chunks) < winner.location.id(self.chunks) {
                        winner = *click;
                    }
                }
                self.latest = Some(winner);
            } else {
                self.latest = None;
            }
        }
        let campaign = self
            .latest
            .filter(|last| last.time >= cutoff)
            .map_or(0, |last| last.campaign);
        let totals = campaign_days.daily(campaign, day);
        totals.sessions += 1;
        totals.purchasing += i64::from(revenue > 0);
        totals.revenue += revenue;
        // A campaign's most recent eligible click determines whether any
        // click from it assists. Expire entries, then count the session.
        let last_by_campaign = &mut self.last_by_campaign;
        self.active.retain(|&id| {
            let last = &mut last_by_campaign[id as usize];
            if *last >= cutoff {
                true
            } else {
                *last = INACTIVE;
                false
            }
        });
        campaign_days.assist(&self.active, day);
    }
}

#[derive(Default)]
pub struct UserDay {
    sessions: i64,
    events: i64,
    revenue: i64,
    categories: Vec<(u32, i64)>,
}

impl UserDay {
    fn add_purchase(&mut self, category: u32, price: i64) {
        for (existing, total) in &mut self.categories {
            if *existing == category {
                *total += price;
                return;
            }
        }
        self.categories.push((category, price));
    }

    fn write(
        &mut self,
        emit: &mut impl FnMut(u32, i32, i64, i64, i64, Option<u32>) -> Result<()>,
        person: u32,
        day: i32,
    ) -> Result<()> {
        let top = self
            .categories
            .iter()
            // Category IDs are assigned in ascending byte order.
            .min_by_key(|&&(category, revenue)| (-revenue, category))
            .map(|&(category, _)| category);
        emit(person, day, self.sessions, self.events, self.revenue, top)?;
        self.sessions = 0;
        self.events = 0;
        self.revenue = 0;
        self.categories.clear();
        Ok(())
    }

    /// Process one nonempty timestamp-ordered event range and its sorted clicks.
    pub fn process_person<T>(
        &mut self,
        person: u32,
        mut events: &[T],
        decode: impl Fn(&T) -> Event,
        clicks: &[impl Borrow<Click>],
        attribution: &mut Attribution<'_>,
        campaign_days: &mut impl CampaignTotals,
        emit: &mut impl FnMut(u32, i32, i64, i64, i64, Option<u32>) -> Result<()>,
    ) -> Result<()> {
        attribution.reset();
        let mut day = decode(&events[0]).time.div_euclid(DAY_MS) as i32;
        while let Some(first) = events.first() {
            let start = decode(first).time;
            let session_day = start.div_euclid(DAY_MS) as i32;
            if session_day != day {
                self.write(emit, person, day)?;
                day = session_day;
            }
            let mut previous = start;
            let mut count = 0;
            let mut revenue = 0;
            for row in events {
                let event = decode(row);
                if event.time - previous > SESSION_GAP_MS {
                    break;
                }
                previous = event.time;
                count += 1;
                if event.price != 0 {
                    let price = i64::from(event.price);
                    revenue += price;
                    self.add_purchase(event.category, price);
                }
            }
            self.sessions += 1;
            self.events += count as i64;
            self.revenue += revenue;
            attribution.credit(clicks, start, day, revenue, campaign_days);
            events = &events[count..];
        }
        self.write(emit, person, day)
    }
}

pub fn process(
    mut emit: impl FnMut(u32, i32, i64, i64, i64, Option<u32>) -> Result<()>,
    events: &[PersonRow<Event>],
    mut clicks: &[PersonRow<Click>],
    attribution: &mut Attribution<'_>,
    campaign_days: &mut impl CampaignTotals,
) -> Result<()> {
    let mut daily = UserDay::default();
    for events in events.chunk_by(|a, b| a.person == b.person) {
        let person = events[0].person;
        if clicks.first().is_some_and(|row| row.person < person) {
            clicks = &clicks[clicks.partition_point(|row| row.person < person)..];
        }
        let end = if clicks.first().is_some_and(|row| row.person == person) {
            clicks.partition_point(|row| row.person <= person)
        } else {
            0
        };
        let (matched, rest) = clicks.split_at(end);
        clicks = rest;
        daily.process_person(
            person,
            events,
            |r| r.data,
            matched,
            attribution,
            campaign_days,
            &mut emit,
        )?;
    }
    Ok(())
}
