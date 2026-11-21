use crate::key::PackedKey;
use ahash::AHashMap;
use shopping_sessions::catalog::{Campaigns, Products};
use shopping_sessions::csv::{after_header, csv, lines};
use std::path::Path;
use std::{fs, io};

const UNKNOWN: &[u8] = b"unknown";
const ORGANIC: &[u8] = b"organic";

/// Exact catalog lookup for parse. Keys that pack into one word compare as
/// integers in a small open-addressing table; every other key, including
/// long and non-ASCII names, uses a byte map.
pub struct Dimension {
    slots: Vec<(u64, u32)>,
    used: usize,
    shift: u32,
    bytes: AHashMap<Box<[u8]>, u32>,
    missing: u32,
}

impl Dimension {
    pub fn with_capacity(count: usize, missing: u32) -> Self {
        // Load stays at or below one half, so probe runs stay short.
        let capacity = (2 * count).next_power_of_two().max(16);
        Self {
            slots: vec![(0, 0); capacity],
            used: 0,
            shift: 64 - capacity.trailing_zeros(),
            bytes: AHashMap::new(),
            missing,
        }
    }

    #[inline]
    fn home(&self, word: u64) -> usize {
        (word.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> self.shift) as usize
    }

    /// Insert or replace a name, returning the value it replaced.
    pub fn insert(&mut self, name: &[u8], value: u32) -> Option<u32> {
        // The empty name packs to the empty-slot marker, so it uses the map.
        match PackedKey::word(name).filter(|&word| word != 0) {
            Some(word) => {
                let capacity = self.slots.len();
                let mut i = self.home(word);
                loop {
                    let slot = &mut self.slots[i];
                    if slot.0 == word {
                        return Some(std::mem::replace(&mut slot.1, value));
                    }
                    if slot.0 == 0 {
                        // Capacity covers every row, so a free slot remains.
                        assert!(2 * self.used < capacity);
                        *slot = (word, value);
                        self.used += 1;
                        return None;
                    }
                    i = (i + 1) & (capacity - 1);
                }
            }
            None => self.bytes.insert(name.into(), value),
        }
    }

    #[inline(always)]
    pub fn get(&self, name: &[u8]) -> u32 {
        let Some(word) = PackedKey::word(name).filter(|&word| word != 0) else {
            return self.bytes.get(name).copied().unwrap_or(self.missing);
        };
        let mask = self.slots.len() - 1;
        let mut i = self.home(word);
        loop {
            let (key, value) = self.slots[i];
            if key == word {
                return value;
            }
            if key == 0 {
                return self.missing;
            }
            i = (i + 1) & mask;
        }
    }
}

fn rows(contents: &[u8]) -> Vec<[&[u8]; 2]> {
    lines(after_header(contents))
        .filter_map(|(_, row)| csv::<2>(row))
        .collect()
}

/// Category names sorted byte-wise, so a smaller ID is a smaller name, and a
/// product index into them. A later row for the same product replaces it.
pub fn products(path: &Path) -> io::Result<(Products, Dimension)> {
    let contents = fs::read(path)?;
    let rows = rows(&contents);
    let mut ids: AHashMap<&[u8], u32> = AHashMap::new();
    for [_, category] in &rows {
        ids.entry(category).or_insert(0);
    }
    ids.entry(UNKNOWN).or_insert(0);
    let mut names: Vec<&[u8]> = ids.keys().copied().collect();
    names.sort_unstable();
    for (id, name) in names.iter().enumerate() {
        ids.insert(name, u32::try_from(id).unwrap());
    }
    let unknown = ids[UNKNOWN];
    let mut index = Dimension::with_capacity(rows.len(), unknown);
    for [product, category] in &rows {
        index.insert(product, ids[category]);
    }
    let products = Products {
        ids: AHashMap::new(),
        categories: names.into_iter().map(Box::from).collect(),
        unknown,
    };
    Ok((products, index))
}

/// Campaign 0 is organic, which never appears in the file. A later row for
/// the same campaign replaces its channel.
pub fn campaigns(path: &Path) -> io::Result<(Campaigns, Dimension)> {
    let contents = fs::read(path)?;
    let rows = rows(&contents);
    let mut values: Vec<(Box<[u8]>, Box<[u8]>)> = vec![(ORGANIC.into(), ORGANIC.into())];
    let mut index = Dimension::with_capacity(rows.len(), 0);
    for [campaign, channel] in rows {
        let id = u32::try_from(values.len()).unwrap();
        match index.insert(campaign, id) {
            None => values.push((campaign.into(), channel.into())),
            Some(previous) => {
                index.insert(campaign, previous);
                values[previous as usize].1 = channel.into();
            }
        }
    }
    let campaigns = Campaigns {
        ids: AHashMap::new(),
        values,
    };
    Ok((campaigns, index))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_and_byte_lookups_agree_with_the_catalog() {
        let names: &[&[u8]] = &[
            b"p1",
            b"p25392",
            b"c_1898",
            b"abcdefghi",
            b"abcdefghij",
            "χ1".as_bytes(),
            b"a\0",
            b"\0",
            b" p1",
            b"p1 ",
            b"",
        ];
        let mut index = Dimension::with_capacity(names.len(), 0);
        for (i, name) in names.iter().enumerate() {
            assert_eq!(index.insert(name, i as u32 + 1), None);
        }
        for (i, name) in names.iter().enumerate() {
            assert_eq!(index.get(name), i as u32 + 1);
        }
        for absent in [&b"P1"[..], b"p2", b"abcdefgh", b"abcdefghik", b"\0\0"] {
            assert_eq!(index.get(absent), 0, "{absent:?}");
        }
        assert_eq!(index.insert(b"p1", 99), Some(1));
        assert_eq!(index.get(b"p1"), 99);
    }
}
