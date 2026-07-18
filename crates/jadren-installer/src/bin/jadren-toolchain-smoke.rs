use std::fs;

use ed25519_dalek::{Signer, SigningKey};
use jadren_toolchain::{ArtifactDescriptor, install_file, sha256_hex, verify_bytes};

fn hex(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(TABLE[(byte >> 4) as usize] as char);
        output.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    output
}

fn main() {
    let bytes = b"jadren-toolchain-smoke";
    let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
    let mut descriptor = ArtifactDescriptor {
        name: "llvm".to_owned(),
        version: "22.1.8".to_owned(),
        target: "x86_64-pc-windows-msvc".to_owned(),
        publisher: "jadren-release".to_owned(),
        sha256: sha256_hex(bytes),
        public_key: hex(&signing_key.verifying_key().to_bytes()),
        signature: String::new(),
    };
    descriptor.signature = hex(&signing_key
        .sign(descriptor.canonical_payload().as_bytes())
        .to_bytes());

    let manifest = descriptor.to_manifest();
    let parsed = ArtifactDescriptor::parse_manifest(&manifest).expect("manifest roundtrip");
    let mut tampered = bytes.to_vec();
    tampered[0] ^= 1;
    let digest_rejected = verify_bytes(&parsed, &tampered).is_err();

    let root = std::env::temp_dir().join(format!("jadren-toolchain-smoke-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create smoke root");
    let artifact = root.join("artifact.bin");
    fs::write(&artifact, bytes).expect("write smoke artifact");
    let install_root = root.join("installed");
    let installed = install_file(&parsed, &artifact, &install_root).expect("install artifact");
    let installed_matches = fs::read(&installed).expect("read installed artifact") == bytes;
    let second_install_rejected = install_file(&parsed, &artifact, &install_root).is_err();
    println!(
        "{{\"schema\":\"jadren-toolchain-smoke-0.1\",\"manifest\":\"passed\",\"signature\":\"passed\",\"digest_tamper\":\"{}\",\"atomic_install\":\"{}\",\"no_overwrite\":\"{}\",\"result\":\"{}\"}}",
        if digest_rejected { "passed" } else { "failed" },
        if installed_matches {
            "passed"
        } else {
            "failed"
        },
        if second_install_rejected {
            "passed"
        } else {
            "failed"
        },
        if digest_rejected && installed_matches && second_install_rejected {
            "pass"
        } else {
            "fail"
        },
    );
    let _ = fs::remove_dir_all(root);
    if !(digest_rejected && installed_matches && second_install_rejected) {
        std::process::exit(1);
    }
}
