//! A pack writer of the tests' own, written from `README.md`, "The pack format", and not
//! from the crate's code: it writes whatever it is given, so a test can break one rule
//! at a time, and the golden tests compare `PackBuilder`'s bytes against it.

use lactose_core::xxh64;

pub const TRAILER_LEN: usize = 24;

/// One index entry, every field as the format stores it.
#[derive(Clone, Debug)]
pub struct RawEntry {
    pub id: u64,
    pub version: u64,
    pub kind: u64,
    pub format: u32,
    pub dependencies: Vec<u64>,
    pub storage: u8,
    pub offset: u64,
    pub stored: u64,
    pub decoded: u64,
}

impl RawEntry {
    /// A raw entry of `len` bytes at `offset`, with no dependencies.
    pub fn raw(id: u64, version: u64, offset: u64, len: u64) -> Self {
        Self {
            id,
            version,
            kind: 1,
            format: 1,
            dependencies: Vec::new(),
            storage: 0,
            offset,
            stored: len,
            decoded: len,
        }
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.id.to_le_bytes());
        out.extend_from_slice(&self.version.to_le_bytes());
        out.extend_from_slice(&self.kind.to_le_bytes());
        out.extend_from_slice(&self.format.to_le_bytes());
        let count = u32::try_from(self.dependencies.len()).expect("a dependency count");
        out.extend_from_slice(&count.to_le_bytes());
        for dependency in &self.dependencies {
            out.extend_from_slice(&dependency.to_le_bytes());
        }
        out.push(self.storage);
        out.extend_from_slice(&self.offset.to_le_bytes());
        out.extend_from_slice(&self.stored.to_le_bytes());
        out.extend_from_slice(&self.decoded.to_le_bytes());
    }
}

/// An index: the count given, then the entries given.
pub fn index(count: u32, entries: &[RawEntry]) -> Vec<u8> {
    let mut out = count.to_le_bytes().to_vec();
    for entry in entries {
        entry.encode(&mut out);
    }
    out
}

/// A trailer for an index of `len` bytes hashing to `hash`.
pub fn trailer(len: u64, hash: u64, version: u16, reserved: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(TRAILER_LEN);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&hash.to_le_bytes());
    out.extend_from_slice(b"LPAK");
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&reserved.to_le_bytes());
    out
}

/// A well-formed file around `payload` and `index`.
pub fn seal(payload: &[u8], index: &[u8]) -> Vec<u8> {
    let mut out = payload.to_vec();
    out.extend_from_slice(index);
    let len = u64::try_from(index.len()).expect("an index length");
    out.extend_from_slice(&trailer(len, xxh64(index, 0), 1, 0));
    out
}

/// Where the index of a well-formed `pack` begins, and its length.
pub fn index_range(pack: &[u8]) -> (usize, usize) {
    let trailer = pack.len() - TRAILER_LEN;
    let len = u64::from_le_bytes(pack[trailer..trailer + 8].try_into().expect("a word"));
    let len = usize::try_from(len).expect("an index in memory");
    (trailer - len, len)
}

/// Rewrites the trailer's hash to match the index as it now stands.
pub fn reseal(pack: &mut [u8]) {
    let (start, len) = index_range(pack);
    let hash = xxh64(&pack[start..start + len], 0);
    let trailer = pack.len() - TRAILER_LEN;
    pack[trailer + 8..trailer + 16].copy_from_slice(&hash.to_le_bytes());
}

/// The number of dependencies the index entry at offset `entry` of `pack` declares.
pub fn dependency_count(pack: &[u8], entry: usize) -> usize {
    let count = u32::from_le_bytes(pack[entry + 28..entry + 32].try_into().expect("a word"));
    usize::try_from(count).expect("a count in memory")
}

/// The offset of entry `at` in the index of a well-formed `pack`.
pub fn entry_at(pack: &[u8], at: usize) -> usize {
    let (start, _) = index_range(pack);
    let mut entry = start + 4;
    for _ in 0..at {
        entry += 57 + 8 * dependency_count(pack, entry);
    }
    entry
}
