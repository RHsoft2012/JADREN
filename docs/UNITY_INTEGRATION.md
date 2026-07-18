# Unity integration

Jadren integrates with Unity through a native library, a generated C ABI, and a
managed C# facade. The objective is to move suitable data-oriented work out of
per-object managed updates while preserving clear ownership and lifecycle rules.

## Intended workflow

1. Write and validate a Jadren kernel.
2. Compile it for the player's target architecture.
3. Generate or use the matching C ABI and C# facade.
4. Pass contiguous Unity data through a validated borrowed view.
5. Execute the kernel in batches.
6. Verify checksum and lifetime rules before using performance results.

## Package layout

The source tree contains Unity Package Manager packages under `unity/`:

- `com.jadren.unity` for native compute integration;
- `com.jadren.animation` for the independent Jadren animation runtime layer.

The packages are development previews. Public package distribution, signing,
and a stable compatibility promise are separate release gates.

## Performance guidance

Jadren is best suited to contiguous, data-oriented workloads. Replacing a C#
method without changing an object-heavy memory layout may provide little benefit
and can be slower. Compare Jadren with an equivalent Burst job using the same:

- data layout and precision;
- number of elements and iterations;
- build configuration;
- scheduling and synchronization boundary;
- target device and thermal state.

Do not treat editor FPS or a single frame as a compiler benchmark.

## Threading and Unity APIs

Worker code must not call Unity main-thread APIs. Scene objects, renderers, and
engine-owned resources are applied on the main thread after the native or GPU
work has completed.
