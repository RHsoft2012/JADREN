//! Source storage and byte-accurate spans for the Jadren compiler.

use std::fmt;
use std::path::{Path, PathBuf};

use jadren_determinism::fingerprint_bytes;

/// Stable identifier of a source file inside one compiler session.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceId(u32);

impl SourceId {
    /// Returns the zero-based numeric identifier.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// Half-open UTF-8 byte range in a source file.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Span {
    /// Source file containing the range.
    pub source: SourceId,
    /// Inclusive byte offset.
    pub start: usize,
    /// Exclusive byte offset.
    pub end: usize,
}

impl Span {
    /// Creates a span when the range is ordered.
    #[must_use]
    pub const fn new(source: SourceId, start: usize, end: usize) -> Option<Self> {
        if start <= end {
            Some(Self { source, start, end })
        } else {
            None
        }
    }

    /// Creates a zero-width span.
    #[must_use]
    pub const fn empty(source: SourceId, offset: usize) -> Self {
        Self {
            source,
            start: offset,
            end: offset,
        }
    }

    /// Returns the span length in UTF-8 bytes.
    #[must_use]
    pub const fn len(self) -> usize {
        self.end - self.start
    }

    /// Returns whether the span is empty.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// One-based human-readable location.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Location {
    /// One-based line number.
    pub line: usize,
    /// One-based Unicode scalar column.
    pub column: usize,
}

/// Immutable source file stored by a compiler session.
#[derive(Clone, Debug)]
pub struct SourceFile {
    id: SourceId,
    path: PathBuf,
    text: String,
    line_starts: Vec<usize>,
    stable_hash: u64,
}

impl SourceFile {
    fn new(id: SourceId, path: PathBuf, text: String) -> Self {
        let line_starts = compute_line_starts(&text);
        let stable_hash = fingerprint_bytes("jadren-source-v1", text.as_bytes()).as_u64();
        Self {
            id,
            path,
            text,
            line_starts,
            stable_hash,
        }
    }

    /// Returns the file identifier.
    #[must_use]
    pub const fn id(&self) -> SourceId {
        self.id
    }

    /// Returns the display path supplied to the source manager.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the UTF-8 source text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns a versioned deterministic FNV-1a fingerprint of the source bytes.
    #[must_use]
    pub const fn stable_hash(&self) -> u64 {
        self.stable_hash
    }

    /// Maps a byte offset to a one-based line and Unicode scalar column.
    #[must_use]
    pub fn location(&self, offset: usize) -> Option<Location> {
        if offset > self.text.len() || !self.text.is_char_boundary(offset) {
            return None;
        }

        let line_index = self
            .line_starts
            .partition_point(|line_start| *line_start <= offset)
            .saturating_sub(1);
        let line_start = self.line_starts[line_index];
        let column = self.text[line_start..offset].chars().count() + 1;
        Some(Location {
            line: line_index + 1,
            column,
        })
    }

    /// Returns one source line without its line terminator.
    #[must_use]
    pub fn line_text(&self, one_based_line: usize) -> Option<&str> {
        let line_index = one_based_line.checked_sub(1)?;
        let start = *self.line_starts.get(line_index)?;
        let end = self
            .line_starts
            .get(line_index + 1)
            .copied()
            .unwrap_or(self.text.len());
        Some(self.text[start..end].trim_end_matches(['\r', '\n']))
    }

    /// Returns the text covered by a valid span.
    #[must_use]
    pub fn slice(&self, span: Span) -> Option<&str> {
        if span.source != self.id
            || span.end > self.text.len()
            || !self.text.is_char_boundary(span.start)
            || !self.text.is_char_boundary(span.end)
        {
            return None;
        }
        self.text.get(span.start..span.end)
    }
}

/// Error produced while adding a source file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceError {
    /// The session exhausted its source identifier space.
    TooManyFiles,
}

impl fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyFiles => formatter.write_str("compiler session contains too many files"),
        }
    }
}

impl std::error::Error for SourceError {}

/// Owns all source files in one compiler session.
#[derive(Clone, Debug, Default)]
pub struct SourceManager {
    files: Vec<SourceFile>,
}

impl SourceManager {
    /// Creates an empty source manager.
    #[must_use]
    pub const fn new() -> Self {
        Self { files: Vec::new() }
    }

    /// Adds an in-memory or on-disk source using the supplied display path.
    pub fn add(
        &mut self,
        path: impl Into<PathBuf>,
        text: impl Into<String>,
    ) -> Result<SourceId, SourceError> {
        let index = u32::try_from(self.files.len()).map_err(|_| SourceError::TooManyFiles)?;
        let id = SourceId(index);
        self.files
            .push(SourceFile::new(id, path.into(), text.into()));
        Ok(id)
    }

    /// Returns a source file by identifier.
    #[must_use]
    pub fn get(&self, id: SourceId) -> Option<&SourceFile> {
        let index = usize::try_from(id.index()).ok()?;
        self.files.get(index)
    }

    /// Iterates over files in deterministic registration order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &SourceFile> {
        self.files.iter()
    }

    /// Returns the number of files in the session.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.files.len()
    }

    /// Returns whether the session has no files.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

fn compute_line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(index + 1);
        }
    }
    starts
}

#[cfg(test)]
mod tests {
    use super::{Location, SourceManager, Span};

    #[test]
    fn maps_crlf_and_unicode_locations() {
        let mut sources = SourceManager::new();
        let id = sources
            .add("memory.jdn", "fn main() {\r\n    let text = \"ľ\"\r\n}\r\n")
            .expect("source ID should fit");
        let file = sources.get(id).expect("source should exist");
        let offset = file.text().find('ľ').expect("test text contains ľ");

        assert_eq!(
            file.location(offset),
            Some(Location {
                line: 2,
                column: 17
            })
        );
        assert_eq!(file.line_text(2), Some("    let text = \"ľ\""));
    }

    #[test]
    fn rejects_invalid_span_boundaries() {
        let mut sources = SourceManager::new();
        let id = sources
            .add("memory.jdn", "ľ")
            .expect("source ID should fit");
        let file = sources.get(id).expect("source should exist");
        let inside_utf8 = Span::new(id, 1, 2).expect("ordered span");

        assert_eq!(file.slice(inside_utf8), None);
    }

    #[test]
    fn source_hash_is_stable_for_equal_text() {
        let mut sources = SourceManager::new();
        let left = sources.add("left", "same").expect("source ID should fit");
        let right = sources.add("right", "same").expect("source ID should fit");

        assert_eq!(
            sources.get(left).map(super::SourceFile::stable_hash),
            sources.get(right).map(super::SourceFile::stable_hash)
        );
    }
}
