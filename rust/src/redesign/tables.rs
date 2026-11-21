use crate::buckets::{self, Partitions, USERS};
use crate::key::Key;
use memmap2::Mmap;
use parquet::errors::Result;
use rayon::prelude::*;
use shopping_sessions::aggregate::{self, Attribution, CampaignDays, Click, Event, PersonRow};
use shopping_sessions::catalog::{Campaigns, Products};
use shopping_sessions::fastpq::{CampaignDaily, NullableStrings, Strings, UserDaily};
use std::{fs, io};
use std::path::Path;

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

pub fn aggregate(
    names: &[Vec<Key>],
    events: Partitions<PersonRow<Event>>,
    clicks: Partitions<PersonRow<Click>>,
    chunks: &[Mmap],
    products: &Products,
    campaigns: &Campaigns,
) -> Result<(Vec<UserColumns>, CampaignColumns)> {
    let parallelism = rayon::current_num_threads().min(USERS);
    let size = USERS.div_ceil(parallelism);
    let parts: Vec<_> = events
        .into_par_iter()
        .zip(clicks)
        .enumerate()
        .chunks(size)
        .map(|parts| -> Result<_> {
            let mut daily = UserColumns::default();
            let mut totals = CampaignDays::new();
            let mut attribution = Attribution::new(chunks, campaigns.values.len());
            for (shard, (events, clicks)) in parts {
                if events.is_empty() {
                    continue;
                }
                let mut events = buckets::flatten(events);
                let mut clicks = buckets::flatten(clicks);
                events.sort_unstable_by_key(|row| (row.person, row.data.time));
                clicks.sort_unstable_by_key(|row| (row.person, row.data.time));
                aggregate::process(
                    |id, day, sessions, count, revenue, top| {
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
                    },
                    &events,
                    &clicks,
                    &mut attribution,
                    &mut totals,
                )?;
            }
            Ok((daily, totals))
        })
        .collect::<Result<_>>()?;
    let mut users = Vec::with_capacity(parts.len());
    let mut all = CampaignDays::new();
    for (daily, totals) in parts {
        users.push(daily);
        for (key, day) in totals {
            all.entry(key).or_default().merge(day);
        }
    }
    let mut campaign = CampaignColumns::default();
    for ((id, day), x) in all {
        let (name, channel) = &campaigns.values[id as usize];
        campaign.campaign_id.push(name);
        campaign.channel.push(channel);
        campaign.day.push(day);
        campaign.sessions.push(x.sessions);
        campaign.purchasing_sessions.push(x.purchasing);
        campaign.revenue_cents.push(x.revenue);
        campaign.assisted_sessions.push(x.assists);
    }
    Ok((users, campaign))
}

pub fn write(path: &Path, users: &[UserColumns], campaigns: &CampaignColumns) -> io::Result<()> {
    let user_dir = path.join("user_daily");
    let campaign_dir = path.join("campaign_daily");
    fs::create_dir_all(&user_dir)?;
    fs::create_dir_all(&campaign_dir)?;
    users.par_iter().enumerate().try_for_each(|(i, batch)| {
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
        .write(&user_dir.join(format!("part-{i}.parquet")))
    })?;
    CampaignDaily {
        campaign_id: campaigns.campaign_id.strings()?,
        channel: campaigns.channel.strings()?,
        day: &campaigns.day,
        sessions: &campaigns.sessions,
        purchasing_sessions: &campaigns.purchasing_sessions,
        revenue_cents: &campaigns.revenue_cents,
        assisted_sessions: &campaigns.assisted_sessions,
    }
    .write(&campaign_dir.join("part-0.parquet"))
}
