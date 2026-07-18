//! Deterministic release SBOM and provenance artifacts.
//!
//! The generator reads committed `Cargo.lock` text only. It does not resolve
//! dependencies, access a registry, execute build scripts, or scan the host.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use jadren_determinism::{Fingerprint, fingerprint_bytes};

/// Current Jadren SBOM schema.
pub const SBOM_SCHEMA: &str = "jadren-sbom-0.1";
/// Current Jadren provenance schema.
pub const PROVENANCE_SCHEMA: &str = "jadren-provenance-0.1";

/// One locked dependency/component.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Component {
    /// Cargo package name.
    pub name: String,
    /// Cargo package version.
    pub version: String,
    /// Registry URL or `workspace` for local packages.
    pub source: String,
    /// Registry checksum, when present.
    pub checksum: Option<String>,
}

/// Deterministic SBOM document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sbom {
    /// Sorted package components.
    pub components: Vec<Component>,
}

impl Sbom {
    /// Parses Cargo.lock package records without resolving anything.
    pub fn from_cargo_lock(text: &str) -> Result<Self, ReleaseError> {
        let mut components = Vec::new();
        let mut current: Option<ComponentBuilder> = None;
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line == "[[package]]" {
                if let Some(builder) = current.take() {
                    components.push(builder.finish(index + 1)?);
                }
                current = Some(ComponentBuilder::default());
                continue;
            }
            let Some(builder) = current.as_mut() else {
                continue;
            };
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "name" => builder.name = Some(parse_string(value.trim(), index + 1)?),
                "version" => builder.version = Some(parse_string(value.trim(), index + 1)?),
                "source" => builder.source = Some(parse_string(value.trim(), index + 1)?),
                "checksum" => builder.checksum = Some(parse_string(value.trim(), index + 1)?),
                _ => {}
            }
        }
        if let Some(builder) = current {
            components.push(builder.finish(text.lines().count() + 1)?);
        }
        components.sort();
        let mut seen = BTreeSet::new();
        components.retain(|component| seen.insert(component.clone()));
        if components.is_empty() {
            return Err(ReleaseError::Lockfile(
                "no [[package]] records found".to_owned(),
            ));
        }
        Ok(Self { components })
    }

    /// Returns canonical JSON with stable component ordering.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut output = String::from("{\"schema\":\"");
        output.push_str(SBOM_SCHEMA);
        output.push_str("\",\"format\":\"SPDX-lite\",\"components\":[");
        for (index, component) in self.components.iter().enumerate() {
            if index != 0 {
                output.push(',');
            }
            output.push_str("{\"name\":");
            output.push_str(&quote(&component.name));
            output.push_str(",\"version\":");
            output.push_str(&quote(&component.version));
            output.push_str(",\"source\":");
            output.push_str(&quote(&component.source));
            output.push_str(",\"checksum\":");
            match &component.checksum {
                Some(checksum) => output.push_str(&quote(checksum)),
                None => output.push_str("null"),
            }
            output.push('}');
        }
        output.push_str("]}\n");
        output
    }

    /// Stable fingerprint of canonical SBOM bytes.
    #[must_use]
    pub fn fingerprint(&self) -> Fingerprint {
        fingerprint_bytes("jadren-sbom-0.1", self.to_json().as_bytes())
    }
}

/// Release provenance bound to one SBOM and source/toolchain identities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Provenance {
    /// SBOM fingerprint.
    pub sbom_fingerprint: Fingerprint,
    /// Caller-supplied source revision/fingerprint.
    pub source_fingerprint: String,
    /// Pinned compiler/toolchain identity.
    pub toolchain: String,
}

