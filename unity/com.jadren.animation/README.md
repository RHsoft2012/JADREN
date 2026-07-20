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

The native pose ABI also exposes an explicit 320-byte `AnimationPoseTile8`
AoSoA tile (ten eight-lane `Float8` fields) and a caller-owned linear blend
export. The blend clamps its weight to `0..1`; scalar packed poses remain the
tail and low-count fallback. This is an ABI/layout milestone, not a general
Animator or GPU performance claim.

The Unity crowd worker can opt into this tile path for position and scale
during bounded cross-fades. Quaternion rotation continues to use the exact
shortest-arc `SlerpUnclamped` contract. Native Slerp and pose tiles are separate
capabilities, and missing exports or extrapolated weights use the managed
fallback without publishing a partial pose.

The current per-character tile bridge is experimental and disabled by default.
Internal Editor measurements found that packing and one native boundary per
character outweighed the vector blend. The next CPU design must aggregate many
characters per call; no speedup is claimed for the current bridge.

An aggregate weighted crowd call was also measured. It removed the per-character
native boundary but remained slower because clip sampling and bidirectional
AoS/AoSoA conversion stayed managed. The next performance path must consume a
packed clip stream and write final skin matrices directly; no speedup is claimed
for either experimental tile bridge.

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
