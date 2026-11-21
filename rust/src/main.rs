mod aggregate;
mod input;
mod output;
mod people;
mod time;

use aggregate::{Attribution, CampaignDays, Click, Event};
use ahash::AHashMap;
use input::{SHARDS, after_header, csv, duplicate_rows, lines, parse_winners, read_parts, transpose};
use output::{UserWriter, write_campaigns};
use people::{Batch, Names, People};
use rayon::prelude::*;
use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;
use time::{DAY_MS, LOOKBACK_MS, MAX_PRICE_CENTS, Window, bounded_uint, calendar_ms};

const OUTPUT_PARTS: usize = 32;
const UNKNOWN: &[u8] = b"unknown";
const ORGANIC: &[u8] = b"organic";

type Strings = AHashMap<Box<[u8]>, Box<[u8]>>;

fn read_dimension(path: &Path) -> io::Result<Strings> {
    let contents = fs::read(path)?;
    Ok(lines(after_header(&contents))
        .filter_map(|(_, row)| csv::<2>(row))
        .map(|[key, value]| (key.into(), value.into()))
        .collect())
}

struct Products {
    ids: AHashMap<Box<[u8]>, u32>,
    categories: Vec<Box<[u8]>>,
    unknown: u32,
}

impl Products {
    fn read(path: &Path) -> io::Result<Self> {
        let products = read_dimension(path)?;
        let mut categories: Vec<_> = products.values().cloned().collect();
        categories.push(UNKNOWN.into());
        categories.sort_unstable();
        categories.dedup();
        let category = |name: &[u8]| {
            let index = categories
                .binary_search_by(|label| label.as_ref().cmp(name))
                .unwrap();
            u32::try_from(index).unwrap()
        };
        let ids = products
            .into_iter()
            .map(|(product, label)| (product, category(&label)))
            .collect();
        let unknown = category(UNKNOWN);
        Ok(Self {
            ids,
            categories,
            unknown,
        })
    }
}

struct Campaigns {
    ids: AHashMap<Box<[u8]>, u32>,
    values: Vec<(Box<[u8]>, Box<[u8]>)>,
}

impl Campaigns {
    fn read(path: &Path) -> io::Result<Self> {
        let values = read_dimension(path)?;
        let mut ids = AHashMap::with_capacity(values.len());
        let mut campaigns = vec![(ORGANIC.into(), ORGANIC.into())];
        for (campaign, channel) in values {
            ids.insert(campaign.clone(), u32::try_from(campaigns.len()).unwrap());
            campaigns.push((campaign, channel));
        }
        Ok(Self {
            ids,
            values: campaigns,
        })
    }
}

fn event(
    [user, timestamp, ingest, kind, product, price]: [&[u8]; 6],
    window: Window,
    products: &Products,
) -> Option<Event> {
    if user.is_empty() || product.is_empty() {
        return None;
    }
    let (price, category) = match kind {
        b"purchase" => {
            let price = bounded_uint(price, MAX_PRICE_CENTS)?;
            if price == 0 {
                return None;
            }
            let category = products
                .ids
                .get(product)
                .copied()
                .unwrap_or(products.unknown);
            (price as u32, category)
        }
        b"cart" | b"view" => (0, 0),
        _ => return None,
    };
    let time = window.timestamp(timestamp)?;
    let ingest = calendar_ms(ingest)?;
    (ingest - time <= DAY_MS).then_some(Event {
        time,
        price,
        category,
    })
}

fn path(variable: &str, default: &str) -> PathBuf {
    env::var_os(variable).map_or_else(|| PathBuf::from(default), PathBuf::from)
}

fn load_window(path: &Path) -> Result<Window, Box<dyn Error>> {
    let manifest: serde_json::Value = serde_json::from_reader(fs::File::open(path)?)?;
    let timestamp = |key: &str| {
        manifest[key]
            .as_str()
            .and_then(|text| calendar_ms(text.as_bytes()))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("invalid {key}")))
    };
    Ok(Window {
        start: timestamp("window_start")?,
        end: timestamp("window_end")?,
    })
}

struct Phases(Option<Instant>);

impl Phases {
    fn new() -> Self {
        Self((env::var("SHOP_TIMING").as_deref() == Ok("1")).then(Instant::now))
    }

