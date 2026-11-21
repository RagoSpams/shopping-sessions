use super::*;
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::record::Field;
use std::path::PathBuf;
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

#[test]
fn user_round_trip_across_unaligned_nullable_pages() {
    let temp = TestDir::new();
    let path = temp.0.join("user.parquet");
    let categories = format!("{}茶", "a".repeat(PAGE_BYTES));
    let user_ids = ["", "é", "🍋", "x", "y"];
    let days = [-25_567, -1, 0, 24_000, 47_482];
    let totals = [i64::MIN, -1, 0, 1, i64::MAX];
    let user = UserDaily {
        user_id: Strings {
            offsets: &[1, 1, 3, 7, 8, 9],
            data: "_é🍋xy",
        },
        day: &days,
        sessions: &totals,
        events: &totals,
        revenue_cents: &totals,
        top_category: NullableStrings {
            values: Strings {
                offsets: &[
                    0,
                    0,
                    PAGE_BYTES as u32,
                    PAGE_BYTES as u32,
                    categories.len() as u32,
                    categories.len() as u32,
                ],
                data: &categories,
            },
            validity: &[0b1111_1011], // Padding bits are ignored.
        },
    };
    user.write(&path).unwrap();
    let reader = SerializedFileReader::new(File::open(&path).unwrap()).unwrap();
    assert_eq!(reader.num_row_groups(), 1);
    let rows: Vec<_> = reader
        .get_row_iter(None)
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows.len(), days.len());
    let expected = [
        Some(""),
        Some(&categories[..PAGE_BYTES]),
        None,
        Some("茶"),
        Some(""),
    ];
    for (index, row) in rows.iter().enumerate() {
        let values: Vec<_> = row.get_column_iter().map(|(_, value)| value).collect();
        assert_eq!(values[0], &Field::Str(user_ids[index].to_owned()));
        assert_eq!(values[1], &Field::Date(days[index]));
        assert_eq!(values[2..5], [&Field::Long(totals[index]); 3]);
        assert_eq!(
            values[5],
            &expected[index].map_or(Field::Null, |s| Field::Str(s.to_owned()))
        );
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
