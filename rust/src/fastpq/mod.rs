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

/// The bitmap stores one bit per row, least significant bit first.
/// A set bit marks a present value. Unused bits in the last byte are ignored.
/// Offsets for rows with clear bits may describe empty or ignored values.
#[derive(Clone, Copy)]
pub struct NullableStrings<'a> {
    pub values: Strings<'a>,
    pub validity: &'a [u8],
}

pub struct UserDaily<'a> {
    pub user_id: Strings<'a>,
    /// Days since 1970-01-01.
    pub day: &'a [i32],
    pub sessions: &'a [i64],
    pub events: &'a [i64],
    pub revenue_cents: &'a [i64],
    pub top_category: NullableStrings<'a>,
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
                ("user_id", Column::String(self.user_id)),
                ("day", Column::Date(self.day)),
                ("sessions", Column::int(self.sessions)),
                ("events", Column::int(self.events)),
                ("revenue_cents", Column::int(self.revenue_cents)),
                ("top_category", Column::Nullable(self.top_category)),
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
    Nullable(NullableStrings<'a>),
    Date(&'a [i32]),
    Int(&'a [i64]),
    /// 64-bit values that all fit in 32 bits, stored as INT32.
    Narrow(&'a [i64]),
}

impl<'a> Column<'a> {
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
            Column::Nullable(values) => {
                values.values.offsets.len().checked_sub(1) == Some(rows)
                    && values.validity.len() == rows.div_ceil(8)
            }
            Column::Date(values) => values.len() == rows,
            Column::Int(values) | Column::Narrow(values) => values.len() == rows,
        };
        if !valid {
            return Err(invalid(&format!("invalid column length for {name}")));
        }
    }

    let mut writer = Pages {
        file: File::create(path)?,
        offset: MAGIC.len() as i64,
        header: Vec::with_capacity(32),
        definitions: Vec::new(),
        strings: Vec::new(),
    };
    writer.file.write_all(MAGIC)?;
    let mut chunks = Vec::with_capacity(columns.len());
    if rows != 0 {
        for (_, column) in columns {
            let offset = writer.offset;
            match column {
                Column::String(values) => writer.strings(*values, None)?,
                Column::Nullable(values) => writer.strings(values.values, Some(values.validity))?,
                Column::Date(values) => writer.integers(values, i32::to_le)?,
                Column::Int(values) => writer.integers(values, i64::to_le)?,
                Column::Narrow(values) => writer.narrow(values)?,
            }
            chunks.push(Chunk {
                offset,
                size: writer.offset - offset,
            });
        }
    }
    let footer = metadata::footer(name, rows as i64, columns, &chunks);
    writer.file.write_all(&footer)
}

struct Pages {
    file: File,
    offset: i64,
    header: Vec<u8>,
    definitions: Vec<u8>,
    strings: Vec<u8>,
}

impl Pages {
    fn page(&mut self, rows: usize, bytes: usize) -> io::Result<()> {
        let size =
            i32::try_from(bytes).map_err(|_| invalid("Parquet data page exceeds i32 size"))?;
        metadata::page_header(&mut self.header, rows as i32, size);
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

    fn strings(&mut self, values: Strings<'_>, validity: Option<&[u8]>) -> io::Result<()> {
        let rows = values.offsets.len() - 1;
        // A page containing one oversized value needs a four-byte value length.
        // Its optional definition levels take four bytes for length and two for the run.
        let max_value = i32::MAX as usize - 4 - 6 * usize::from(validity.is_some());
        let mut start = 0;
        while start < rows {
            let limit = rows.min(start + PAGE_ROWS);
            self.strings.clear();
            let estimate = values.offsets[limit].saturating_sub(values.offsets[start]);
            self.strings
                .reserve((estimate.min(PAGE_BYTES) + (limit - start) * 4).min(PAGE_BYTES));
            let mut end = start;
            while end < limit {
                if validity.is_none_or(|bits| bits[end / 8] & (1 << (end % 8)) != 0) {
                    let from = values.offsets[end];
                    let to = values.offsets[end + 1];
                    let value = values
                        .data
                        .get(from..to)
                        .ok_or_else(|| invalid("invalid UTF-8 string offsets"))?;
                    if self.strings.len() + value.len() + 4 > PAGE_BYTES && end != start {
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
            if let Some(bits) = validity {
                definitions(&mut self.definitions, bits, start, end - start);
            }
            self.page(end - start, self.definitions.len() + self.strings.len())?;
            self.file.write_all(&self.definitions)?;
            self.file.write_all(&self.strings)?;
            start = end;
        }
        Ok(())
    }
}

fn definitions(out: &mut Vec<u8>, bits: &[u8], start: usize, rows: usize) {
    // Data page v1 prefixes the definition levels with their encoded length.
    // One bit-packed hybrid run holds groups of eight one-bit levels.
    let groups = rows.div_ceil(8);
    out.extend_from_slice(&[0; 4]);
    metadata::varint(out, (groups as u64) << 1 | 1);
    let base = start / 8;
    let shift = start % 8;
    let source = &bits[base..base + groups];
    if shift == 0 {
        out.extend_from_slice(source);
    } else {
        for (index, &byte) in source.iter().enumerate() {
            let next = bits.get(base + index + 1).copied().unwrap_or(0);
            out.push(byte >> shift | next << (8 - shift));
        }
    }
    if rows % 8 != 0 {
        *out.last_mut().unwrap() &= (1 << (rows % 8)) - 1;
    }
    let len = (out.len() - 4) as u32;
    out[..4].copy_from_slice(&len.to_le_bytes());
}
