use memchr::memchr;
use memmap2::{Mmap, MmapOptions};
use std::fs::{self, File};
use std::io;
use std::path::Path;

/// A chunk owns the rows that start inside one fixed byte range of a file.
/// Planning reads only file lengths, so a cold disk is not touched until the
/// parse workers and the prefetcher read the data itself.
#[derive(Clone, Copy)]
pub struct Span {
    /// Length of the owned range. It ranks parse jobs and sizes buffers.
    pub nominal: usize,
    /// The range starts the file. Its mapping begins with the header line;
    /// a later range's mapping begins one byte before the range.
    file_start: bool,
}

impl Span {
    /// Mapping offsets of the first owned row start and of the end of the
    /// owned range. The last owned row may continue past that end, and the
    /// mapping runs to the end of the file so it can be completed.
    pub fn rows(&self, chunk: &[u8]) -> (usize, usize) {
        let until = usize::from(!self.file_start) + self.nominal;
        // The header line, or the end of a row owned by an earlier range,
        // finishes at the first newline. Only the range itself is searched,
        // so a row spanning many ranges is scanned once, by its owner.
        let from = memchr(b'\n', &chunk[..until.min(chunk.len())]).map_or(until, |end| end + 1);
        (from, until)
    }
}

/// Map fixed `chunk_bytes` ranges of every nonempty CSV file in `directory`,
/// in sorted path order, without reading their contents.
pub fn plan(
    directory: &Path,
    chunk_bytes: usize,
    chunks: &mut Vec<Mmap>,
    spans: &mut Vec<Span>,
) -> io::Result<()> {
    let mut paths = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<io::Result<Vec<_>>>()?;
    paths.retain(|path| path.as_os_str().as_encoded_bytes().ends_with(b".csv"));
    paths.sort_unstable();
    for path in paths {
        let file = File::open(path)?;
        let length = usize::try_from(file.metadata()?.len()).unwrap();
        let mut start = 0;
        while start < length {
            let map_start = start.saturating_sub(1);
            // The file is immutable for the whole run.
            let map = unsafe {
                MmapOptions::new()
                    .offset(map_start as u64)
                    .len(length - map_start)
                    .map(&file)?
            };
            chunks.push(map);
            spans.push(Span {
                nominal: chunk_bytes.min(length - start),
                file_start: start == 0,
            });
            start += chunk_bytes;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::rows;
    use shopping_sessions::csv::{after_header, csv, lines};

    /// Every split of a file into ranges yields exactly the rows, with the
    /// same absolute offsets, as the reference reader of the whole file.
    #[test]
    fn ranges_partition_rows_exactly() {
        let dir = std::env::temp_dir().join(format!("shop-chunks-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut body = b"a,b,c\n".to_vec();
        for i in 0..300 {
            let row: Vec<u8> = match i % 7 {
                0 => b"\"x\",,\"\"".to_vec(),
                1 => b"\xcf\x87,long,".iter().copied().chain(std::iter::repeat_n(b'z', 90)).collect(),
                2 => b"".to_vec(),
                3 => b"x\r,y,z\r".to_vec(),
                4 => b"bad\"quote,1,2".to_vec(),
                5 => std::iter::repeat_n(b'q', 700).chain(*b",1,2").collect(),
                _ => format!("{i},u{i},{}", i * 3).into_bytes(),
            };
            body.extend_from_slice(&row);
            body.push(b'\n');
        }
        body.extend_from_slice(b"last,unterminated,row");
        let headers: [&[u8]; 3] = [b"a,b,c\n", b"\n", b"h"];
        for (h, header) in headers.iter().enumerate() {
            let mut file = header.to_vec();
            file.extend_from_slice(&body[6..]);
            let path = dir.join(format!("f{h}.csv"));
            fs::write(&path, &file).unwrap();
            let expected: Vec<_> = lines(after_header(&file))
                .filter_map(|(offset, line)| csv::<3>(line).map(|f| (offset, f)))
                .map(|(offset, f)| {
                    let base = file.len() - after_header(&file).len();
                    (base + offset as usize, f.map(<[u8]>::to_vec))
                })
                .collect();
            for chunk_bytes in [1, 2, 3, 7, 63, 64, 127, 128, 129, 700, 701, 4096, 1 << 20] {
                let (mut chunks, mut spans) = (Vec::new(), Vec::new());
                let sub = dir.join(format!("d{h}-{chunk_bytes}"));
                fs::create_dir_all(&sub).unwrap();
                fs::copy(&path, sub.join("only.csv")).unwrap();
                fs::write(sub.join("empty.csv"), b"").unwrap();
                plan(&sub, chunk_bytes, &mut chunks, &mut spans).unwrap();
                let mut seen = Vec::new();
                let mut start = 0usize;
                for (chunk, span) in chunks.iter().zip(&spans) {
                    let map_start = start.saturating_sub(1);
                    let (from, until) = span.rows(chunk);
                    for (offset, f) in rows::<3>(chunk, from, until) {
                        seen.push((map_start + offset as usize, f.map(<[u8]>::to_vec)));
                    }
                    start += chunk_bytes;
                }
                assert_eq!(seen, expected, "header {h} chunk {chunk_bytes}");
            }
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
