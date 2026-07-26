# Public roadmap

This roadmap intentionally describes user-facing outcomes. Detailed task IDs,
engineering checklists, audit evidence, and security-sensitive deployment notes
remain in the private development repository.

## Compiler preview

- harden the draft language and diagnostics;
- complete repeatable Windows and Linux build validation;
- publish a clean command-line developer preview;
- maintain scalar and SIMD fallback correctness.

## Unity preview

- stabilize the Unity packages and sample scenes;
- validate native lifecycle behaviour in Editor and Player builds;
- publish reproducible comparisons with equivalent Burst workloads;
- improve batched animation and GPU skinning samples.

## Mobile and GPU validation

- expand physical Android ARM64/NEON testing beyond the recorded three-device smoke matrix;
- measure thermal and battery behaviour on representative devices;
- expand bounded Vulkan, DirectX 12, and Metal execution evidence;
- publish support only for combinations verified on matching hardware.

## Public release readiness

- complete name and legal review;
- finish security review and private vulnerability reporting setup;
- sign release artifacts and publish checksums;
- provide English installation, learning, API, and migration documentation;
- release through a controlled download and rollback process.

Dates will be announced only after the corresponding release gates are complete.
