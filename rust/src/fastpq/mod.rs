//! Uncompressed Parquet data page v1 writers for the two output tables.

mod metadata;
#[cfg(test)]
mod tests;

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

const MAGIC: &[u8; 4] = b"PAR1";
const PAGE_ROWS: usize = 256 * 1024;
const PAGE_BYTES: usize = 8 * 1024 * 1024;

/// One UTF-8 byte offset per row, plus the end offset. Empty input uses `[0]`.
/// Offsets may start anywhere in `data`, allowing slices of a larger column.
#[derive(Clone, Copy)]
pub struct Strings<'a> {
    pub offsets: &'a [usize],
    pub data: &'a str,
}

/// Dictionary-encoded strings: one index per row into `values`. With a
/// nullable column, `NULL` marks a missing value.
#[derive(Clone, Copy)]
pub struct Dictionary<'a> {
    pub values: Strings<'a>,
    pub indices: &'a [u32],
}

impl Dictionary<'_> {
    pub const NULL: u32 = u32::MAX;
}

pub struct UserDaily<'a> {
    pub user_id: Dictionary<'a>,
    /// Days since 1970-01-01.
    pub day: &'a [i32],
    pub sessions: &'a [i64],
    pub events: &'a [i64],
    pub revenue_cents: &'a [i64],
    pub top_category: Dictionary<'a>,
}

pub struct CampaignDaily<'a> {
    pub campaign_id: Strings<'a>,
    pub channel: Strings<'a>,
    /// Days since 1970-01-01.
    pub day: &'a [i32],
    pub sessions: &'a [i64],
    pub purchasing_sessions: &'a [i64],
    pub revenue_cents: &'a [i64],
    pub assisted_sessions: &'a [i64],
}

impl UserDaily<'_> {
    /// Creates or truncates `path` and writes the full Parquet file.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        write_table(
            path,
            "user_daily",
            self.day.len(),
            &[
                ("user_id", Column::Dict(self.user_id, false)),
                ("day", Column::Date(self.day)),
                ("sessions", Column::int(self.sessions)),
                ("events", Column::int(self.events)),
                ("revenue_cents", Column::int(self.revenue_cents)),
                ("top_category", Column::Dict(self.top_category, true)),
            ],
        )
    }
}

impl CampaignDaily<'_> {
    /// Creates or truncates `path` and writes the full Parquet file.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        write_table(
            path,
            "campaign_daily",
            self.day.len(),
            &[
                ("campaign_id", Column::String(self.campaign_id)),
                ("channel", Column::String(self.channel)),
                ("day", Column::Date(self.day)),
                ("sessions", Column::int(self.sessions)),
                ("purchasing_sessions", Column::int(self.purchasing_sessions)),
                ("revenue_cents", Column::int(self.revenue_cents)),
                ("assisted_sessions", Column::int(self.assisted_sessions)),
            ],
        )
    }
}

