use memchr::{memchr, memchr_iter};
use memmap2::{Mmap, MmapOptions};
use std::fs::{self, File};
use std::io;
use std::path::Path;

pub fn csv<const N: usize>(line: &[u8]) -> Option<[&[u8]; N]> {
    #[cfg(target_arch = "x86_64")]
    if (16..=128).contains(&line.len()) {
        return csv_with_masks(line);
    }
    csv_with_search(line)
}

#[cfg(target_arch = "x86_64")]
fn csv_with_masks<const N: usize>(line: &[u8]) -> Option<[&[u8]; N]> {
    use std::arch::x86_64::{
        __m128i, _mm_cmpeq_epi8, _mm_loadu_si128, _mm_movemask_epi8, _mm_set1_epi8,
    };

    let (mut commas, mut quotes) = (0u128, 0u128);
    // x86-64 always has SSE2. Full vectors and the possibly overlapping
    // final vector both lie entirely within the current line.
    unsafe {
        let comma = _mm_set1_epi8(b',' as i8);
        let quote = _mm_set1_epi8(b'"' as i8);
        let mut scan = |start: usize| {
            let bytes = _mm_loadu_si128(line.as_ptr().add(start).cast::<__m128i>());
            commas |= (_mm_movemask_epi8(_mm_cmpeq_epi8(bytes, comma)) as u128) << start;
            quotes |= (_mm_movemask_epi8(_mm_cmpeq_epi8(bytes, quote)) as u128) << start;
        };
        for start in (0..line.len() / 16 * 16).step_by(16) {
            scan(start);
        }
        if line.len() % 16 != 0 {
            scan(line.len() - 16);
        }
    }
    if commas.count_ones() as usize != N - 1 {
        return None;
    }
    let mut result = [&[][..]; N];
    let mut start = 0;
    for (index, field) in result.iter_mut().enumerate() {
        let end = if index + 1 == N {
            line.len()
        } else {
            let end = commas.trailing_zeros() as usize;
            commas &= commas - 1;
            end
        };
        let bytes = &line[start..end];
        *field = if bytes.first() == Some(&b'"') {
            if bytes.len() < 2 || bytes.last() != Some(&b'"') {
                return None;
            }
            // Consume only each field's permitted pair. Any other quote,
            // including one in an unquoted field, remains after the loop.
            quotes &= !((1u128 << start) | (1u128 << (end - 1)));
            &bytes[1..bytes.len() - 1]
        } else {
            bytes
        };
        start = end + 1;
    }
    (quotes == 0).then_some(result)
}

fn csv_with_search<const N: usize>(line: &[u8]) -> Option<[&[u8]; N]> {
    let mut commas = memchr_iter(b',', line);
    let mut start = 0;
    let mut result = [&[][..]; N];
    for (index, field) in result.iter_mut().enumerate() {
        let end = if index + 1 == N {
            if commas.next().is_some() {
                return None;
            }
            line.len()
        } else {
            commas.next()?
        };
        let bytes = &line[start..end];
        let bytes = if let Some(quoted) = bytes.strip_prefix(b"\"") {
            quoted.strip_suffix(b"\"")?
        } else {
            bytes
        };
        if memchr(b'"', bytes).is_some() {
            return None;
        }
        *field = bytes;
        start = end + 1;
    }
    Some(result)
}

pub fn lines(data: &[u8]) -> impl Iterator<Item = (u32, &[u8])> {
    let mut start = 0;
    memchr_iter(b'\n', data)
        .chain(std::iter::once(data.len()))
        .filter_map(move |end| {
            let offset = start;
            start = end + 1;
            (offset < end).then(|| (u32::try_from(offset).unwrap(), &data[offset..end]))
        })
}

pub fn after_header(data: &[u8]) -> &[u8] {
    memchr(b'\n', data).map_or(&[], |end| &data[end + 1..])
}

pub fn read_chunks(directory: &Path, chunk_bytes: usize) -> io::Result<Vec<Mmap>> {
    let mut paths = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<io::Result<Vec<_>>>()?;
    paths.retain(|path| path.as_os_str().as_encoded_bytes().ends_with(b".csv"));
    paths.sort_unstable();

    let mut chunks = Vec::new();
    for path in paths {
        let file = File::open(path)?;
        if file.metadata()?.len() == 0 {
            continue;
        }
        // The input is immutable for the entire run. Each chunk owns a separate
        // read-only mapping so it can be unmapped as soon as its rows are parsed.
        let file_bytes = unsafe { Mmap::map(&file)? };
        let mut start = file_bytes.len() - after_header(&file_bytes).len();
        while start < file_bytes.len() {
            let target = (start + chunk_bytes).min(file_bytes.len());
            let end = if target == file_bytes.len() {
                target
            } else {
                memchr(b'\n', &file_bytes[target - 1..])
                    .map_or(file_bytes.len(), |next| target + next)
            };
            let chunk = unsafe {
                MmapOptions::new()
                    .offset(start as u64)
                    .len(end - start)
                    .map(&file)?
            };
            chunks.push(chunk);
            start = end;
        }
    }
    Ok(chunks)
}

#[derive(Clone, Copy)]
pub struct Location {
    pub chunk: u32,
    pub offset: u32,
}

impl Location {
    pub fn new(chunk: usize, offset: u32) -> Self {
        Self {
            chunk: u32::try_from(chunk).unwrap(),
            offset,
        }
    }

    pub fn row(self, chunks: &[Mmap]) -> &[u8] {
        let tail = &chunks[self.chunk as usize][self.offset as usize..];
        &tail[..memchr(b'\n', tail).unwrap_or(tail.len())]
    }

    pub fn id(self, chunks: &[Mmap]) -> &[u8] {
        let tail = &chunks[self.chunk as usize][self.offset as usize..];
        // Called only for records that passed CSV parsing, so the first
        // comma is on this line and any opening quote has a closing quote.
        let field = &tail[..memchr(b',', tail).unwrap()];
        if field[0] == b'"' {
            &field[1..field.len() - 1]
        } else {
            field
        }
    }
}
