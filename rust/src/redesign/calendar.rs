use shopping_sessions::time::{Window, calendar_ms};

#[derive(Clone, Copy)]
struct Day {
    prefix: [u8; 10],
    midnight: i64,
}

pub struct Calendar([Day; 32]);

impl Calendar {
    pub fn new() -> Self {
        Self(
            [Day {
                prefix: [0; 10],
                midnight: i64::MIN,
            }; 32],
        )
    }

    #[inline]
    pub fn read(&mut self, value: &[u8]) -> Option<i64> {
        let zone = match value {
            [.., b'Z'] if value.len() == 20 => 0,
            [.., sign @ (b'+' | b'-'), h0, h1, b':', m0, m1] if value.len() == 25 => {
                let hours = decimal_pair(*h0, *h1)?;
                let minutes = decimal_pair(*m0, *m1)?;
                if hours >= 24 || minutes >= 60 {
                    return None;
                }
                (hours * 60 + minutes) * if *sign == b'+' { 60 } else { -60 }
            }
            _ => return None,
        };
        if value[10] != b'T' {
            return None;
        }
        let hms = u64::from_le_bytes(value[11..19].try_into().unwrap());
        const DIGITS: u64 = 0xffff_00ff_ff00_ffff;
        const COLONS: u64 = u64::from_le_bytes(*b"\0\0:\0\0:\0\0");
        if hms & !DIGITS != COLONS {
            return None;
        }
        let digits = (hms ^ 0x3030_3030_3030_3030) & DIGITS;
        // For ASCII decimal bytes, adding 0x76 leaves the high bit clear.
        // A carry between bytes already implies an invalid byte.
        if (digits | digits.wrapping_add(0x7676_7676_7676_7676)) & 0x8080_8080_8080_8080 != 0 {
            return None;
        }
        let pair = |n: u64| ((n & 15) * 10 + ((n >> 8) & 15)) as i64;
        let (hour, minute, second) = (pair(digits), pair(digits >> 24), pair(digits >> 48));
        if hour >= 24 || minute >= 60 || second >= 60 {
            return None;
        }

        // The day of month selects a slot. Exact prefix equality checks the
        // year, month, day and both separators before reusing the conversion.
        let slot = (usize::from(value[8]) * 10 + usize::from(value[9])) & 31;
        let entry = &mut self.0[slot];
        let prefix: [u8; 10] = value[..10].try_into().unwrap();
        if entry.prefix != prefix || entry.midnight == i64::MIN {
            let mut date = *b"0000-00-00T00:00:00Z";
            date[..10].copy_from_slice(&prefix);
            let midnight = calendar_ms(&date)?;
            *entry = Day { prefix, midnight };
        }
        Some(entry.midnight + ((hour * 60 + minute) * 60 + second - zone) * 1000)
    }

    #[inline]
    pub fn timestamp(&mut self, window: Window, value: &[u8]) -> Option<i64> {
        if value.get(4) != Some(&b'-') {
            if let Some(time) = thirteen_digits(value) {
                return (window.start <= time && time < window.end).then_some(time);
            }
            return window.timestamp(value);
        }
        let time = self.read(value)?;
        (window.start <= time && time < window.end).then_some(time)
    }
}

/// Eight ASCII digits as a number, or None if any byte is not a digit.
#[inline]
fn eight_digits(bytes: [u8; 8]) -> Option<u64> {
    let digits = u64::from_le_bytes(bytes).wrapping_sub(0x3030_3030_3030_3030);
    // A byte below '0' borrows into bit 7, and adding 0x76 sets bit 7 for
    // any value above 9. Either way the byte is not a digit.
    if (digits | digits.wrapping_add(0x7676_7676_7676_7676)) & 0x8080_8080_8080_8080 != 0 {
        return None;
    }
    let pairs = digits.wrapping_mul(10).wrapping_add(digits >> 8);
    let quads =
        (pairs & 0x00ff_00ff_00ff_00ff).wrapping_mul(1 + (100 << 16)) >> 16 & 0x0000_ffff_0000_ffff;
    Some((quads & 0xffff) * 10_000 + (quads >> 32))
}

/// A thirteen-digit F3 value. Other lengths use the general parser.
#[inline]
fn thirteen_digits(value: &[u8]) -> Option<i64> {
    if value.len() != 13 {
        return None;
    }
    let high = eight_digits(value[..8].try_into().unwrap())?;
    // Bytes 5..13 overlap the first word, so the low five digits are the
    // remainder of this second eight-digit number.
    let low = eight_digits(value[5..13].try_into().unwrap())? % 100_000;
    Some((high * 100_000 + low) as i64)
}

#[inline]
fn decimal_pair(tens: u8, units: u8) -> Option<i64> {
    let (a, b) = (tens.wrapping_sub(b'0'), units.wrapping_sub(b'0'));
    (a < 10 && b < 10).then(|| i64::from(a) * 10 + i64::from(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thirteen_digit_fast_path_matches_the_general_parser() {
        let window = Window {
            start: i64::MIN,
            end: i64::MAX,
        };
        let mut cache = Calendar::new();
        for original in [
            b"1791071890000".as_slice(),
            b"0000000000000",
            b"9999999999999",
            b"1234567890123",
            b"0001791071890",
        ] {
            assert_eq!(
                cache.timestamp(window, original),
                window.timestamp(original)
            );
            for position in 0..original.len() {
                let mut value = original.to_vec();
                for byte in 0..=255 {
                    value[position] = byte;
                    assert_eq!(
                        cache.timestamp(window, &value),
                        window.timestamp(&value),
                        "{value:?}"
                    );
                }
            }
        }
        let narrow = Window {
            start: 1_791_071_890_000,
            end: 1_791_071_890_002,
        };
        for (value, expected) in [
            (b"1791071889999", None),
            (b"1791071890000", Some(1_791_071_890_000)),
            (b"1791071890001", Some(1_791_071_890_001)),
            (b"1791071890002", None),
        ] {
            assert_eq!(cache.timestamp(narrow, value), expected);
        }
    }

    #[test]
    fn calendar_cache_and_clock_match_scalar_validation() {
        let mut cache = Calendar::new();
        for original in [
            b"2000-02-29T23:58:59Z".as_slice(),
            b"1900-02-28T01:02:03-00:00",
            b"0001-01-01T00:00:00+23:59",
            b"9999-12-31T23:59:59Z",
        ] {
            assert_eq!(cache.read(original), calendar_ms(original));
            for position in 0..original.len() {
                let mut value = original.to_vec();
                for byte in 0..=255 {
                    value[position] = byte;
                    assert_eq!(cache.read(&value), calendar_ms(&value), "{value:?}");
                }
            }
        }
    }
}
