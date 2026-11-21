mod buckets;
mod calendar;
mod key;
mod links;
mod records;
mod scan;
mod tables;
mod timing;

use ahash::RandomState;
use shopping_sessions::catalog::{Campaigns, Products, load_window, path};
use shopping_sessions::csv::read_chunks;
use shopping_sessions::time::{LOOKBACK_MS, Window};
use std::error::Error;
use std::num::NonZeroUsize;
use timing::{Stage, Timings};

#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

// jemalloc reads this pointer before main. The NUL-terminated string lives
// for the entire process. The kernel may satisfy these hints with normal pages.
#[unsafe(export_name = "_rjem_malloc_conf")]
static MALLOC_CONF: Option<&std::ffi::c_char> =
    unsafe { c"thp:always,metadata_thp:always".as_ptr().as_ref() };

fn main() -> Result<(), Box<dyn Error>> {
    let mut timer = Timings::new();
    let input = path("DATA_DIR", "/data");
    let output = path("OUTPUT_DIR", "/output");
    let threads = match std::env::var("NCPU") {
        Ok(n) => n.parse::<NonZeroUsize>()?.get(),
        Err(_) => std::thread::available_parallelism()?.get(),
    };
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build_global()?;
    let hash = RandomState::new();
    timer.end(Stage::Setup);

    let window = load_window(&input.join("manifest.json"))?;
    let products = Products::read(&input.join("products.csv"))?;
    let campaigns = Campaigns::read(&input.join("campaigns.csv"))?;
    const CHUNK: usize = 8 * 1024 * 1024;
    let mut chunks = read_chunks(&input.join("links"), CHUNK)?;
    let links_end = chunks.len();
    chunks.extend(read_chunks(&input.join("events"), CHUNK)?);
    let events_end = chunks.len();
    chunks.extend(read_chunks(&input.join("clicks"), CHUNK)?);
    let link_batch = links::parse(&chunks, 0..links_end, &hash);
    timer.end(Stage::Parse);
    let directory = links::build(link_batch, &chunks, &hash);
    timer.end(Stage::Links);

    // Retiring the event dedup index before building the click index also
    // bounds peak working memory on the 60M workload.
    let event_index =
        records::parse_events(&chunks, links_end..events_end, &hash, window, &products);
    timer.end(Stage::Parse);
    let events = records::dedup(event_index, &chunks);
    timer.end(Stage::Dedup);
    let clicks_window = Window {
        start: window.start - LOOKBACK_MS,
        end: window.end,
    };
    let click_index = records::parse_clicks(
        &chunks,
        events_end..chunks.len(),
        &hash,
        clicks_window,
        &campaigns,
    );
    timer.end(Stage::Parse);
    let clicks = records::dedup(click_index, &chunks);
    timer.end(Stage::Dedup);

    let people = links::resolve(directory, events, clicks, &hash, &chunks);
    timer.end(Stage::Resolve);
    let (users, daily) = tables::aggregate(
        &people.names,
        people.events,
        people.clicks,
        &chunks,
        &products,
        &campaigns,
        window,
    )?;
    timer.end(Stage::Aggregate);
    // The output columns own their strings and numbers. After aggregation
    // no output writer can refer to the input mappings.
    let (_, written) = rayon::join(|| drop(chunks), || tables::write(&output, &users, &daily));
    written?;
    timer.end(Stage::Write);
    timer.print();
    // All writers have closed. The OS reclaims the remaining workspaces at exit.
    std::process::exit(0);
}
