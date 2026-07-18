//! Signed, deterministic toolchain artifact verification and installation.
//!
//! The crate deliberately has no network client and never executes an artifact.
//! Callers must provide an exact artifact path and a signed descriptor. The
//! installer verifies the bytes and Ed25519 signature before staging a copy and
//! atomically publishing a new, never-overwritten version directory.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

/// Current signed artifact manifest schema.
pub const SCHEMA_VERSION: u32 = 1;

/// A release artifact with immutable content and publisher identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactDescriptor {
    /// Human-readable toolchain component name.
    pub name: String,
    /// Semantic artifact version.
    pub version: String,
    /// Canonical target triple.
    pub target: String,
    /// Publisher/trust-root identity.
    pub publisher: String,
    /// Lowercase SHA-256 of the exact artifact bytes.
    pub sha256: String,
    /// Ed25519 public key, encoded as lowercase hexadecimal.
    pub public_key: String,
    /// Ed25519 signature over [`Self::canonical_payload`], lowercase hex.
    pub signature: String,
}

impl ArtifactDescriptor {
    /// Returns the stable bytes covered by the publisher signature.
    #[must_use]
    pub fn canonical_payload(&self) -> String {
        format!(
            "jadren-artifact\n schema={SCHEMA_VERSION}\n name={}\n version={}\n target={}\n publisher={}\n sha256={}\n",
            self.name, self.version, self.target, self.publisher, self.sha256
        )
    }

    /// Renders a strict, deterministic manifest suitable for committing.
    #[must_use]
    pub fn to_manifest(&self) -> String {
        format!(
            "schema = {SCHEMA_VERSION}\nname = {}\nversion = {}\ntarget = {}\npublisher = {}\nsha256 = {}\npublic_key = {}\nsignature = {}\n",
            quote(&self.name),
            quote(&self.version),
            quote(&self.target),
            quote(&self.publisher),
            quote(&self.sha256),
            quote(&self.public_key),
            quote(&self.signature),
        )
    }

