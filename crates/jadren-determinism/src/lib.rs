//! Reproducible hashing, ordering, and lexical path normalization.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

const FNV1A_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Versioned 64-bit deterministic fingerprint.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct Fingerprint(u64);

impl Fingerprint {
    /// Creates a fingerprint from its stable integer representation.
    #[must_use]
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }

    /// Returns the stable integer representation.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:016x}", self.0)
    }
}

/// Streaming FNV-1a hasher with explicit length-delimited field helpers.
#[derive(Clone, Copy, Debug)]
pub struct StableHasher {
    state: u64,
}

impl StableHasher {
    /// Creates an empty stable hash stream.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: FNV1A_OFFSET,
        }
    }

    /// Creates a stream separated from other hash purposes by a versioned domain.
    #[must_use]
    pub fn with_domain(domain: &str) -> Self {
        let mut hasher = Self::new();
        hasher.write_str(domain);
        hasher
    }

    /// Appends raw bytes. Chunking does not change the result.
    pub const fn update(&mut self, bytes: &[u8]) {
        let mut index = 0;
        while index < bytes.len() {
            self.state ^= bytes[index] as u64;
            self.state = self.state.wrapping_mul(FNV1A_PRIME);
            index += 1;
        }
    }

    /// Appends one length-delimited byte field.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.write_u64(bytes.len() as u64);
        self.update(bytes);
    }

    /// Appends one length-delimited UTF-8 field.
    pub fn write_str(&mut self, value: &str) {
        self.write_bytes(value.as_bytes());
    }

    /// Appends a fixed-width integer in little-endian order.
    pub const fn write_u64(&mut self, value: u64) {
        self.update(&value.to_le_bytes());
    }

    /// Appends a boolean with a canonical representation.
    pub const fn write_bool(&mut self, value: bool) {
        self.update(&[value as u8]);
    }

    /// Finishes the current stream.
    #[must_use]
    pub const fn finish(self) -> Fingerprint {
        Fingerprint(self.state)
    }
}

impl Default for StableHasher {
    fn default() -> Self {
        Self::new()
    }
}

/// Hashes one byte sequence in a versioned semantic domain.
#[must_use]
pub fn fingerprint_bytes(domain: &str, bytes: &[u8]) -> Fingerprint {
    let mut hasher = StableHasher::with_domain(domain);
    hasher.write_bytes(bytes);
    hasher.finish()
}

/// Map with deterministic key iteration order.
pub type DeterministicMap<K, V> = BTreeMap<K, V>;

/// Set with deterministic key iteration order.
pub type DeterministicSet<T> = BTreeSet<T>;

/// Normalizes a path lexically without filesystem access.
///
/// Separators become `/`, Windows drive letters become lowercase, `.` components
/// disappear, and safe `..` components collapse their predecessor.
#[must_use]
pub fn normalize_path(path: impl AsRef<Path>) -> String {
    let raw = path
        .as_ref()
        .as_os_str()
        .to_string_lossy()
        .replace('\\', "/");
    let (prefix, rest, rooted) = split_prefix(&raw);
    let mut components: Vec<&str> = Vec::new();

    for component in rest.split('/') {
        match component {
            "" | "." => {}
            ".." if components.last().is_some_and(|last| *last != "..") => {
                let _ = components.pop();
            }
            ".." if !rooted => components.push(component),
            ".." => {}
            _ => components.push(component),
        }
    }

    let body = components.join("/");
    match (prefix.as_str(), body.as_str()) {
        ("", "") => ".".to_owned(),
        (prefix, "") => prefix.to_owned(),
        ("", body) => body.to_owned(),
        (prefix, body) if prefix.ends_with('/') => format!("{prefix}{body}"),
        (prefix, body) => format!("{prefix}/{body}"),
    }
}

fn split_prefix(path: &str) -> (String, &str, bool) {
    if let Some(rest) = path.strip_prefix("//") {
        return ("//".to_owned(), rest, true);
    }
    if let Some(rest) = path.strip_prefix('/') {
        return ("/".to_owned(), rest, true);
    }
    let bytes = path.as_bytes();
    if bytes.get(1) == Some(&b':') && bytes[0].is_ascii_alphabetic() {
        let drive = (bytes[0] as char).to_ascii_lowercase();
        let rest = &path[2..];
        let rooted = rest.starts_with('/');
        return (
            if rooted {
                format!("{drive}:/")
            } else {
                format!("{drive}:")
            },
            rest.trim_start_matches('/'),
            rooted,
        );
    }
    (String::new(), path, false)
}

#[cfg(test)]
mod tests {
    use super::{DeterministicMap, Fingerprint, StableHasher, fingerprint_bytes, normalize_path};

    #[test]
    fn stable_hasher_has_a_golden_empty_value_and_chunk_independence() {
        assert_eq!(
            StableHasher::new().finish(),
            Fingerprint::from_u64(0xcbf2_9ce4_8422_2325)
        );

        let mut whole = StableHasher::new();
        whole.update(b"Jadren language");
        let mut chunked = StableHasher::new();
        chunked.update(b"Jadren ");
        chunked.update(b"language");
        assert_eq!(whole.finish(), chunked.finish());
        assert_eq!(whole.finish().to_string().len(), 16);
    }

    #[test]
    fn domains_and_field_boundaries_are_distinct() {
        assert_ne!(
            fingerprint_bytes("source-v1", b"abc"),
            fingerprint_bytes("config-v1", b"abc")
        );

        let mut left = StableHasher::with_domain("fields-v1");
        left.write_str("ab");
        left.write_str("c");
        let mut right = StableHasher::with_domain("fields-v1");
        right.write_str("a");
        right.write_str("bc");
        assert_ne!(left.finish(), right.finish());
    }

    #[test]
    fn ordered_map_and_paths_are_reproducible() {
        let mut values = DeterministicMap::new();
        values.insert("zeta", 1);
        values.insert("alpha", 2);
        assert_eq!(
            values.keys().copied().collect::<Vec<_>>(),
            ["alpha", "zeta"]
        );

        assert_eq!(
            normalize_path(r"C:\work\.\src\..\main.jdn"),
            "c:/work/main.jdn"
        );
        assert_eq!(normalize_path("./src/../main.jdn"), "main.jdn");
        assert_eq!(normalize_path("../../main.jdn"), "../../main.jdn");
    }
}