enum Column<'a> {
    String(Strings<'a>),
    Date(&'a [i32]),
    Int(&'a [i64]),
    /// 64-bit values that all fit in 32 bits, stored as INT32.
    Narrow(&'a [i64]),
    /// Strings with a dictionary page; the flag marks a nullable column.
    Dict(Dictionary<'a>, bool),
}

impl<'a> Column<'a> {
    fn optional(&self) -> bool {
        matches!(self, Self::Dict(_, true))
    }

    /// Any integer width is accepted, so a column whose values all fit in
    /// 32 bits is written in half the bytes.
    fn int(values: &'a [i64]) -> Self {
        if values.iter().all(|&value| i32::try_from(value).is_ok()) {
            Self::Narrow(values)
        } else {
            Self::Int(values)
        }
    }
}

struct Chunk {
    offset: i64,
    size: i64,
    /// The first data page follows the dictionary page, if one was written.
    data: i64,
    dictionary: bool,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn write_table(
    path: &Path,
    name: &str,
    rows: usize,
    columns: &[(&str, Column<'_>)],
) -> io::Result<()> {
    for (name, column) in columns {
        let valid = match column {
            Column::String(values) => values.offsets.len().checked_sub(1) == Some(rows),
            Column::Date(values) => values.len() == rows,
            Column::Int(values) | Column::Narrow(values) => values.len() == rows,
            Column::Dict(values, optional) => {
                let count = values.values.offsets.len().saturating_sub(1);
                values.indices.len() == rows
                    && values.indices.iter().all(|&index| {
                        (index as usize) < count || (*optional && index == Dictionary::NULL)
                    })
            }
        };
        if !valid {
            return Err(invalid(&format!("invalid column length for {name}")));
        }
    }

    let mut writer = Pages {
        file: File::create(path)?,
        offset: MAGIC.len() as i64,
        data: 0,
        header: Vec::with_capacity(32),
        definitions: Vec::new(),
        strings: Vec::new(),
    };
    writer.file.write_all(MAGIC)?;
    let mut chunks = Vec::with_capacity(columns.len());
    if rows != 0 {
        for (_, column) in columns {
            let offset = writer.offset;
            writer.data = 0;
            let mut dictionary = false;
            match column {
                Column::String(values) => {
                    writer.strings(rows, false, |row| text(*values, row).map(Some))?
                }
                Column::Date(values) => writer.integers(values, i32::to_le)?,
                Column::Int(values) => writer.integers(values, i64::to_le)?,
                Column::Narrow(values) => writer.narrow(values)?,
                Column::Dict(values, optional) => {
                    dictionary = writer.dictionary(*values, *optional)?;
                }
            }
            chunks.push(Chunk {
                offset,
                size: writer.offset - offset,
                data: writer.data,
                dictionary,
            });
        }
    }
    let footer = metadata::footer(name, rows as i64, columns, &chunks);
    writer.file.write_all(&footer)
}

fn text(values: Strings<'_>, row: usize) -> io::Result<&str> {
    values
        .data
        .get(values.offsets[row]..values.offsets[row + 1])
        .ok_or_else(|| invalid("invalid UTF-8 string offsets"))
}

struct Pages {
    file: File,
    offset: i64,
    /// Offset of the current column chunk's first data page.
    data: i64,
    header: Vec<u8>,
    definitions: Vec<u8>,
    strings: Vec<u8>,
}

impl Pages {
    fn page(&mut self, rows: usize, bytes: usize) -> io::Result<()> {
        self.encoded_page(rows, bytes, metadata::PLAIN)
    }

    fn encoded_page(&mut self, rows: usize, bytes: usize, encoding: i32) -> io::Result<()> {
        let size =
            i32::try_from(bytes).map_err(|_| invalid("Parquet data page exceeds i32 size"))?;
        if self.data == 0 {
            self.data = self.offset;
        }
        metadata::page_header(&mut self.header, rows as i32, size, encoding);
        self.file.write_all(&self.header)?;
        self.offset += self.header.len() as i64 + i64::from(size);
        Ok(())
    }

    fn integers<T: Copy>(&mut self, values: &[T], _to_le: fn(T) -> T) -> io::Result<()> {
        for page in values.chunks(PAGE_ROWS) {
            #[cfg(target_endian = "big")]
            let converted: Vec<_> = page.iter().copied().map(_to_le).collect();
            #[cfg(target_endian = "big")]
            let page = converted.as_slice();
            let len = size_of_val(page);
            // SAFETY: Called only with i32 and i64 slices, whose bytes are all
            // initialized. The byte view has the same extent and lifetime.
            let bytes = unsafe { std::slice::from_raw_parts(page.as_ptr().cast::<u8>(), len) };
            self.page(page.len(), len)?;
            self.file.write_all(bytes)?;
        }
        Ok(())
    }

    fn narrow(&mut self, values: &[i64]) -> io::Result<()> {
        for page in values.chunks(PAGE_ROWS) {
            self.strings.clear();
            for &value in page {
                // The caller checked that every value fits.
                self.strings
                    .extend_from_slice(&(value as i32).to_le_bytes());
            }
            self.page(page.len(), self.strings.len())?;
            self.file.write_all(&self.strings)?;
        }
        Ok(())
    }

    fn strings<'v>(
        &mut self,
        rows: usize,
        optional: bool,
        value: impl Fn(usize) -> io::Result<Option<&'v str>>,
    ) -> io::Result<()> {
        // A page containing one oversized value needs a four-byte value length.
        // Its optional definition levels take four bytes for length and two for the run.
        let max_value = i32::MAX as usize - 4 - 6 * usize::from(optional);
        let mut present = Vec::new();
        let mut start = 0;
        while start < rows {
            let limit = rows.min(start + PAGE_ROWS);
            self.strings.clear();
            present.clear();
            let mut end = start;
            while end < limit {
                let item = value(end)?;
                present.push(item.is_some());
                if let Some(value) = item {
                    if self.strings.len() + value.len() + 4 > PAGE_BYTES && end != start {
                        present.pop();
                        break;
                    }
                    if value.len() > max_value {
                        return Err(invalid("string is too large for a Parquet data page"));
                    }
                    self.strings
                        .extend_from_slice(&(value.len() as u32).to_le_bytes());
                    self.strings.extend_from_slice(value.as_bytes());
                    if self.strings.len() > PAGE_BYTES {
                        end += 1;
                        break;
                    }
                }
                end += 1;
            }
            self.definitions.clear();
            if optional {
                definitions(&mut self.definitions, &present);
            }
            self.page(end - start, self.definitions.len() + self.strings.len())?;
            self.file.write_all(&self.definitions)?;
            self.file.write_all(&self.strings)?;
            start = end;
        }
        Ok(())
    }

    /// Writes a dictionary page and RLE-encoded index pages. Returns false
    /// after falling back to plain pages for a dictionary too large for one page.
    fn dictionary(&mut self, values: Dictionary<'_>, optional: bool) -> io::Result<bool> {
        let rows = values.indices.len();
        let count = values.values.offsets.len() - 1;
        self.strings.clear();
        for index in 0..count {
            let value = text(values.values, index)?;
            self.strings
                .extend_from_slice(&(value.len() as u32).to_le_bytes());
            self.strings.extend_from_slice(value.as_bytes());
            if self.strings.len() > i32::MAX as usize {
                break;
            }
        }
        // An all-null column still gets a one-entry dictionary that no row uses.
        let entries = count.max(1);
        if count == 0 {
            self.strings.extend_from_slice(&0u32.to_le_bytes());
        }
        if self.strings.len() > i32::MAX as usize {
            return self
                .strings(rows, optional, |row| match values.indices[row] {
                    Dictionary::NULL => Ok(None),
                    index => text(values.values, index as usize).map(Some),
                })
                .map(|()| false);
        }
        metadata::dictionary_header(&mut self.header, entries as i32, self.strings.len() as i32);
        self.file.write_all(&self.header)?;
        self.file.write_all(&self.strings)?;
        self.offset += (self.header.len() + self.strings.len()) as i64;
        let width = (u32::BITS - (entries as u32 - 1).leading_zeros()).max(1);
        let bytes = width.div_ceil(8) as usize;
        let mut present = Vec::new();
        for page in values.indices.chunks(PAGE_ROWS) {
            self.definitions.clear();
            if optional {
                present.clear();
                present.extend(page.iter().map(|&index| index != Dictionary::NULL));
                definitions(&mut self.definitions, &present);
            }
            // Indices are one RLE run per repeated value, after the bit width.
            self.strings.clear();
            self.strings.push(width as u8);
            let mut indices = page
                .iter()
                .copied()
                .filter(|&index| index != Dictionary::NULL);
            if let Some(mut current) = indices.next() {
                let mut run = 1u64;
                let flush = |out: &mut Vec<u8>, value: u32, run: u64| {
                    metadata::varint(out, run << 1);
                    out.extend_from_slice(&value.to_le_bytes()[..bytes]);
                };
                for index in indices {
                    if index == current {
                        run += 1;
                    } else {
                        flush(&mut self.strings, current, run);
                        current = index;
                        run = 1;
                    }
                }
                flush(&mut self.strings, current, run);
            }
            self.encoded_page(
                page.len(),
                self.definitions.len() + self.strings.len(),
                metadata::RLE_DICTIONARY,
            )?;
            self.file.write_all(&self.definitions)?;
            self.file.write_all(&self.strings)?;
        }
        Ok(true)
    }
}

fn definitions(out: &mut Vec<u8>, present: &[bool]) {
    // Data page v1 prefixes the definition levels with their encoded length.
    // One bit-packed hybrid run holds groups of eight one-bit levels.
    let groups = present.len().div_ceil(8);
    out.extend_from_slice(&[0; 4]);
    metadata::varint(out, (groups as u64) << 1 | 1);
    for group in present.chunks(8) {
        out.push(
            group
                .iter()
                .enumerate()
                .fold(0, |byte, (bit, &set)| byte | u8::from(set) << bit),
        );
    }
    let len = (out.len() - 4) as u32;
    out[..4].copy_from_slice(&len.to_le_bytes());
}
