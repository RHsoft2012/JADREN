# Jadren Animation System

The Jadren Animation System is an independent Unity animation runtime developed
for large character groups. It does not depend on third-party animation assets.

## Design

- batch-oriented pose sampling;
- structure-of-arrays friendly native layout;
- scalar, AVX2, and ARM64/NEON backend boundaries;
- managed worker and main-thread pose applier separation;
- visibility-based full, reduced, and hidden update modes;
- opt-in GPU pose and skinning experiments;
- deterministic fallback when a backend is unavailable.

## Unity authoring

Unity Animator and imported clips can remain the authoring source. Runtime data
is converted into Jadren-owned batch structures, processed outside per-character
`MonoBehaviour.Update`, and applied through a controlled bridge.

## GPU scope

The current GPU work validates compute dispatch, completion, readback, reusable
buffers, material bindings, and procedural crowd rendering in bounded samples.
It is not yet a general replacement for every `SkinnedMeshRenderer`, Animator
Controller feature, inverse-kinematics system, or production rendering pipeline.

## Performance expectations

The system should reduce Animator and GameObject overhead when many compatible
characters share batched data and update policy. Actual gains depend on rig
complexity, visible character count, skinning path, render pipeline, and target
hardware. A public performance claim requires a clean, reproducible benchmark.
