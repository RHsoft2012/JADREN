//! Sign one immutable release artifact descriptor with an external Ed25519 key.
//!
//! The key is read from a caller-owned file containing exactly 32 bytes as
//! 64 lowercase/uppercase hexadecimal characters. It is never accepted on the
//! command line and is never copied into the output manifest.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use ed25519_dalek::{Signer, SigningKey};
use jadren_toolchain::{ArtifactDescriptor, sha256_hex};

fn usage() -> &'static str {
    "usage: jadren-release-manifest --artifact <path> --name <name> --version <version> --target <target> --publisher <publisher> --key-file <hex-file> --output <manifest>"
}

fn argument(args: &[String], name: &str) -> Result<String, String> {
    let Some(index) = args.iter().position(|value| value == name) else {
        return Err(format!("missing {name}\n{}", usage()));
    };
    args.get(index + 1)
        .filter(|value| !value.is_empty() && !value.starts_with('-'))
        .cloned()
        .ok_or_else(|| format!("missing value for {name}\n{}", usage()))
}

fn decode_hex_key(value: &str) -> Result<[u8; 32], String> {
    let value = value.trim();
    if value.len() != 64 {
        return Err("key file must contain exactly 64 hexadecimal characters".to_owned());
    }
    let mut output = [0_u8; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        let offset = index * 2;
        *byte = u8::from_str_radix(&value[offset..offset + 2], 16)
            .map_err(|_| format!("key file contains non-hex data at byte {index}"))?;
    }
    Ok(output)
}

fn write_new_file(path: &Path, contents: &str) -> Result<(), String> {
    if path.exists() {
        return Err(format!(
            "refusing to overwrite existing manifest `{}`",
            path.display()
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("manifest has no parent directory: `{}`", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create manifest directory: {error}"))?;
    let staging = parent.join(format!(
        ".{}.staging-{}",
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("manifest"),
        std::process::id()
    ));
    if staging.exists() {
        return Err(format!(
            "staging file already exists: `{}`",
            staging.display()
        ));
    }
    fs::write(&staging, contents)
        .map_err(|error| format!("failed to write staging manifest: {error}"))?;
    if let Err(error) = fs::rename(&staging, path) {
        let _ = fs::remove_file(&staging);
        return Err(format!("failed to publish manifest atomically: {error}"));
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let artifact = PathBuf::from(argument(&args, "--artifact")?);
    let name = argument(&args, "--name")?;
    let version = argument(&args, "--version")?;
    let target = argument(&args, "--target")?;
    let publisher = argument(&args, "--publisher")?;
    let key_file = PathBuf::from(argument(&args, "--key-file")?);
    let output = PathBuf::from(argument(&args, "--output")?);
    if args.iter().any(|value| {
        value.starts_with('-')
            && ![
                "--artifact",
                "--name",
                "--version",
                "--target",
                "--publisher",
                "--key-file",
                "--output",
            ]
            .contains(&value.as_str())
    }) {
        return Err(format!("unknown option\n{}", usage()));
    }

    let bytes = fs::read(&artifact)
        .map_err(|error| format!("failed to read artifact `{}`: {error}", artifact.display()))?;
    let key_text = fs::read_to_string(&key_file)
        .map_err(|error| format!("failed to read external signing key: {error}"))?;
    let signing_key = SigningKey::from_bytes(&decode_hex_key(&key_text)?);
    let mut descriptor = ArtifactDescriptor {
        name,
        version,
        target,
        publisher,
        sha256: sha256_hex(&bytes),
        public_key: hex(&signing_key.verifying_key().to_bytes()),
        signature: String::new(),
    };
    descriptor.signature = hex(&signing_key
        .sign(descriptor.canonical_payload().as_bytes())
        .to_bytes());
    let manifest = descriptor.to_manifest();
    let parsed = ArtifactDescriptor::parse_manifest(&manifest)
        .map_err(|error| format!("generated manifest failed self-parse: {error}"))?;
    jadren_toolchain::verify_bytes(&parsed, &bytes)
        .map_err(|error| format!("generated manifest failed self-verify: {error}"))?;
    write_new_file(&output, &manifest)?;
    println!(
        "{{\"schema\":\"jadren-release-manifest-0.1\",\"manifest\":{},\"artifact\":{},\"sha256\":\"{}\",\"status\":\"signed-local\",\"result\":\"pass\"}}",
        json_quote(&output.display().to_string()),
        json_quote(&artifact.display().to_string()),
        parsed.sha256,
    );
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(TABLE[(byte >> 4) as usize] as char);
        output.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    output
}

fn json_quote(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\r', "\\r")
            .replace('\n', "\\n")
    )
}

fn main() {
    if let Err(error) = run() {
        eprintln!("jadren-release-manifest: {error}");
        std::process::exit(1);
    }
}
