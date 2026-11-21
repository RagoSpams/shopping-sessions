use crate::csv::Location;
use crate::output::CampaignDay;
use crate::time::{DAY_MS, LOOKBACK_MS, SESSION_GAP_MS};
use ahash::AHashMap;
use memmap2::Mmap;
use parquet::errors::Result;

pub struct PersonRow<T> {
    pub person: u32,
    pub data: T,
}

#[derive(Clone, Copy)]
pub struct Event {
    pub time: i64,
    pub price: u32,
    pub category: u32,
}

#[derive(Clone, Copy)]
pub struct Click {
    pub time: i64,
    pub campaign: u32,
    pub location: Location,
}

pub type CampaignDays = AHashMap<(u32, i32), CampaignDay>;

pub trait CampaignTotals {
    fn daily(&mut self, campaign: u32, day: i32) -> &mut CampaignDay;
}

impl CampaignTotals for CampaignDays {
    #[inline]
    fn daily(&mut self, campaign: u32, day: i32) -> &mut CampaignDay {
        self.entry((campaign, day)).or_default()
    }
}

#[derive(Default)]
struct ActiveCampaign {
    count: usize,
    position: usize,
}

pub struct Attribution<'a> {
    chunks: &'a [Mmap],
    campaigns: Vec<ActiveCampaign>,
    active: Vec<u32>,
    left: usize,
    right: usize,
    latest: Option<Click>,
}

impl<'a> Attribution<'a> {
    pub fn new(chunks: &'a [Mmap], campaign_count: usize) -> Self {
        Self {
            chunks,
            campaigns: (0..campaign_count).map(|_| ActiveCampaign::default()).collect(),
            active: Vec::new(),
            left: 0,
            right: 0,
            latest: None,
        }
    }

    fn reset(&mut self) {
        for campaign in self.active.drain(..) {
            self.campaigns[campaign as usize].count = 0;
        }
        self.left = 0;
        self.right = 0;
        self.latest = None;
    }

    fn advance(&mut self, start: i64, clicks: &[PersonRow<Click>]) {
        while let Some(row) = clicks.get(self.right) {
            let click = row.data;
            if click.time > start {
                break;
            }
            if self.latest.is_none_or(|latest| {
                click.time > latest.time
                    || click.location.id(self.chunks) < latest.location.id(self.chunks)
            }) {
                self.latest = Some(click);
            }
            let slot = &mut self.campaigns[click.campaign as usize];
            if slot.count == 0 {
                slot.position = self.active.len();
                self.active.push(click.campaign);
            }
            slot.count += 1;
            self.right += 1;
        }
        while self.left < self.right && clicks[self.left].data.time < start - LOOKBACK_MS {
            let campaign = clicks[self.left].data.campaign;
            let slot = &mut self.campaigns[campaign as usize];
            slot.count -= 1;
            if slot.count == 0 {
                let position = slot.position;
                self.active.swap_remove(position);
                if let Some(&swapped) = self.active.get(position) {
                    self.campaigns[swapped as usize].position = position;
                }
            }
            self.left += 1;
        }
    }

    fn credit(
        &mut self,
        clicks: &[PersonRow<Click>],
        start: i64,
        day: i32,
        revenue: i64,
        campaign_days: &mut impl CampaignTotals,
    ) {
        self.advance(start, clicks);
        let campaign = if self.active.is_empty() {
            0 // Organic is reserved at index 0.
        } else {
            self.latest.unwrap().campaign
        };
        let totals = campaign_days.daily(campaign, day);
        totals.sessions += 1;
        totals.purchasing += i64::from(revenue > 0);
        totals.revenue += revenue;
        for &campaign in &self.active {
            campaign_days.daily(campaign, day).assists += 1;
        }
    }
}

#[derive(Default)]
struct UserDay {
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
}

pub fn process(
    mut emit: impl FnMut(u32, i32, i64, i64, i64, Option<u32>) -> Result<()>,
    events: &[PersonRow<Event>],
    mut clicks: &[PersonRow<Click>],
    attribution: &mut Attribution<'_>,
    campaign_days: &mut impl CampaignTotals,
) -> Result<()> {
    let mut daily = UserDay::default();
    for mut events in events.chunk_by(|a, b| a.person == b.person) {
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
        attribution.reset();
        let mut day = events[0].data.time.div_euclid(DAY_MS) as i32;
        while let Some(first) = events.first() {
            let start = first.data.time;
            let session_day = start.div_euclid(DAY_MS) as i32;
            if session_day != day {
                daily.write(&mut emit, person, day)?;
                day = session_day;
            }
            let mut previous = start;
            let mut count = 0;
            let mut revenue = 0;
            for row in events {
                let event = &row.data;
                if event.time - previous > SESSION_GAP_MS {
                    break;
                }
                previous = event.time;
                count += 1;
                if event.price != 0 {
                    let price = i64::from(event.price);
                    revenue += price;
                    daily.add_purchase(event.category, price);
                }
            }
            daily.sessions += 1;
            daily.events += count as i64;
            daily.revenue += revenue;
            attribution.credit(matched, start, day, revenue, campaign_days);
            events = &events[count..];
        }
        daily.write(&mut emit, person, day)?;
    }
    Ok(())
}
