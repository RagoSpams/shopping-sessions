use crate::key::PackedKey;
use ahash::AHashMap;

/// Exact catalog lookup for parse. Keys that pack into one word compare as
/// integers in a small open-addressing table; every other key, including
/// long and non-ASCII names, uses the catalog's own byte map.
pub struct Dimension<'a> {
    slots: Vec<(u64, u32)>,
    shift: u32,
    bytes: &'a AHashMap<Box<[u8]>, u32>,
    missing: u32,
}

impl<'a> Dimension<'a> {
    pub fn new(bytes: &'a AHashMap<Box<[u8]>, u32>, missing: u32) -> Self {
        let words: Vec<_> = bytes
            .iter()
            .filter_map(|(name, &value)| PackedKey::word(name).map(|word| (word, value)))
            .collect();
        // Load stays at or below one half, so probe runs stay short.
        let capacity = (2 * words.len()).next_power_of_two().max(16);
        let shift = 64 - capacity.trailing_zeros();
        let mut slots = vec![(0, 0); capacity];
        for (word, value) in words {
            let mut i = Self::home(word, shift);
            while slots[i].0 != 0 {
                i = (i + 1) & (capacity - 1);
            }
            slots[i] = (word, value);
        }
        Self {
            slots,
            shift,
            bytes,
            missing,
        }
    }

    #[inline]
    fn home(word: u64, shift: u32) -> usize {
        (word.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> shift) as usize
    }

    #[inline]
    pub fn get(&self, name: &[u8]) -> u32 {
        let Some(word) = PackedKey::word(name) else {
            return self.bytes.get(name).copied().unwrap_or(self.missing);
        };
        // The empty name packs to the empty-slot marker and is never a key.
        if word == 0 {
            return self.bytes.get(name).copied().unwrap_or(self.missing);
        }
        let mask = self.slots.len() - 1;
        let mut i = Self::home(word, self.shift);
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
        ];
        let catalog: AHashMap<Box<[u8]>, u32> = names
            .iter()
            .enumerate()
            .map(|(i, name)| (Box::from(*name), i as u32 + 1))
            .collect();
        let index = Dimension::new(&catalog, 0);
        for (i, name) in names.iter().enumerate() {
            assert_eq!(index.get(name), i as u32 + 1);
        }
        for absent in [&b""[..], b"P1", b"p2", b"abcdefgh", b"abcdefghik", b"\0\0"] {
            assert_eq!(index.get(absent), 0, "{absent:?}");
        }
    }
}