    /// Parses the canonical manifest subset and rejects duplicate/unknown keys.
    pub fn parse_manifest(text: &str) -> Result<Self, InstallerError> {
        let mut schema = None;
        let mut name = None;
        let mut version = None;
        let mut target = None;
        let mut publisher = None;
        let mut sha256 = None;
        let mut public_key = None;
        let mut signature = None;
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line.split_once('=').ok_or_else(|| {
                InstallerError::Manifest(format!("line {} must be `key = value`", index + 1))
            })?;
            let key = key.trim();
            let value = value.trim();
            match key {
                "schema" => set_once(&mut schema, parse_u32(value, index + 1)?, key, index + 1)?,
                "name" => set_once(&mut name, parse_string(value, index + 1)?, key, index + 1)?,
                "version" => set_once(
                    &mut version,
                    parse_string(value, index + 1)?,
                    key,
                    index + 1,
                )?,
                "target" => set_once(&mut target, parse_string(value, index + 1)?, key, index + 1)?,
                "publisher" => set_once(
                    &mut publisher,
                    parse_string(value, index + 1)?,
                    key,
                    index + 1,
                )?,
                "sha256" => set_once(&mut sha256, parse_string(value, index + 1)?, key, index + 1)?,
                "public_key" => set_once(
                    &mut public_key,
                    parse_string(value, index + 1)?,
                    key,
                    index + 1,
                )?,
                "signature" => set_once(
                    &mut signature,
                    parse_string(value, index + 1)?,
                    key,
                    index + 1,
                )?,
                _ => {
                    return Err(InstallerError::Manifest(format!(
                        "line {} has unsupported key `{key}`",
                        index + 1
                    )));
                }
            }
        }
        if schema != Some(SCHEMA_VERSION) {
            return Err(InstallerError::Manifest(format!(
                "expected schema {SCHEMA_VERSION}"
            )));
        }
        let descriptor = Self {
            name: required(name, "name")?,
            version: required(version, "version")?,
            target: required(target, "target")?,
            publisher: required(publisher, "publisher")?,
            sha256: required(sha256, "sha256")?,
            public_key: required(public_key, "public_key")?,
            signature: required(signature, "signature")?,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    fn validate(&self) -> Result<(), InstallerError> {
        for (field, value) in [
            ("name", self.name.as_str()),
            ("version", self.version.as_str()),
            ("target", self.target.as_str()),
            ("publisher", self.publisher.as_str()),
        ] {
            if value.is_empty() || value.contains(['\r', '\n']) {
                return Err(InstallerError::Manifest(format!(
                    "{field} must be a non-empty single line"
                )));
            }
        }
        if self.sha256.len() != 64 || !is_lower_hex(&self.sha256) {
            return Err(InstallerError::Manifest(
                "sha256 must be 64 lowercase hexadecimal characters".to_owned(),
            ));
        }
        if self.public_key.len() != 64 || !is_lower_hex(&self.public_key) {
            return Err(InstallerError::Manifest(
                "public_key must be 64 lowercase hexadecimal characters".to_owned(),
            ));
        }
        if self.signature.len() != 128 || !is_lower_hex(&self.signature) {
            return Err(InstallerError::Manifest(
                "signature must be 128 lowercase hexadecimal characters".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Computes the lowercase SHA-256 digest of bytes.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    encode_hex(&digest)
}

/// Verifies descriptor identity, digest and Ed25519 signature for bytes.
pub fn verify_bytes(descriptor: &ArtifactDescriptor, bytes: &[u8]) -> Result<(), InstallerError> {
    descriptor.validate()?;
    let actual = sha256_hex(bytes);
    if actual != descriptor.sha256 {
        return Err(InstallerError::DigestMismatch {
            expected: descriptor.sha256.clone(),
            actual,
        });
    }
    let public_key = decode_fixed::<32>(&descriptor.public_key, "public_key")?;
    let signature = decode_fixed::<64>(&descriptor.signature, "signature")?;
    let key = VerifyingKey::from_bytes(&public_key)
        .map_err(|error| InstallerError::Signature(format!("invalid public key: {error}")))?;
    key.verify(
        descriptor.canonical_payload().as_bytes(),
        &Signature::from_bytes(&signature),
    )
    .map_err(|error| InstallerError::Signature(format!("signature verification failed: {error}")))
}

/// Verifies an artifact file before any installation mutation.
pub fn verify_file(
    descriptor: &ArtifactDescriptor,
    artifact: impl AsRef<Path>,
) -> Result<(), InstallerError> {
    let path = artifact.as_ref();
    let bytes = fs::read(path).map_err(|error| InstallerError::Io {
        path: path.to_path_buf(),
        error,
    })?;
    verify_bytes(descriptor, &bytes)
}

/// Installs a verified artifact into a new version directory.
pub fn install_file(
    descriptor: &ArtifactDescriptor,
    artifact: impl AsRef<Path>,
    root: impl AsRef<Path>,
) -> Result<PathBuf, InstallerError> {
    let artifact = artifact.as_ref();
    let root = root.as_ref();
    descriptor.validate()?;
    validate_component(&descriptor.name, "name")?;
    validate_component(&descriptor.version, "version")?;
    validate_component(&descriptor.target, "target")?;
    verify_file(descriptor, artifact)?;
    let root = root
        .canonicalize()
        .or_else(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                fs::create_dir_all(root).map(|()| root.to_path_buf())
            } else {
                Err(error)
            }
        })
        .map_err(|error| InstallerError::Io {
            path: root.to_path_buf(),
            error,
        })?;
    let component_root = root.join(&descriptor.name);
    let version_root = component_root.join(&descriptor.version);
    let destination = version_root.join(format!("{}.artifact", descriptor.target));
    if destination.exists() {
        return Err(InstallerError::AlreadyInstalled(destination));
    }
    let staging = root.join(format!(
        ".staging-{}-{}-{}",
        descriptor.name,
        descriptor.version,
        std::process::id()
    ));
    if staging.exists() {
        return Err(InstallerError::StagingExists(staging));
    }
    fs::create_dir_all(&staging).map_err(|error| InstallerError::Io {
        path: staging.clone(),
        error,
    })?;
    let staged_file = staging.join(destination.file_name().expect("destination has filename"));
    let result = (|| {
        fs::copy(artifact, &staged_file).map_err(|error| InstallerError::Io {
            path: staged_file.clone(),
            error,
        })?;
        fs::create_dir_all(&component_root).map_err(|error| InstallerError::Io {
            path: component_root.clone(),
            error,
        })?;
        fs::rename(&staging, &version_root).map_err(|error| InstallerError::Io {
            path: version_root.clone(),
            error,
        })?;
        Ok(destination.clone())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn validate_component(value: &str, field: &str) -> Result<(), InstallerError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains(['/', '\\', ':'])
        || Path::new(value)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(InstallerError::Manifest(format!(
            "{field} is not a safe path component"
        )));
    }
    Ok(())
}

fn set_once<T>(
    slot: &mut Option<T>,
    value: T,
    key: &str,
    line: usize,
) -> Result<(), InstallerError> {
    if slot.replace(value).is_some() {
        return Err(InstallerError::Manifest(format!(
            "line {line} repeats key `{key}`"
        )));
    }
    Ok(())
}

fn required<T>(value: Option<T>, key: &str) -> Result<T, InstallerError> {
    value.ok_or_else(|| InstallerError::Manifest(format!("missing key `{key}`")))
}

fn parse_u32(value: &str, line: usize) -> Result<u32, InstallerError> {
    value
        .parse()
        .map_err(|_| InstallerError::Manifest(format!("line {line} has invalid schema number")))
}

fn parse_string(value: &str, line: usize) -> Result<String, InstallerError> {
    if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        return Err(InstallerError::Manifest(format!(
            "line {line} requires a quoted string"
        )));
    }
    let inner = &value[1..value.len() - 1];
    if inner.contains(['\r', '\n', '"']) {
        return Err(InstallerError::Manifest(format!(
            "line {line} contains unsupported string character"
        )));
    }
    Ok(inner.to_owned())
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn is_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn decode_fixed<const N: usize>(value: &str, field: &str) -> Result<[u8; N], InstallerError> {
    let mut output = [0_u8; N];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_value(chunk[0]).ok_or_else(|| {
            InstallerError::Manifest(format!("{field} contains invalid hexadecimal"))
        })?;
        let low = hex_value(chunk[1]).ok_or_else(|| {
            InstallerError::Manifest(format!("{field} contains invalid hexadecimal"))
        })?;
        output[index] = (high << 4) | low;
    }
    Ok(output)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

/// Verification/install failures.
#[derive(Debug)]
pub enum InstallerError {
    /// Strict manifest parse/validation failure.
    Manifest(String),
    /// File system operation failed.
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
    /// Content digest does not match the signed descriptor.
    DigestMismatch { expected: String, actual: String },
    /// Publisher key or signature is invalid.
    Signature(String),
    /// The exact version is already installed and will not be overwritten.
    AlreadyInstalled(PathBuf),
    /// A previous staging directory exists and must be inspected manually.
    StagingExists(PathBuf),
}

impl fmt::Display for InstallerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest(message) => write!(formatter, "manifest error: {message}"),
            Self::Io { path, error } => {
                write!(formatter, "I/O error at `{}`: {error}", path.display())
            }
            Self::DigestMismatch { expected, actual } => {
                write!(
                    formatter,
                    "artifact digest mismatch: expected {expected}, got {actual}"
                )
            }
            Self::Signature(message) => write!(formatter, "signature error: {message}"),
            Self::AlreadyInstalled(path) => write!(
                formatter,
                "artifact already installed at `{}`",
                path.display()
            ),
            Self::StagingExists(path) => write!(
                formatter,
                "staging directory already exists at `{}`",
                path.display()
            ),
        }
    }
}

impl Error for InstallerError {}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn descriptor(bytes: &[u8]) -> ArtifactDescriptor {
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let mut descriptor = ArtifactDescriptor {
            name: "llvm".to_owned(),
            version: "22.1.8".to_owned(),
            target: "x86_64-pc-windows-msvc".to_owned(),
            publisher: "jadren-release".to_owned(),
            sha256: sha256_hex(bytes),
            public_key: encode_hex(&signing_key.verifying_key().to_bytes()),
            signature: String::new(),
        };
        descriptor.signature = encode_hex(
            &signing_key
                .sign(descriptor.canonical_payload().as_bytes())
                .to_bytes(),
        );
        descriptor
    }

