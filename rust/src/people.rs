use crate::input::{SHARDS, csv, lines, read_parts, shard};
use ahash::{AHashMap, RandomState};
use hashbrown::HashTable;
use std::io;
use std::path::Path;

const UNLINKED: u32 = 1 << 31;

#[derive(Default)]
struct Aliases {
    short: AHashMap<u128, u64>,
    long: AHashMap<Box<[u8]>, u64>,
}

impl Aliases {
    fn get(&self, name: &[u8]) -> Option<u64> {
        match packed_name(name) {
            Some(key) => self.short.get(&key).copied(),
            None => self.long.get(name).copied(),
        }
    }

    fn values_mut(&mut self) -> impl Iterator<Item = &mut u64> {
        self.short.values_mut().chain(self.long.values_mut())
    }
}

// Reserve one byte for the length so the integer is an exact key even
// when an ID contains zero bytes. Longer names use the byte-slice map.
fn packed_name(name: &[u8]) -> Option<u128> {
    if name.len() >= 16 {
        return None;
    }
    let mut key = [0u8; 16];
    key[..name.len()].copy_from_slice(name);
    key[15] = name.len() as u8;
    Some(u128::from_le_bytes(key))
}

pub struct People {
    aliases: Aliases,
    hash: RandomState,
    pub names: Vec<Vec<Box<[u8]>>>,
}

impl People {
    pub fn read(directory: &Path) -> io::Result<Self> {
        let mut people = Self {
            aliases: Aliases::default(),
            hash: RandomState::new(),
            names: crate::input::empty_shards(),
        };
        let chunks = read_parts(directory)?;
        let mut components = Vec::new();
        for chunk in &chunks {
            for (_, row) in lines(chunk) {
                if let Some([first, second]) = csv::<2>(row)
                    && !first.is_empty()
                    && !second.is_empty()
                {
                    let left = link_id(&mut people.aliases, &mut components, first);
                    let right = link_id(&mut people.aliases, &mut components, second);
                    join(&mut components, left, right);
                }
            }
        }
        let mut canonical = vec![0; components.len()];
        for (id, &component) in components.iter().enumerate() {
            if component.parent != id {
                continue;
            }
            let name = component.minimum;
            let bucket = shard(people.hash.hash_one(name));
            let names = &mut people.names[bucket];
            let index = u32::try_from(names.len()).unwrap();
            assert!(index < UNLINKED);
            canonical[id] = (u64::from(index) << 32) | bucket as u64;
            names.push(name.into());
        }
        // The link interner already holds every alias. Rewrite each value
        // in place to the shard and index of its component's minimum name.
        for id in people.aliases.values_mut() {
            *id = canonical[root(&mut components, *id as usize)];
        }
        Ok(people)
    }
}

#[derive(Clone, Copy)]
struct Component<'a> {
    parent: usize,
    size: usize,
    minimum: &'a [u8],
}

fn link_id<'a>(
    aliases: &mut Aliases,
    components: &mut Vec<Component<'a>>,
    name: &'a [u8],
) -> usize {
    let mut add_component = || {
        let id = components.len();
        components.push(Component {
            parent: id,
            size: 1,
            minimum: name,
        });
        id as u64
    };
    let id = if let Some(key) = packed_name(name) {
        *aliases.short.entry(key).or_insert_with(add_component)
    } else if let Some(&id) = aliases.long.get(name) {
        id
    } else {
        let id = add_component();
        aliases.long.insert(name.into(), id);
        id
    };
    id as usize
}

fn root(components: &mut [Component<'_>], mut id: usize) -> usize {
    while components[id].parent != id {
        let grandparent = components[components[id].parent].parent;
        components[id].parent = grandparent;
        id = grandparent;
    }
    id
}

fn join(components: &mut [Component<'_>], left: usize, right: usize) {
    let mut left = root(components, left);
    let mut right = root(components, right);
    if left == right {
        return;
    }
    if components[left].size < components[right].size {
        std::mem::swap(&mut left, &mut right);
    }
    components[right].parent = left;
    components[left].size += components[right].size;
    components[left].minimum = components[left].minimum.min(components[right].minimum);
}

pub struct PersonRow<T> {
    pub person: u32,
    pub data: T,
}

pub struct Partition<T> {
    names: HashTable<u32>,
    strings: Vec<Box<[u8]>>,
    rows: Vec<PersonRow<T>>,
}

pub struct Batch<T>(pub Vec<Partition<T>>);

impl<T> Default for Batch<T> {
    fn default() -> Self {
        Self(
            (0..SHARDS)
                .map(|_| Partition {
                    names: HashTable::new(),
                    strings: Vec::new(),
                    rows: Vec::new(),
                })
                .collect(),
        )
    }
}

impl<T> Batch<T> {
    pub fn push(&mut self, people: &People, name: &[u8], data: T) {
        let (partition, person) = if let Some(person) = people.aliases.get(name) {
            (person as u32 as usize, (person >> 32) as u32)
        } else {
            let hash = people.hash.hash_one(name);
            let bucket = shard(hash);
            let partition = &mut self.0[bucket];
            let id = intern(&mut partition.names, &mut partition.strings, &people.hash, hash, name);
            (bucket, id | UNLINKED)
        };
        self.0[partition].rows.push(PersonRow { person, data });
    }
}

fn intern(
    table: &mut HashTable<u32>,
    strings: &mut Vec<Box<[u8]>>,
    state: &RandomState,
    hash: u64,
    name: &[u8],
) -> u32 {
    *table
        .entry(
            hash,
            |id| &*strings[*id as usize] == name,
            |id| state.hash_one(&*strings[*id as usize]),
        )
        .or_insert_with(|| {
            let id = u32::try_from(strings.len()).unwrap();
            assert!(id < UNLINKED);
            strings.push(name.into());
            id
        })
        .get()
}

// Each shard owns its linked names and merges the thread-local unlinked
// names from both inputs before grouping rows. Linked IDs already agree.
pub struct Names {
    pub strings: Vec<Box<[u8]>>,
    unlinked: HashTable<u32>,
    hash: RandomState,
}

impl Names {
    pub fn new(strings: Vec<Box<[u8]>>) -> Self {
        Self {
            strings,
            unlinked: HashTable::new(),
            hash: RandomState::new(),
        }
    }

    pub fn merge<T>(&mut self, parts: Vec<Partition<T>>) -> Vec<PersonRow<T>> {
        let mut merged = Vec::with_capacity(parts.iter().map(|part| part.rows.len()).sum());
        for part in parts {
            let remap: Vec<_> = part
                .strings
                .iter()
                .map(|name| {
                    let hash = self.hash.hash_one(&**name);
                    intern(&mut self.unlinked, &mut self.strings, &self.hash, hash, name)
                })
                .collect();
            merged.extend(part.rows.into_iter().map(|mut row| {
                if row.person & UNLINKED != 0 {
                    row.person = remap[(row.person ^ UNLINKED) as usize];
                }
                row
            }));
        }
        merged
    }
}
