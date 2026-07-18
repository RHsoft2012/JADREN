//! Explicit, non-destructive edition migration planning.
//!
//! Jadren 0.1 has one supported package edition (`2026`) and an earlier draft
//! spelling (`0.1-draft`). The planner changes only the manifest edition line;
//! it never guesses source rewrites or silently edits files.

use std::error::Error;
use std::fmt;

/// Current package edition.
pub const CURRENT_EDITION: &str = "2026";
/// Historical draft spelling accepted by the migration planner.
pub const DRAFT_EDITION: &str = "0.1-draft";

/// One deterministic text edit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationEdit {
    /// One-based line number.
    pub line: usize,
    /// Original line bytes without the trailing newline.
    pub before: String,
    /// Replacement line bytes without the trailing newline.
    pub after: String,
}

/// Planned manifest migration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationPlan {
    /// Canonical source edition.
    pub from: String,
    /// Canonical destination edition.
    pub to: String,
    /// Stable edits in source order.
    pub edits: Vec<MigrationEdit>,
    /// Resulting manifest text.
    pub output: String,
}

impl MigrationPlan {
    /// Whether migration would change the input.
    #[must_use]
    pub fn changed(&self) -> bool {
        !self.edits.is_empty()
    }
}

/// Plans a manifest-only edition migration.
pub fn plan_manifest(
    text: &str,
    from: impl AsRef<str>,
    to: impl AsRef<str>,
) -> Result<MigrationPlan, MigrationError> {
    let from = canonical_edition(from.as_ref())?;
    let to = canonical_edition(to.as_ref())?;
    if to != CURRENT_EDITION {
        return Err(MigrationError::UnsupportedTarget(to));
    }
    if from == to {
        return Ok(MigrationPlan {
            from,
            to,
            edits: Vec::new(),
            output: text.to_owned(),
        });
    }
    if from != DRAFT_EDITION {
        return Err(MigrationError::UnsupportedSource(from));
    }

    let mut output = String::with_capacity(text.len());
    let mut edits = Vec::new();
    for (index, line) in text.split_inclusive('\n').enumerate() {
        let line_ending = if line.ends_with("\r\n") {
            "\r\n"
        } else if line.ends_with('\n') {
            "\n"
        } else {
            ""
        };
        let candidate = line.strip_suffix(line_ending).unwrap_or(line);
        let trimmed = candidate.trim();
        if let Some(value) = trimmed.strip_prefix("edition")
            && value.trim_start().starts_with('=')
            && value.contains("\"0.1-draft\"")
        {
            let before = candidate.to_owned();
            let marker = "\"0.1-draft\"";
            let marker_start =
                candidate
                    .find(marker)
                    .ok_or_else(|| MigrationError::EditionNotFound {
                        expected: from.clone(),
                    })?;
            let marker_end = marker_start + marker.len();
            let after = format!(
                "{}\"{CURRENT_EDITION}\"{}",
                &candidate[..marker_start],
                &candidate[marker_end..]
            );
            output.push_str(&after);
            output.push_str(line_ending);
            edits.push(MigrationEdit {
                line: index + 1,
                before,
                after,
            });
        } else {
            output.push_str(line);
        }
    }
    if edits.is_empty() {
        return Err(MigrationError::EditionNotFound { expected: from });
    }
    Ok(MigrationPlan {
        from,
        to,
        edits,
        output,
    })
}

fn canonical_edition(value: &str) -> Result<String, MigrationError> {
    match value.trim().to_ascii_lowercase().as_str() {
        CURRENT_EDITION => Ok(CURRENT_EDITION.to_owned()),
        DRAFT_EDITION => Ok(DRAFT_EDITION.to_owned()),
        value => Err(MigrationError::UnsupportedEdition(value.to_owned())),
    }
}

/// Edition migration planning failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MigrationError {
    /// The requested source spelling is not known.
    UnsupportedSource(String),
    /// The requested destination is not supported.
    UnsupportedTarget(String),
    /// Either from/to spelling is unknown.
    UnsupportedEdition(String),
    /// Expected edition line was not found in the manifest.
    EditionNotFound { expected: String },
}

impl fmt::Display for MigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSource(value) => {
                write!(formatter, "unsupported source edition `{value}`")
            }
            Self::UnsupportedTarget(value) => {
                write!(formatter, "unsupported target edition `{value}`")
            }
            Self::UnsupportedEdition(value) => write!(formatter, "unsupported edition `{value}`"),
            Self::EditionNotFound { expected } => {
                write!(formatter, "manifest has no edition line for `{expected}`")
            }
        }
    }
}

impl Error for MigrationError {}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str =
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"0.1-draft\"\n";

    #[test]
    fn plans_only_the_explicit_manifest_edition_edit() {
        let plan = plan_manifest(MANIFEST, "0.1-draft", "2026").unwrap();
        assert!(plan.changed());
        assert_eq!(plan.edits.len(), 1);
        assert!(plan.output.contains("edition = \"2026\""));
        assert!(plan.output.contains("name = \"demo\""));
    }

    #[test]
    fn current_edition_is_a_noop_and_unknowns_fail() {
        let current = MANIFEST.replace("0.1-draft", "2026");
        assert!(!plan_manifest(&current, "2026", "2026").unwrap().changed());
        assert!(plan_manifest(MANIFEST, "2027", "2026").is_err());
        assert!(plan_manifest(MANIFEST, "0.1-draft", "2027").is_err());
        assert!(plan_manifest("[package]\nname = \"demo\"\n", "0.1-draft", "2026").is_err());
    }
}