    #[test]
    fn signed_descriptor_roundtrips_and_verifies() {
        let bytes = b"toolchain";
        let descriptor = descriptor(bytes);
        assert_eq!(
            ArtifactDescriptor::parse_manifest(&descriptor.to_manifest()).unwrap(),
            descriptor
        );
        verify_bytes(&descriptor, bytes).unwrap();
    }

    #[test]
    fn digest_and_signature_tampering_are_rejected() {
        let bytes = b"toolchain";
        let descriptor = descriptor(bytes);
        assert!(matches!(
            verify_bytes(&descriptor, b"tampered"),
            Err(InstallerError::DigestMismatch { .. })
        ));
        let mut tampered = descriptor.clone();
        tampered.publisher = "attacker".to_owned();
        assert!(matches!(
            verify_bytes(&tampered, bytes),
            Err(InstallerError::Signature(_))
        ));
    }

    #[test]
    fn installer_stages_and_never_overwrites_version() {
        let temp = std::env::temp_dir().join(format!("jadren-installer-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();
        let artifact_path = temp.join("llvm.bin");
        fs::write(&artifact_path, b"toolchain").unwrap();
        let descriptor = descriptor(b"toolchain");
        let root = temp.join("toolchains");
        let installed = install_file(&descriptor, &artifact_path, &root).unwrap();
        assert_eq!(fs::read(&installed).unwrap(), b"toolchain");
        assert!(matches!(
            install_file(&descriptor, &artifact_path, &root),
            Err(InstallerError::AlreadyInstalled(_))
        ));
        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn unsafe_components_are_rejected() {
        let mut name_descriptor = descriptor(b"toolchain");
        name_descriptor.name = "../escape".to_owned();
        assert!(matches!(
            install_file(&name_descriptor, "missing", "root"),
            Err(InstallerError::Manifest(_)) | Err(InstallerError::Io { .. })
        ));
        let mut target_descriptor = descriptor(b"toolchain");
        target_descriptor.target = "..\\escape".to_owned();
        assert!(matches!(
            install_file(&target_descriptor, "missing", "root"),
            Err(InstallerError::Manifest(_)) | Err(InstallerError::Io { .. })
        ));
    }
}