    fn record(&mut self, phase: &str) {
        if let Some(previous) = &mut self.0 {
            let elapsed = previous.elapsed();
            *previous = Instant::now();
            eprintln!("{phase}: {:.1} ms", elapsed.as_secs_f64() * 1_000.0);
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut phases = Phases::new();
    let data = path("DATA_DIR", "/data");
    let output = path("OUTPUT_DIR", "/output");
    let threads = match env::var("NCPU") {
        Ok(value) => value.parse::<std::num::NonZeroUsize>()?.get(),
        Err(_) => std::thread::available_parallelism()?.get(),
    };
    rayon::ThreadPoolBuilder::new().num_threads(threads).build_global()?;
    let window = load_window(&data.join("manifest.json"))?;
    let people = People::read(&data.join("links"))?;
    let products = Products::read(&data.join("products.csv"))?;
    let campaigns = Campaigns::read(&data.join("campaigns.csv"))?;
    phases.record("links and dimensions");

    let event_chunks = read_parts(&data.join("events"))?;
    let event_losers = duplicate_rows::<7>(&event_chunks, true);
    phases.record("event index and deduplication");
    let event_batches = parse_winners::<7, Batch<Event>, _, _>(
        event_chunks,
        event_losers,
        |batch, _, [_, user, timestamp, ingest, kind, product, price]| {
            let fields = [user, timestamp, ingest, kind, product, price];
            if let Some(row) = event(fields, window, &products) {
                batch.push(&people, user, row);
            }
        },
    );
    let events = transpose(event_batches.into_iter().map(|batch| batch.0).collect());
    phases.record("event validation and person partitioning");

    let click_chunks = read_parts(&data.join("clicks"))?;
    let click_losers = duplicate_rows::<4>(&click_chunks, false);
    let click_window = Window {
        start: window.start - LOOKBACK_MS,
        end: window.end,
    };
    let click_batches = parse_winners::<4, Batch<Click>, _, _>(
        click_chunks.iter().map(|chunk| &chunk[..]).collect(),
        click_losers,
        |batch, location, [_, user, campaign, timestamp]| {
            if !user.is_empty()
                && let Some(&campaign) = campaigns.ids.get(campaign)
                && let Some(time) = click_window.timestamp(timestamp)
            {
                batch.push(&people, user, Click { time, campaign, location });
            }
        },
    );
    let clicks = transpose(click_batches.into_iter().map(|batch| batch.0).collect());
    phases.record("click deduplication, validation and person partitioning");

    let user_dir = output.join("user_daily");
    let campaign_dir = output.join("campaign_daily");
    fs::create_dir_all(&user_dir)?;
    fs::create_dir_all(&campaign_dir)?;
    let mut shards = people.names.into_iter().zip(events).zip(clicks);
    let groups: Vec<_> = (0..OUTPUT_PARTS)
        .map(|_| shards.by_ref().take(SHARDS / OUTPUT_PARTS).collect::<Vec<_>>())
        .collect();
    let campaign_days: Vec<_> = groups
        .into_par_iter()
        .enumerate()
        .map(|(part, shards)| -> parquet::errors::Result<_> {
            let path = user_dir.join(format!("part-{part}.parquet"));
            let mut writer = if part == 0 {
                Some(UserWriter::new(&path)?)
            } else {
                None
            };
            let mut totals = CampaignDays::new();
            let mut attribution = Attribution::new(&click_chunks, campaigns.values.len());
            for ((names, events), clicks) in shards {
                let mut names = Names::new(names);
                let mut events = names.merge(events);
                if events.is_empty() {
                    continue;
                }
                let mut clicks = names.merge(clicks);
                events.sort_unstable_by_key(|row| (row.person, row.data.time));
                clicks.sort_unstable_by_key(|row| (row.person, row.data.time));
                if writer.is_none() {
                    writer = Some(UserWriter::new(&path)?);
                }
                aggregate::process(
                    writer.as_mut().unwrap(),
                    &names.strings,
                    &products.categories,
                    &events,
                    &clicks,
                    &mut attribution,
                    &mut totals,
                )?;
            }
            if let Some(writer) = writer {
                writer.finish()?;
            } else if let Err(error) = fs::remove_file(path)
                && error.kind() != io::ErrorKind::NotFound
            {
                return Err(error.into());
            }
            Ok(totals)
        })
        .collect::<Result<_, _>>()?;
    phases.record("person sort, aggregation and user parquet");

    let mut totals = CampaignDays::new();
    for days in campaign_days {
        for (key, day) in days {
            totals.entry(key).or_default().merge(day);
        }
    }
    write_campaigns(
        &campaign_dir.join("part-0.parquet"),
        totals.into_iter().map(|((campaign, day), totals)| {
            let (id, channel) = &campaigns.values[campaign as usize];
            (id.as_ref(), channel.as_ref(), day, totals)
        }),
    )?;
    phases.record("campaign reduction and parquet");
    Ok(())
}
