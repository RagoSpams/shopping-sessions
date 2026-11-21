use super::*;
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::record::Field;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let path = std::env::temp_dir().join(format!("fastpq-{}-{stamp:?}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read_rows(path: &Path) -> Vec<parquet::record::Row> {
    let reader = SerializedFileReader::new(File::open(path).unwrap()).unwrap();
    assert_eq!(reader.num_row_groups(), 1);
    reader
        .get_row_iter(None)
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn user_dictionaries_round_trip_across_pages_with_nulls() {
    let temp = TestDir::new();
    let path = temp.0.join("user.parquet");
    let names = Strings {
        offsets: &[1, 1, 3, 7, 8],
        data: "_é🍋x",
    };
    let categories = Strings {
        offsets: &[0, 3, 4],
        data: "茶b",
    };
    // More rows than one data page, with runs that cross the page boundary.
    let rows = PAGE_ROWS + 1000;
    let user_id: Vec<u32> = (0..rows).map(|row| ((row / 3) % 4) as u32).collect();
    let top_category: Vec<u32> = (0..rows)
        .map(|row| match row % 5 {
            0 | 1 => Dictionary::NULL,
            2 => 1,
            _ => 0,
        })
        .collect();
    let days: Vec<i32> = (0..rows).map(|row| row as i32 - 25_567).collect();
    let totals: Vec<i64> = (0..rows)
        .map(|row| [i64::MIN, -1, 0, 1, i64::MAX][row % 5])
        .collect();
    UserDaily {
        user_id: Dictionary {
            values: names,
            indices: &user_id,
        },
        day: &days,
        sessions: &totals,
        events: &totals,
        revenue_cents: &totals,
        top_category: Dictionary {
            values: categories,
            indices: &top_category,
        },
    }
    .write(&path)
    .unwrap();
    let read = read_rows(&path);
    assert_eq!(read.len(), rows);
    let name = ["", "é", "🍋", "x"];
    let category = [Some("茶"), Some("b")];
    for (row, values) in read.iter().enumerate() {
        let values: Vec<_> = values.get_column_iter().map(|(_, value)| value).collect();
        assert_eq!(
            values[0],
            &Field::Str(name[user_id[row] as usize].to_owned())
        );
        assert_eq!(values[1], &Field::Date(days[row]));
        assert_eq!(values[2..5], [&Field::Long(totals[row]); 3]);
        let expected = match top_category[row] {
            Dictionary::NULL => None,
            index => category[index as usize],
        };
        assert_eq!(
            values[5],
            &expected.map_or(Field::Null, |s| Field::Str(s.to_owned()))
        );
    }
}

#[test]
fn all_null_dictionary_and_narrow_integers() {
    let temp = TestDir::new();
    let path = temp.0.join("user.parquet");
    let empty = Strings {
        offsets: &[0],
        data: "",
    };
    let small = [i64::from(i32::MIN), 0, i64::from(i32::MAX)];
    UserDaily {
        user_id: Dictionary {
            values: Strings {
                offsets: &[0, 1],
                data: "u",
            },
            indices: &[0, 0, 0],
        },
        day: &[0, 1, 2],
        sessions: &small,
        events: &small,
        revenue_cents: &small,
        top_category: Dictionary {
            values: empty,
            indices: &[Dictionary::NULL; 3],
        },
    }
    .write(&path)
    .unwrap();
    for (row, values) in read_rows(&path).iter().enumerate() {
        let values: Vec<_> = values.get_column_iter().map(|(_, value)| value).collect();
        assert_eq!(values[0], &Field::Str("u".to_owned()));
        assert_eq!(values[2..5], [&Field::Int(small[row] as i32); 3]);
        assert_eq!(values[5], &Field::Null);
    }
}

#[test]
fn plain_strings_split_pages_around_large_values() {
    let temp = TestDir::new();
    let path = temp.0.join("campaign.parquet");
    let large = format!("{}茶", "a".repeat(PAGE_BYTES));
    let data = format!("x{large}y");
    let ends = [1, 1 + PAGE_BYTES, 1 + large.len(), data.len()];
    let offsets = [0, ends[0], ends[1], ends[2], ends[3]];
    let values = Strings {
        offsets: &offsets,
        data: &data,
    };
    let zero = [0; 4];
    CampaignDaily {
        campaign_id: values,
        channel: values,
        day: &[0; 4],
        sessions: &zero,
        purchasing_sessions: &zero,
        revenue_cents: &zero,
        assisted_sessions: &zero,
    }
    .write(&path)
    .unwrap();
    let expected = ["x", &large[..PAGE_BYTES], "茶", "y"];
    for (row, values) in read_rows(&path).iter().enumerate() {
        let values: Vec<_> = values.get_column_iter().map(|(_, value)| value).collect();
        assert_eq!(values[0], &Field::Str(expected[row].to_owned()));
        assert_eq!(values[1], &Field::Str(expected[row].to_owned()));
    }
}

#[test]
fn empty_campaign_file_and_invalid_column_length() {
    let temp = TestDir::new();
    let path = temp.0.join("campaign.parquet");
    let empty = Strings {
        offsets: &[0],
        data: "",
    };
    let mut campaign = CampaignDaily {
        campaign_id: empty,
        channel: empty,
        day: &[],
        sessions: &[],
        purchasing_sessions: &[],
        revenue_cents: &[],
        assisted_sessions: &[1],
    };
    assert_eq!(
        campaign.write(&path).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(!path.exists());
    campaign.assisted_sessions = &[];
    campaign.write(&path).unwrap();
    let reader = SerializedFileReader::new(File::open(&path).unwrap()).unwrap();
    assert_eq!(reader.num_row_groups(), 0);
    assert_eq!(reader.get_row_iter(None).unwrap().count(), 0);
}
