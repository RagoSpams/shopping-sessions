use crate::input::{SHARDS, csv, lines, read_parts, shard};
use ahash::{AHashMap, RandomState};
use hashbrown::HashTable;
use std::io;
use std::path::Path;

const UNLINKED: u32 = 1 << 31;

#[derive(Clone, Copy)]
struct Person {
    shard: usize,
    index: u32,
}

pub struct People {
    aliases: AHashMap<Box<[u8]>, Person>,
    hash: RandomState,
    pub names: Vec<Vec<Box<[u8]>>>,
}

impl People {
    pub fn read(directory: &Path) -> io::Result<Self> {
        let mut people = Self {
            aliases: AHashMap::new(),
            hash: RandomState::new(),
            names: crate::input::empty_shards(),
        };
        let chunks = read_parts(directory)?;
        let mut ids = AHashMap::new();
        let mut components = Vec::new();
        for chunk in &chunks {
            for (_, row) in lines(chunk) {
                if let Some([first, second]) = csv::<2>(row)
                    && !first.is_empty()
                    && !second.is_empty()
                {
                    let left = link_id(&mut ids, &mut components, first);
                    let right = link_id(&mut ids, &mut components, second);
                    join(&mut components, left, right);
                }
            }
        }
        let mut canonical = vec![None; components.len()];
        for (id, &component) in components.iter().enumerate() {
            if component.parent != id {
                continue;
            }
            let name = component.minimum;
            let bucket = shard(people.hash.hash_one(name));
            let names = &mut people.names[bucket];
            canonical[id] = Some(Person {
                shard: bucket,
                index: u32::try_from(names.len()).unwrap(),
            });
            names.push(name.into());
        }
        people.aliases.reserve(ids.len());
        for (name, id) in ids {
            let person = canonical[root(&mut components, id)].unwrap();
            people.aliases.insert(name.into(), person);
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
    ids: &mut AHashMap<&'a [u8], usize>,
    components: &mut Vec<Component<'a>>,
    name: &'a [u8],
) -> usize {
    *ids.entry(name).or_insert_with(|| {
        let id = components.len();
        components.push(Component {
            parent: id,
            size: 1,
            minimum: name,
        });
        id
    })
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
            (person.shard, person.index)
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
