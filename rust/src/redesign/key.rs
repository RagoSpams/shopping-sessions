use ahash::RandomState;
use memmap2::Mmap;

/// Short keys contain every byte and their length. A long key identifies a byte
/// range in an immutable mapping. A hash collision always reaches exact equality.
#[derive(Clone, Copy, Default)]
pub struct Key([u8; 16]);

impl Key {
    #[inline]
    pub fn new(value: &[u8], chunk: &[u8], chunk_id: usize) -> Self {
        let mut bytes = [0; 16];
        if value.len() <= 15 {
            bytes[..value.len()].copy_from_slice(value);
            bytes[15] = value.len() as u8;
        } else {
            let offset = value.as_ptr() as usize - chunk.as_ptr() as usize;
            bytes[..4].copy_from_slice(&u32::try_from(chunk_id).unwrap().to_le_bytes());
            bytes[4..8].copy_from_slice(&u32::try_from(offset).unwrap().to_le_bytes());
            bytes[8..].copy_from_slice(&(value.len() as u64 | (1 << 63)).to_le_bytes());
        }
        Self(bytes)
    }

    #[inline]
    pub fn bytes<'a>(&'a self, chunks: &'a [Mmap]) -> &'a [u8] {
        if self.0[15] < 16 {
            return &self.0[..self.0[15] as usize];
        }
        let chunk = u32::from_le_bytes(self.0[..4].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(self.0[4..8].try_into().unwrap()) as usize;
        let len = u64::from_le_bytes(self.0[8..].try_into().unwrap()) & !(1 << 63);
        &chunks[chunk][offset..offset + len as usize]
    }
}

// Nine seven-bit characters fit into the low 63 bits of a word. Shorter
// strings have implicit zero padding. A nonempty string ending in NUL uses
// the mapped form so its length can never alias that padding.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PackedKey(u64);

const MAPPED: u64 = 1 << 63;
const ASCII: u64 = 0x7f7f_7f7f_7f7f_7f7f;

impl PackedKey {
    #[inline]
    pub fn new(value: &[u8], chunk: &[u8], chunk_id: usize) -> Self {
        if value.len() <= 9 {
            let (first, last) = if value.len() >= 8 {
                (
                    u64::from_le_bytes(value[..8].try_into().unwrap()),
                    value.get(8).copied().unwrap_or(0),
                )
            } else {
                let mut bytes = [0; 8];
                bytes[..value.len()].copy_from_slice(value);
                (u64::from_le_bytes(bytes), 0)
            };
            if first & !ASCII == 0 && last < 128 && value.last() != Some(&0) {
                #[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
                let packed = unsafe { std::arch::x86_64::_pext_u64(first, ASCII) };
                #[cfg(not(all(target_arch = "x86_64", target_feature = "bmi2")))]
                let packed = first
                    .to_le_bytes()
                    .iter()
                    .enumerate()
                    .fold(0, |word, (i, &b)| word | (u64::from(b) << (7 * i)));
                return Self(packed | (u64::from(last) << 56));
            }
        }
        let offset = u32::try_from(value.as_ptr() as usize - chunk.as_ptr() as usize).unwrap();
        // Linux limits the number of live mappings to a signed int.
        let chunk_id = i32::try_from(chunk_id).unwrap() as u64;
        Self(MAPPED | (chunk_id << 32) | u64::from(offset))
    }

    #[inline]
    fn mapped(self, chunks: &[Mmap]) -> (&[u8], usize) {
        let index = ((self.0 & !MAPPED) >> 32) as usize;
        let tail = &chunks[index][self.0 as u32 as usize..];
        // Only structurally valid fields reach this code. The saved offset
        // follows any opening quote. Delimiters never appear inside a value.
        let end = memchr::memchr2(b',', b'\n', tail).unwrap_or(tail.len());
        let field = &tail[..end];
        (field.strip_suffix(b"\"").unwrap_or(field), index)
    }

    #[inline]
    pub fn matches(self, other: Self, chunks: &[Mmap]) -> bool {
        self == other
            || (self.0 & other.0 & MAPPED != 0 && self.mapped(chunks).0 == other.mapped(chunks).0)
    }

    #[inline]
    pub fn hash(self, state: &RandomState, chunks: &[Mmap]) -> u64 {
        if self.0 & MAPPED == 0 {
            state.hash_one(self.0)
        } else {
            state.hash_one(self.mapped(chunks).0)
        }
    }

    pub fn name(self, chunks: &[Mmap]) -> Key {
        if self.0 & MAPPED != 0 {
            let (name, index) = self.mapped(chunks);
            return Key::new(name, &chunks[index], index);
        }
        #[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
        let first = unsafe { std::arch::x86_64::_pdep_u64(self.0, ASCII) }.to_le_bytes();
        #[cfg(not(all(target_arch = "x86_64", target_feature = "bmi2")))]
        let first: [u8; 8] = std::array::from_fn(|i| ((self.0 >> (7 * i)) & 127) as u8);
        let mut bytes = [0; 16];
        bytes[..8].copy_from_slice(&first);
        bytes[8] = (self.0 >> 56) as u8;
        bytes[15] = (64 - self.0.leading_zeros()).div_ceil(7) as u8;
        Key(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use memmap2::MmapMut;

    #[test]
    fn packed_and_mapped_fields_round_trip_exactly() {
        let values: &[&[u8]] = &[
            b"",
            b"a",
            b"a\0",
            b"\0a",
            b"abcdefg",
            b"abcdefgh",
            b"abcdefghi",
            b"abcdefghij",
            b"abcdefgh\0",
            b"\0bcdefghi",
            b"\x01",
            "χ123456789".as_bytes(),
            b"name with spaces ",
            b"x\r",
        ];
        let mut bytes = Vec::new();
        let mut ranges = Vec::new();
        for quote in [false, true] {
            for &value in values {
                if quote {
                    bytes.push(b'"');
                }
                let start = bytes.len();
                bytes.extend_from_slice(value);
                ranges.push(start..bytes.len());
                if quote {
                    bytes.push(b'"');
                }
                bytes.push(b',');
            }
        }
        let mut map = MmapMut::map_anon(bytes.len()).unwrap();
        map.copy_from_slice(&bytes);
        let chunks = [map.make_read_only().unwrap()];
        let keys: Vec<_> = ranges
            .iter()
            .map(|r| PackedKey::new(&chunks[0][r.clone()], &chunks[0], 0))
            .collect();
        let hash = RandomState::new();
        for (i, key) in keys.iter().enumerate() {
            assert_eq!(key.name(&chunks).bytes(&chunks), values[i % values.len()]);
            for (j, other) in keys.iter().enumerate() {
                let equal = values[i % values.len()] == values[j % values.len()];
                assert_eq!(key.matches(*other, &chunks), equal);
                if equal {
                    assert_eq!(key.hash(&hash, &chunks), other.hash(&hash, &chunks));
                }
            }
        }
    }
}
