use crate::buckets::{self, Partitions, USERS};
use crate::key::Key;
use memmap2::Mmap;
use parquet::errors::Result;
use rayon::prelude::*;
use shopping_sessions::aggregate::{self, Attribution, CampaignDays, Click, Event, PersonRow};
use shopping_sessions::catalog::{Campaigns, Products};
use shopping_sessions::output::{self, CampaignDay, UserWriter};
use std::fs;
use std::path::Path;

#[derive(Default)]
pub struct UserColumns<'a> {
    pub user_id: Vec<&'a [u8]>,
    pub day: Vec<i32>,
    pub sessions: Vec<i64>,
    pub events: Vec<i64>,
    pub revenue_cents: Vec<i64>,
    pub top_category: Vec<Option<&'a [u8]>>,
}

#[derive(Default)]
pub struct CampaignColumns<'a> {
    pub campaign_id: Vec<&'a [u8]>,
    pub channel: Vec<&'a [u8]>,
    pub day: Vec<i32>,
    pub sessions: Vec<i64>,
    pub purchasing_sessions: Vec<i64>,
    pub revenue_cents: Vec<i64>,
    pub assisted_sessions: Vec<i64>,
}

pub fn aggregate<'a>(
    names: &'a [Vec<Key>],
    events: Partitions<PersonRow<Event>>,
    clicks: Partitions<PersonRow<Click>>,
    chunks: &'a [Mmap],
    products: &'a Products,
    campaigns: &'a Campaigns,
) -> Result<(Vec<UserColumns<'a>>, CampaignColumns<'a>)> {
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
                        daily
                            .top_category
                            .push(top.map(|id| products.categories[id as usize].as_ref()));
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

// Adapter for the crate writer. The producer's columns can also be handed
// directly to a fixed-schema writer without revisiting the session pipeline.
pub fn write(
    path: &Path,
    users: &[UserColumns<'_>],
    campaigns: &CampaignColumns<'_>,
) -> Result<()> {
    let user_dir = path.join("user_daily");
    let campaign_dir = path.join("campaign_daily");
    fs::create_dir_all(&user_dir)?;
    fs::create_dir_all(&campaign_dir)?;
    users
        .par_iter()
        .enumerate()
        .try_for_each(|(i, batch)| -> Result<()> {
            let mut writer =
                UserWriter::with_encoding(&user_dir.join(format!("part-{i}.parquet")), true)?;
            for row in 0..batch.day.len() {
                writer.push(
                    batch.user_id[row],
                    batch.day[row],
                    batch.sessions[row],
                    batch.events[row],
                    batch.revenue_cents[row],
                    batch.top_category[row],
                )?;
            }
            writer.finish()
        })?;
    output::campaign_file(
        &campaign_dir.join("part-0.parquet"),
        (0..campaigns.day.len()).map(|i| {
            (
                campaigns.campaign_id[i],
                campaigns.channel[i],
                campaigns.day[i],
                CampaignDay {
                    sessions: campaigns.sessions[i],
                    purchasing: campaigns.purchasing_sessions[i],
                    revenue: campaigns.revenue_cents[i],
                    assists: campaigns.assisted_sessions[i],
                },
            )
        }),
        true,
    )
}
