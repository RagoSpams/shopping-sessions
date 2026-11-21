use memchr::memchr;
use shopping_sessions::csv::csv;

/// Rows of a mapped chunk that start in `from..until`; the last may continue
/// past `until`. Offsets stay relative to the mapping so a winning click can
/// recover its original ID on an attribution tie.
pub fn rows<const N: usize>(
    chunk: &[u8],
    from: usize,
    until: usize,
) -> impl Iterator<Item = (u32, [&[u8]; N])> {
    let mut start = from;
    std::iter::from_fn(move || {
        while start < until.min(chunk.len()) {
            let offset = start;
            let tail = &chunk[start..];
            #[cfg(all(
                target_arch = "x86_64",
                target_feature = "avx2",
                target_feature = "bmi2"
            ))]
            if tail.len() >= 128 {
                if let Some((len, commas, quotes)) = masks(tail) {
                    start += len + 1;
                    if let Some(fields) = split(&tail[..len], commas, quotes) {
                        return Some((u32::try_from(offset).unwrap(), fields));
                    }
                    continue;
                }
            }
            let len = memchr(b'\n', tail).unwrap_or(tail.len());
            start += len + usize::from(len < tail.len());
            if let Some(fields) = csv(&tail[..len]) {
                return Some((u32::try_from(offset).unwrap(), fields));
            }
        }
        None
    })
}

#[cfg(all(
    target_arch = "x86_64",
    target_feature = "avx2",
    target_feature = "bmi2"
))]
#[inline]
fn masks(tail: &[u8]) -> Option<(usize, u128, u128)> {
    use std::arch::x86_64::*;
    let mut commas = 0;
    let mut quotes = 0;
    // The caller verified 128 readable bytes. The binary is compiled for AVX2.
    unsafe {
        for block in 0..4 {
            let offset = block * 32;
            let bytes = _mm256_loadu_si256(tail.as_ptr().add(offset).cast());
            let mask =
                |c| _mm256_movemask_epi8(_mm256_cmpeq_epi8(bytes, _mm256_set1_epi8(c))) as u32;
            let lf = mask(b'\n' as i8);
            let before_lf = (lf & lf.wrapping_neg()).wrapping_sub(1);
            commas |= u128::from(mask(b',' as i8) & before_lf) << offset;
            quotes |= u128::from(mask(b'"' as i8) & before_lf) << offset;
            if lf != 0 {
                return Some((offset + lf.trailing_zeros() as usize, commas, quotes));
            }
        }
    }
    None
}

#[cfg(all(
    target_arch = "x86_64",
    target_feature = "avx2",
    target_feature = "bmi2"
))]
#[inline]
fn at_commas(bits: u128, commas: u128) -> u64 {
    use std::arch::x86_64::_pext_u64;
    // Pack one adjacent-quote flag per comma into consecutive field bits.
    unsafe {
        let low = _pext_u64(bits as u64, commas as u64);
        let high = _pext_u64((bits >> 64) as u64, (commas >> 64) as u64);
        low | (high << (commas as u64).count_ones())
    }
}

#[cfg(all(
    target_arch = "x86_64",
    target_feature = "avx2",
    target_feature = "bmi2"
))]
#[inline]
fn split<const N: usize>(line: &[u8], mut commas: u128, quotes: u128) -> Option<[&[u8]; N]> {
    if line.is_empty() || commas.count_ones() as usize != N - 1 {
        return None;
    }
    let mut quoted_fields = 0;
    if quotes != 0 {
        let last = 1u128 << (line.len() - 1);
        let starts = (commas << 1) | 1;
        let ends = (commas >> 1) | last;
        // Reject internal quotes and a one-byte field consisting solely of '"'.
        if quotes & !(starts | ends) != 0 || quotes & starts & ends != 0 {
            return None;
        }
        quoted_fields = (at_commas(quotes >> 1, commas) << 1) | (quotes as u64 & 1);
        let closing = at_commas(quotes << 1, commas) | (u64::from(quotes & last != 0) << (N - 1));
        if quoted_fields != closing {
            return None;
        }
    }
    let mut start = 0;
    let fields = std::array::from_fn(|index| {
        let end = if index + 1 == N {
            line.len()
        } else {
            let end = commas.trailing_zeros() as usize;
            commas &= commas - 1;
            end
        };
        let trim = (quoted_fields & 1) as usize;
        quoted_fields >>= 1;
        let field = &line[start + trim..end - trim];
        start = end + 1;
        field
    });
    Some(fields)
}

#[cfg(test)]
mod tests {
    use super::rows;
    use shopping_sessions::csv::{csv, lines};

    fn check<const N: usize>(chunk: &[u8]) {
        let expected: Vec<_> = lines(chunk)
            .filter_map(|(o, line)| csv::<N>(line).map(|f| (o, f)))
            .collect();
        assert_eq!(rows::<N>(chunk, 0, chunk.len()).collect::<Vec<_>>(), expected);
    }

    #[test]
    fn mapped_scanner_matches_reference_at_boundaries() {
        let fields = [
            b"".as_slice(),
            b"\"\"",
            b"abc",
            b"\"abc\"",
            b"x\"",
            b"\"x",
            b"\"",
            b" \",",
            b"a\0b",
            b"x\r",
        ];
        let mut chunk = Vec::new();
        for width in [
            0, 1, 15, 16, 30, 31, 32, 61, 62, 63, 64, 95, 96, 126, 127, 128, 129, 173,
        ] {
            for a in fields {
                for b in fields {
                    chunk.extend_from_slice(a);
                    chunk.push(b',');
                    chunk.extend(std::iter::repeat_n(b'z', width));
                    chunk.extend_from_slice(b",");
                    chunk.extend_from_slice(b);
                    chunk.push(b'\n');
                }
            }
        }
        chunk.extend_from_slice(b"\"last\",,final");
        check::<3>(&chunk);
    }

    #[test]
    fn sliding_window_matches_reference_on_random_rows() {
        // A small deterministic generator mixes short and long rows, quotes in
        // every position, empty lines, CR bytes and wrong field counts.
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move |n: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % n
        };
        let alphabet = b"ab,\"\r z\n";
        for round in 0..200 {
            let mut chunk = Vec::new();
            for _ in 0..200 {
                let fields = 1 + next(9) as usize;
                for f in 0..fields {
                    if f > 0 {
                        chunk.push(b',');
                    }
                    let quoted = next(4) == 0;
                    if quoted {
                        chunk.push(b'"');
                    }
                    let width = match next(10) {
                        0 => next(200) as usize,
                        1 => 0,
                        _ => next(14) as usize,
                    };
                    for _ in 0..width {
                        // Mostly plain bytes, occasionally structural ones.
                        let byte = if next(25) == 0 {
                            alphabet[next(alphabet.len() as u64) as usize]
                        } else {
                            b'a' + next(26) as u8
                        };
                        chunk.push(byte);
                    }
                    if quoted && next(8) != 0 {
                        chunk.push(b'"');
                    }
                }
                if next(30) != 0 {
                    chunk.push(b'\n');
                }
            }
            if round % 2 == 0 {
                chunk.extend_from_slice(b"tail,without,newline");
            }
            check::<2>(&chunk);
            check::<3>(&chunk);
            check::<4>(&chunk);
            check::<7>(&chunk);
        }
    }
}
