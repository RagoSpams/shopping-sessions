use std::fs::{self, File};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

// Larger requests are truncated by the kernel's per-call readahead limit.
const STEP: usize = 1 << 20;

unsafe extern "C" {
    fn readahead(fd: i32, offset: i64, count: usize) -> isize;
}

fn csv_files(directory: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.as_os_str().as_encoded_bytes().ends_with(b".csv"))
        .collect();
    paths.sort_unstable();
    paths
}

/// Start reading every input file into the page cache, in the order the
/// pipeline consumes them, from a thread that only waits on the disk. Parse
/// workers that reach a page still in flight wait for that same read instead
/// of issuing small reads of their own. Cached files cost one lookup per
/// megabyte. Failures leave ordinary page faults to read the data.
pub fn start(input: &Path) {
    let mut paths = vec![input.join("products.csv"), input.join("campaigns.csv")];
    for directory in ["links", "events", "clicks"] {
        paths.extend(csv_files(&input.join(directory)));
    }
    std::thread::spawn(move || {
        for path in paths {
            let Ok(file) = File::open(&path) else { continue };
            let Ok(metadata) = file.metadata() else { continue };
            let length = metadata.len() as usize;
            let mut offset = 0;
            while offset < length {
                // The descriptor stays open for the call. readahead only fills
                // the page cache and never changes the file.
                if unsafe { readahead(file.as_raw_fd(), offset as i64, STEP) } < 0 {
                    break;
                }
                offset += STEP;
            }
        }
    });
}
