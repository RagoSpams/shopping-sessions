use parquet::basic::Compression;
use parquet::data_type::{ByteArray, ByteArrayType, DataType, Int32Type, Int64Type};
use parquet::errors::Result;
use parquet::file::properties::{EnabledStatistics, WriterProperties};
use parquet::file::writer::{SerializedFileWriter, SerializedRowGroupWriter};
use parquet::schema::parser::parse_message_type;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Arc;

const BATCH_ROWS: usize = 32_768;
const USER_SCHEMA: &str = "
    message user_daily {
        REQUIRED BINARY user_id (UTF8);
        REQUIRED INT32 day (DATE);
        REQUIRED INT64 sessions;
        REQUIRED INT64 events;
        REQUIRED INT64 revenue_cents;
        OPTIONAL BINARY top_category (UTF8);
    }
";
const CAMPAIGN_SCHEMA: &str = "
    message campaign_daily {
        REQUIRED BINARY campaign_id (UTF8);
        REQUIRED BINARY channel (UTF8);
        REQUIRED INT32 day (DATE);
        REQUIRED INT64 sessions;
        REQUIRED INT64 purchasing_sessions;
        REQUIRED INT64 revenue_cents;
        REQUIRED INT64 assisted_sessions;
    }
";

type Writer = SerializedFileWriter<BufWriter<File>>;

fn writer(path: &Path, schema: &str, plain: bool) -> Result<Writer> {
    let properties = WriterProperties::builder()
        .set_compression(if plain { Compression::UNCOMPRESSED } else { Compression::SNAPPY })
        .set_dictionary_enabled(!plain)
        .set_statistics_enabled(EnabledStatistics::None)
        .build();
    SerializedFileWriter::new(
        BufWriter::new(File::create(path)?),
        Arc::new(parse_message_type(schema)?),
        Arc::new(properties),
    )
}

fn column<T: DataType>(
    row_group: &mut SerializedRowGroupWriter<'_, impl Write + Send>,
    values: &[T::T],
    definition_levels: Option<&[i16]>,
) -> Result<()> {
    let mut column = row_group.next_column()?.unwrap();
    column
        .typed::<T>()
        .write_batch(values, definition_levels, None)?;
    column.close()
}

pub struct UserWriter {
    file: Writer,
    users: Vec<ByteArray>,
    days: Vec<i32>,
    sessions: Vec<i64>,
    events: Vec<i64>,
    revenue: Vec<i64>,
    categories: Vec<ByteArray>,
    category_levels: Vec<i16>,
}

impl UserWriter {
    pub fn new(path: &Path) -> Result<Self> {
        Self::with_encoding(path, false)
    }

    pub fn with_encoding(path: &Path, plain: bool) -> Result<Self> {
        Ok(Self {
            file: writer(path, USER_SCHEMA, plain)?,
            users: Vec::new(),
            days: Vec::new(),
            sessions: Vec::new(),
            events: Vec::new(),
            revenue: Vec::new(),
            categories: Vec::new(),
            category_levels: Vec::new(),
        })
    }

    pub fn push(
        &mut self,
        user: &[u8],
        day: i32,
        sessions: i64,
        events: i64,
        revenue: i64,
        category: Option<&[u8]>,
    ) -> Result<()> {
        self.users.push(user.into());
        self.days.push(day);
        self.sessions.push(sessions);
        self.events.push(events);
        self.revenue.push(revenue);
        self.category_levels.push(i16::from(category.is_some()));
        if let Some(category) = category {
            self.categories.push(category.into());
        }
        if self.users.len() == BATCH_ROWS {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if self.users.is_empty() {
            return Ok(());
        }
        let mut group = self.file.next_row_group()?;
        column::<ByteArrayType>(&mut group, &self.users, None)?;
        column::<Int32Type>(&mut group, &self.days, None)?;
        column::<Int64Type>(&mut group, &self.sessions, None)?;
        column::<Int64Type>(&mut group, &self.events, None)?;
        column::<Int64Type>(&mut group, &self.revenue, None)?;
        column::<ByteArrayType>(&mut group, &self.categories, Some(&self.category_levels))?;
        group.close()?;
        self.users.clear();
        self.days.clear();
        self.sessions.clear();
        self.events.clear();
        self.revenue.clear();
        self.categories.clear();
        self.category_levels.clear();
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        self.flush()?;
        self.file.close()?;
        Ok(())
    }
}

#[derive(Default)]
pub struct CampaignDay {
    pub sessions: i64,
    pub purchasing: i64,
    pub revenue: i64,
    pub assists: i64,
}

impl CampaignDay {
    pub fn merge(&mut self, other: Self) {
        self.sessions += other.sessions;
        self.purchasing += other.purchasing;
        self.revenue += other.revenue;
        self.assists += other.assists;
    }
}

pub fn write_campaigns<'a>(
    path: &Path,
    rows: impl Iterator<Item = (&'a [u8], &'a [u8], i32, CampaignDay)>,
) -> Result<()> {
    campaign_file(path, rows, false)
}

pub fn campaign_file<'a>(
    path: &Path,
    rows: impl Iterator<Item = (&'a [u8], &'a [u8], i32, CampaignDay)>,
    plain: bool,
) -> Result<()> {
    let mut writer = writer(path, CAMPAIGN_SCHEMA, plain)?;
    let mut rows = rows.peekable();
    while rows.peek().is_some() {
        let mut campaigns = Vec::new();
        let mut channels = Vec::new();
        let mut days = Vec::new();
        let mut sessions = Vec::new();
        let mut purchasing = Vec::new();
        let mut revenue = Vec::new();
        let mut assists = Vec::new();
        for (campaign, channel, day, totals) in rows.by_ref().take(BATCH_ROWS) {
            campaigns.push(ByteArray::from(campaign));
            channels.push(ByteArray::from(channel));
            days.push(day);
            sessions.push(totals.sessions);
            purchasing.push(totals.purchasing);
            revenue.push(totals.revenue);
            assists.push(totals.assists);
        }
        let mut group = writer.next_row_group()?;
        column::<ByteArrayType>(&mut group, &campaigns, None)?;
        column::<ByteArrayType>(&mut group, &channels, None)?;
        column::<Int32Type>(&mut group, &days, None)?;
        column::<Int64Type>(&mut group, &sessions, None)?;
        column::<Int64Type>(&mut group, &purchasing, None)?;
        column::<Int64Type>(&mut group, &revenue, None)?;
        column::<Int64Type>(&mut group, &assists, None)?;
        group.close()?;
    }
    writer.close()?;
    Ok(())
}
