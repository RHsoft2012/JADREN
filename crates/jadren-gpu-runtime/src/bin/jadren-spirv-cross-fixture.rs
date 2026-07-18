//! Test-only process fixture for the shared SPIRV-Cross boundary.
//!
//! This binary deliberately does not translate SPIR-V. It validates the
//! shell-free argument contract and emits deterministic target-shaped source
//! so integration tests can exercise the success/cleanup path without making
//! a real SPIRV-Cross installation a prerequisite.

use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args_os();
    let _program = arguments.next();
    let input = arguments
        .next()
        .map(PathBuf::from)
        .ok_or("missing SPIR-V input path")?;
    let mut target = None;
    let mut entry = None;
    let mut output = None;
    let mut shader_model = None;

    while let Some(argument) = arguments.next() {
        match argument.to_string_lossy().as_ref() {
            "--msl" => target = Some("msl"),
            "--hlsl" => target = Some("hlsl"),
            "--shader-model" => {
                shader_model = Some(
                    arguments
                        .next()
                        .ok_or("missing HLSL shader model")?
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            "--entry" => {
                entry = Some(
                    arguments
                        .next()
                        .ok_or("missing source entry")?
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            "--rename-entry-point" => {
                let old_entry = arguments
                    .next()
                    .ok_or("missing entry rename source")?
                    .to_string_lossy()
                    .into_owned();
                let new_entry = arguments
                    .next()
                    .ok_or("missing entry rename target")?
                    .to_string_lossy()
                    .into_owned();
                let stage = arguments
                    .next()
                    .ok_or("missing entry rename stage")?
                    .to_string_lossy()
                    .into_owned();
                if old_entry != new_entry || stage != "comp" {
                    return Err("fixture requires identity compute entry rename".into());
                }
            }
            "--output" => {
                output = Some(PathBuf::from(
                    arguments.next().ok_or("missing source output path")?,
                ));
            }
            unknown => return Err(format!("unexpected fixture argument: {unknown}").into()),
        }
    }

    let _input_bytes = fs::read(input)?;
    let entry = entry.ok_or("missing source entry")?;
    let target = target.ok_or("missing source target")?;
    if target == "hlsl" && shader_model.as_deref() != Some("60") {
        return Err("fixture requires HLSL shader model 60".into());
    }
    let source = match target {
        "msl" => format!(
            "#include <metal_stdlib>\nusing namespace metal;\nkernel void {entry}(uint3 gid [[thread_position_in_grid]]) {{ (void)gid; }}\n"
        ),
        "hlsl" => format!(
            "[numthreads(1, 1, 1)]\nvoid {entry}(uint3 gid : SV_DispatchThreadID) {{ (void)gid; }}\n"
        ),
        _ => return Err(format!("unexpected fixture target: {target}").into()),
    };
    fs::write(output.ok_or("missing source output path")?, source)?;
    Ok(())
}
