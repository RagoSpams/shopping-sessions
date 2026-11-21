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

    #[inline]
    pub fn hash(self, state: &RandomState, chunks: &[Mmap]) -> u64 {
        if self.0[15] < 16 {
            state.hash_one(u128::from_le_bytes(self.0))
        } else {
            state.hash_one(self.bytes(chunks))
        }
    }

    #[inline]
    pub fn matches(self, other: Self, chunks: &[Mmap]) -> bool {
        self.0 == other.0
            || (self.0[15] >= 16 && other.0[15] >= 16 && self.bytes(chunks) == other.bytes(chunks))
    }
}
