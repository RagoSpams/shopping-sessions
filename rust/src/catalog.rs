use crate::csv::{after_header, csv, lines};
use crate::time::{Window, calendar_ms};
use ahash::AHashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::{env, fs, io};

const UNKNOWN: &[u8] = b"unknown";
const ORGANIC: &[u8] = b"organic";

type Strings = AHashMap<Box<[u8]>, Box<[u8]>>;

fn read_dimension(path: &Path) -> io::Result<Strings> {
    let contents = fs::read(path)?;
    Ok(lines(after_header(&contents))
        .filter_map(|(_, row)| csv::<2>(row))
        .map(|[key, value]| (key.into(), value.into()))
        .collect())
}

pub struct Products {
    pub ids: AHashMap<Box<[u8]>, u32>,
    pub categories: Vec<Box<[u8]>>,
    pub unknown: u32,
}

impl Products {
    pub fn read(path: &Path) -> io::Result<Self> {
        let products = read_dimension(path)?;
        let mut categories: Vec<_> = products.values().cloned().collect();
        categories.push(UNKNOWN.into());
        categories.sort_unstable();
        categories.dedup();
        let category = |name: &[u8]| {
            let index = categories
                .binary_search_by(|label| label.as_ref().cmp(name))
                .unwrap();
            u32::try_from(index).unwrap()
        };
        let ids = products
            .into_iter()
            .map(|(product, label)| (product, category(&label)))
            .collect();
        let unknown = category(UNKNOWN);
        Ok(Self {
            ids,
            categories,
            unknown,
        })
    }
}

pub struct Campaigns {
    pub ids: AHashMap<Box<[u8]>, u32>,
    pub values: Vec<(Box<[u8]>, Box<[u8]>)>,
}

impl Campaigns {
    pub fn read(path: &Path) -> io::Result<Self> {
        let values = read_dimension(path)?;
        let mut ids = AHashMap::with_capacity(values.len());
        let mut campaigns = vec![(ORGANIC.into(), ORGANIC.into())];
        for (campaign, channel) in values {
            ids.insert(campaign.clone(), u32::try_from(campaigns.len()).unwrap());
            campaigns.push((campaign, channel));
        }
        Ok(Self {
            ids,
            values: campaigns,
        })
    }
}

pub fn path(variable: &str, default: &str) -> PathBuf {
    env::var_os(variable).map_or_else(|| PathBuf::from(default), PathBuf::from)
}

pub fn load_window(path: &Path) -> Result<Window, Box<dyn Error>> {
    let manifest: serde_json::Value = serde_json::from_reader(fs::File::open(path)?)?;
    let timestamp = |key: &str| {
        manifest[key]
            .as_str()
            .and_then(|text| calendar_ms(text.as_bytes()))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("invalid {key}")))
    };
    Ok(Window {
        start: timestamp("window_start")?,
        end: timestamp("window_end")?,
    })
}