impl Provenance {
    /// Creates provenance for an SBOM and explicit build identities.
    pub fn new(
        sbom: &Sbom,
        source_fingerprint: impl Into<String>,
        toolchain: impl Into<String>,
    ) -> Result<Self, ReleaseError> {
        let source_fingerprint = source_fingerprint.into();
        let toolchain = toolchain.into();
        if source_fingerprint.is_empty() || toolchain.is_empty() {
            return Err(ReleaseError::Provenance(
                "source fingerprint and toolchain must be non-empty".to_owned(),
            ));
        }
        Ok(Self {
            sbom_fingerprint: sbom.fingerprint(),
            source_fingerprint,
            toolchain,
        })
    }

    /// Returns canonical JSON provenance.
    #[must_use]
    pub fn to_json(&self) -> String {
        format!(
            "{{\"schema\":\"{PROVENANCE_SCHEMA}\",\"sbom_fingerprint\":\"{}\",\"source_fingerprint\":{},\"toolchain\":{}}}\n",
            self.sbom_fingerprint,
            quote(&self.source_fingerprint),
            quote(&self.toolchain),
        )
    }
}

#[derive(Default)]
struct ComponentBuilder {
    name: Option<String>,
    version: Option<String>,
    source: Option<String>,
    checksum: Option<String>,
}

impl ComponentBuilder {
    fn finish(self, line: usize) -> Result<Component, ReleaseError> {
        let name = self.name.ok_or_else(|| {
            ReleaseError::Lockfile(format!("package near line {line} has no name"))
        })?;
        let version = self.version.ok_or_else(|| {
            ReleaseError::Lockfile(format!("package `{name}` near line {line} has no version"))
        })?;
        Ok(Component {
            name,
            version,
            source: self.source.unwrap_or_else(|| "workspace".to_owned()),
            checksum: self.checksum,
        })
    }
}

fn parse_string(value: &str, line: usize) -> Result<String, ReleaseError> {
    if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        return Err(ReleaseError::Lockfile(format!(
            "line {line} requires a quoted string"
        )));
    }
    let inner = &value[1..value.len() - 1];
    if inner.contains(['\r', '\n']) {
        return Err(ReleaseError::Lockfile(format!(
            "line {line} contains an invalid string"
        )));
    }
    Ok(inner.to_owned())
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// SBOM/provenance generation failure.
#[derive(Debug)]
pub enum ReleaseError {
    /// Cargo.lock format is malformed or incomplete.
    Lockfile(String),
    /// Provenance identity is incomplete.
    Provenance(String),
}

impl fmt::Display for ReleaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lockfile(message) => write!(formatter, "lockfile error: {message}"),
            Self::Provenance(message) => write!(formatter, "provenance error: {message}"),
        }
    }
}

impl Error for ReleaseError {}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCK: &str = "version = 4\n\n[[package]]\nname = \"jadren-cli\"\nversion = \"0.1.0\"\ndependencies = []\n\n[[package]]\nname = \"sha2\"\nversion = \"0.10.9\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"abc\"\n";

    #[test]
    fn sbom_is_sorted_and_deterministic() {
        let sbom = Sbom::from_cargo_lock(LOCK).unwrap();
        assert_eq!(sbom.components[0].name, "jadren-cli");
        assert_eq!(sbom.components[0].source, "workspace");
        assert_eq!(
            sbom.to_json(),
            Sbom::from_cargo_lock(LOCK).unwrap().to_json()
        );
        assert_eq!(
            sbom.fingerprint(),
            Sbom::from_cargo_lock(LOCK).unwrap().fingerprint()
        );
    }

    #[test]
    fn provenance_binds_sbom_and_identities() {
        let sbom = Sbom::from_cargo_lock(LOCK).unwrap();
        let provenance = Provenance::new(&sbom, "source-abc", "rust-1.97+llvm-22.1.8").unwrap();
        assert!(provenance.to_json().contains("source-abc"));
        assert!(Provenance::new(&sbom, "", "toolchain").is_err());
    }

    #[test]
    fn malformed_lockfile_is_rejected() {
        assert!(Sbom::from_cargo_lock("version = 4\n").is_err());
        assert!(Sbom::from_cargo_lock("[[package]]\nname = \"x\"\n").is_err());
    }
}
