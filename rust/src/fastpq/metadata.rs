//! The subset of Thrift Compact Protocol used by Parquet page headers and footers.

use super::{Chunk, Column, MAGIC};

const I32: u8 = 5;
const I64: u8 = 6;
const BINARY: u8 = 8;
const LIST: u8 = 9;
const STRUCT: u8 = 12;

const PLAIN: i32 = 0;
const RLE: i32 = 3;
const UNCOMPRESSED: i32 = 0;
const DATA_PAGE: i32 = 0;

impl Column<'_> {
    fn physical_type(&self) -> i32 {
        match self {
            Self::Date(_) => 1,                       // INT32
            Self::Int(_) => 2,                        // INT64
            Self::String(_) | Self::Nullable(_) => 6, // BYTE_ARRAY
        }
    }
}

pub(super) fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 128 {
        out.push(value as u8 | 128);
        value >>= 7;
    }
    out.push(value as u8);
}

fn integer(out: &mut Vec<u8>, value: i64) {
    varint(out, ((value as u64) << 1) ^ ((value >> 63) as u64));
}

fn binary(out: &mut Vec<u8>, value: &str) {
    varint(out, value.len() as u64);
    out.extend_from_slice(value.as_bytes());
}

struct Compact<'a> {
    out: &'a mut Vec<u8>,
    last: u8,
}

fn structure(out: &mut Vec<u8>, fields: impl FnOnce(&mut Compact<'_>)) {
    fields(&mut Compact { out, last: 0 });
    out.push(0); // STOP
}

impl Compact<'_> {
    fn field(&mut self, id: u8, kind: u8) {
        // The fields below are in ascending order and use only short headers.
        let delta = id - self.last;
        debug_assert!((1..=15).contains(&delta));
        self.out.push(delta << 4 | kind);
        self.last = id;
    }

    fn i32(&mut self, id: u8, value: i32) {
        self.field(id, I32);
        integer(self.out, i64::from(value));
    }

    fn i64(&mut self, id: u8, value: i64) {
        self.field(id, I64);
        integer(self.out, value);
    }

    fn str(&mut self, id: u8, value: &str) {
        self.field(id, BINARY);
        binary(self.out, value);
    }

    fn structure(&mut self, id: u8, fields: impl FnOnce(&mut Compact<'_>)) {
        self.field(id, STRUCT);
        structure(self.out, fields);
    }

    fn list(&mut self, id: u8, kind: u8, len: usize) -> &mut Vec<u8> {
        // All metadata lists have fewer than 15 items.
        debug_assert!(len < 15);
        self.field(id, LIST);
        self.out.push((len as u8) << 4 | kind);
        self.out
    }
}

pub(super) fn page_header(out: &mut Vec<u8>, rows: i32, bytes: i32) {
    out.clear();
    structure(out, |page| {
        page.i32(1, DATA_PAGE);
        page.i32(2, bytes); // uncompressed_page_size
        page.i32(3, bytes); // compressed_page_size
        page.structure(5, |data| {
            data.i32(1, rows);
            data.i32(2, PLAIN);
            data.i32(3, RLE); // definition_level_encoding
            data.i32(4, RLE); // repetition_level_encoding
        });
    });
}

pub(super) fn footer(
    name: &str,
    rows: i64,
    columns: &[(&str, Column<'_>)],
    chunks: &[Chunk],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(512);
    structure(&mut out, |file| {
        file.i32(1, 1); // version
        let schema = file.list(2, STRUCT, columns.len() + 1);
        structure(schema, |root| {
            root.str(4, name);
            root.i32(5, columns.len() as i32);
        });
        for (name, column) in columns {
            structure(schema, |leaf| {
                leaf.i32(1, column.physical_type());
                leaf.i32(3, i32::from(matches!(column, Column::Nullable(_))));
                leaf.str(4, name);
                let annotation = match column {
                    Column::String(_) | Column::Nullable(_) => Some((0, 1)), // UTF8 / STRING
                    Column::Date(_) => Some((6, 6)),                         // DATE / DATE
                    Column::Int(_) => None,
                };
                if let Some((converted, logical)) = annotation {
                    leaf.i32(6, converted);
                    leaf.structure(10, |union| union.structure(logical, |_| {}));
                }
            });
        }
        file.i64(3, rows);
        let groups = file.list(4, STRUCT, usize::from(rows != 0));
        if rows != 0 {
            structure(groups, |group| {
                let list = group.list(1, STRUCT, chunks.len());
                for ((name, column), chunk) in columns.iter().zip(chunks) {
                    structure(list, |entry| {
                        entry.i64(2, 0); // Deprecated file_offset
                        entry.structure(3, |meta| {
                            meta.i32(1, column.physical_type());
                            let optional = matches!(column, Column::Nullable(_));
                            let encodings = meta.list(2, I32, 1 + usize::from(optional));
                            integer(encodings, i64::from(PLAIN));
                            if optional {
                                integer(encodings, i64::from(RLE));
                            }
                            binary(meta.list(3, BINARY, 1), name);
                            meta.i32(4, UNCOMPRESSED);
                            meta.i64(5, rows); // num_values includes nulls
                            meta.i64(6, chunk.size); // total_uncompressed_size
                            meta.i64(7, chunk.size); // total_compressed_size
                            meta.i64(9, chunk.offset); // data_page_offset
                        });
                    });
                }
                group.i64(2, chunks.iter().map(|chunk| chunk.size).sum());
                group.i64(3, rows);
            });
        }
    });
    let len = out.len() as u32;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(MAGIC);
    out
}
