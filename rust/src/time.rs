pub const DAY_MS: i64 = 86_400_000;
pub const LOOKBACK_MS: i64 = 7 * DAY_MS;
pub const SESSION_GAP_MS: i64 = 1_800_000;
pub const MAX_PRICE_CENTS: i64 = 10_000_000;

pub fn bounded_uint(value: &[u8], maximum: i64) -> Option<i64> {
    if value.is_empty() || maximum < 0 {
        return None;
    }
    let (limit, last_digit) = (maximum / 10, (maximum % 10) as u8);
    let mut number = 0;
    for &byte in value {
        let digit = byte.wrapping_sub(b'0');
        if digit > 9 || number > limit || (number == limit && digit > last_digit) {
            return None;
        }
        number = number * 10 + i64::from(digit);
    }
    Some(number)
}

fn digits(value: &[u8]) -> Option<i64> {
    let mut number = 0;
    for &byte in value {
        let digit = byte.wrapping_sub(b'0');
        if digit > 9 {
            return None;
        }
        number = number * 10 + i64::from(digit);
    }
    Some(number)
}

pub fn calendar_ms(value: &[u8]) -> Option<i64> {
    let offset_minutes = match value {
        [.., b'Z'] if value.len() == 20 => 0,
        [.., sign @ (b'+' | b'-'), h1, h2, b':', m1, m2] if value.len() == 25 => {
            let hours = digits(&[*h1, *h2])?;
            let minutes = digits(&[*m1, *m2])?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            (hours * 60 + minutes) * if *sign == b'+' { 1 } else { -1 }
        }
        _ => return None,
    };
    if (value[4], value[7], value[10], value[13], value[16])
        != (b'-', b'-', b'T', b':', b':')
    {
        return None;
    }
    let year = digits(&value[0..4])?;
    let month = digits(&value[5..7])?;
    let day = digits(&value[8..10])?;
    let hour = digits(&value[11..13])?;
    let minute = digits(&value[14..16])?;
    let second = digits(&value[17..19])?;
    if year == 0 || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }

    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_index = month as usize - 1;
    let month_days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month_index];
    if day == 0 || day > month_days + i64::from(leap && month == 2) {
        return None;
    }
    let previous_year = year - 1;
    let days_before_year =
        365 * previous_year + previous_year / 4 - previous_year / 100 + previous_year / 400;
    let days_before_month =
        [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334][month_index];
    // There are 719,162 days before 1970-01-01 in the proleptic Gregorian calendar.
    let epoch_days =
        days_before_year + days_before_month + i64::from(leap && month > 2) + day - 1 - 719_162;
    epoch_days
        .checked_mul(DAY_MS)
        .map(|days| days + ((hour * 60 + minute - offset_minutes) * 60 + second) * 1_000)
}

#[derive(Clone, Copy)]
pub struct Window {
    pub start: i64,
    pub end: i64,
}

impl Window {
    pub fn timestamp(self, value: &[u8]) -> Option<i64> {
        // A calendar form has an ASCII hyphen at byte 4. Otherwise only
        // a nonnegative decimal F3 can be valid. Both paths enforce bounds.
        let timestamp = if value.get(4) == Some(&b'-') {
            calendar_ms(value)?
        } else {
            bounded_uint(value, self.end - 1)?
        };
        (self.start <= timestamp && timestamp < self.end).then_some(timestamp)
    }
}
