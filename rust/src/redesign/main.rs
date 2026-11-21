mod buckets;
mod calendar;
mod dims;
mod key;
mod links;
mod records;
mod scan;
mod tables;
mod timing;

use ahash::RandomState;
use memmap2::UncheckedAdvice;
use shopping_sessions::catalog::{Campaigns, Products, load_window, path};
use shopping_sessions::csv::read_chunks;
use shopping_sessions::time::{LOOKBACK_MS, Window};
use memmap2::Mmap;
use std::error::Error;
use std::path::Path;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
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
    let directory = links::build(link_batch, &chunks);
    timer.end(Stage::Links);

    let inputs = Inputs {
        chunks: &chunks,
        links_end,
        events_end,
        hash: &hash,
        window,
        products: &products,
        campaigns: &campaigns,
        output: &output,
    };
    // Events travel as one sortable word whenever the window and catalog fit.
    if records::PackedEvent::fits(window, products.categories.len()) {
        run::<records::PackedEvent>(inputs, directory, &mut timer)?;
    } else {
        run::<shopping_sessions::aggregate::Event>(inputs, directory, &mut timer)?;
    }
    timer.print();
    // All writers have closed. The OS reclaims the remaining workspaces at exit.
    std::process::exit(0);
}

struct Inputs<'a> {
    chunks: &'a [Mmap],
    links_end: usize,
    events_end: usize,
    hash: &'a RandomState,
    window: Window,
    products: &'a Products,
    campaigns: &'a Campaigns,
    output: &'a Path,
}

fn run<E: records::EventCodec + tables::EventRows>(
    inputs: Inputs<'_>,
    directory: Vec<links::Directory>,
    timer: &mut Timings,
) -> Result<(), Box<dyn Error>> {
    let Inputs {
        chunks,
        links_end,
        events_end,
        hash,
        window,
        products,
        campaigns,
        output,
    } = inputs;
    // Retiring the event dedup index before building the click index also
    // bounds peak working memory on the 60M workload.
    let event_index =
        records::parse_events::<E>(chunks, links_end..events_end, hash, window, products);
    timer.end(Stage::Parse);
    let events = records::dedup(event_index, hash, chunks);
    timer.end(Stage::Dedup);
    let clicks_window = Window {
        start: window.start - LOOKBACK_MS,
        end: window.end,
    };
    let click_index = records::parse_clicks(
        chunks,
        events_end..chunks.len(),
        hash,
        clicks_window,
        campaigns,
    );
    timer.end(Stage::Parse);
    let clicks = records::dedup(click_index, hash, chunks);
    timer.end(Stage::Dedup);

    let people = links::resolve(directory, events, clicks, hash, chunks);
    timer.end(Stage::Resolve);
    let user_dir = output.join("user_daily");
    let campaign_dir = output.join("campaign_daily");
    std::fs::create_dir_all(&user_dir)?;
    std::fs::create_dir_all(&campaign_dir)?;
    // Output columns own their strings and numbers, so writers never read the
    // inputs. Aggregation rarely does: long names and tied click IDs. Workers
    // that finish early discard input page-table entries while others still
    // aggregate. A later read faults the same immutable file bytes back in.
    // The address-space lock is shared, so workers clear separate mappings
    // concurrently, and exit only removes empty mappings.
    let next_chunk = AtomicUsize::new(0);
    let release = || {
        loop {
            let i = next_chunk.fetch_add(1, Ordering::Relaxed);
            let Some(chunk) = chunks.get(i) else { break };
            let _ = unsafe { chunk.unchecked_advise(UncheckedAdvice::DontNeed) };
        }
    };
    let daily = tables::aggregate(
        &people.names,
        people.events,
        people.clicks,
        chunks,
        products,
        campaigns,
        window,
        &user_dir,
        &release,
    )?;
    timer.end(Stage::Aggregate);
    tables::write_campaigns(&campaign_dir.join("part-0.parquet"), &daily)?;
    timer.end(Stage::Write);
    Ok(())
}
