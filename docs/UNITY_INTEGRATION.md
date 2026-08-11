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

For a guided first pass, use the public [Unity lessons](https://jadren.rhsoft.eu/lessons.html):
scene commands are in Lesson 08, UI events and state in Lesson 09, Editor
Preview/Apply tooling in Lesson 10, and the complete Unity 6 project workflow
in Lesson 11.

## Bounded scene/UI command host

For runtime scene and interface work, the Unity package uses an explicit
numeric registry and bounded command buffers. A Jadren program writes records
into caller-owned buffers; a `JadrenUnityCommandExecutor` or
`JadrenUnityUiCommandExecutor` applies them on Unity's main thread. The host
owns `GameObject` references, prefab/component registration, lifetime and
thread-affinity validation.

The current scene command set can:

- spawn a registered prefab or create one of Unity's six primitive types;
- set position, rotation and scale, toggle active state, set a parent, and
  destroy a previously mapped entity;
- add only a component type explicitly approved by the host registry;
- set whitelisted float/bool component properties and renderer colours.

The UI command set can target registered controls and update colour, active
state, interactability and UTF-8 text. Separate bounded list/table buffers
support clearing and updating rows/cells, while the event router sends
pointer-enter/exit, click, submit/cancel, select/deselect, value-changed and
scroll events back through caller-owned buffers. The `UnityTransform` and
`UnityColor` Jadren values are source-level conveniences that lower to the
same fixed ABI records; they do not introduce reflection or an unbounded
runtime object graph.

`UnitySceneNode` groups an entity/prefab handle, optional parent, transform and
active intent and lowers to the minimum spawn/parent/active batch. `UnityUiStyle`
groups one target, colour, active state and interactability and lowers to three
fixed UI records. Both helpers return the number of records written and reject
an insufficient caller-owned buffer.

`UnityTextRange` and `UnityUiTextValue` apply the same rule to text: the
descriptor carries only a validated offset/length into a caller-owned UTF-8
pool and the Unity host decodes it on the main thread.

`UnityUiListItemValue` and `UnityUiTableCellValue` use the same typed range for
dynamic rows and cells. Their writers emit one bounded list/table record while
Unity remains responsible for prefab reuse, layout and main-thread lifetime.

The normal frame boundary is explicit: Jadren produces a bounded batch,
publishes it through the mailbox/scheduler, and a Unity `Update`/`LateUpdate`
host consumes and applies it. The reverse path is the same for UI events. No
Unity API is called from worker code, and an unknown id, duplicate registry
entry, invalid offset or full buffer is rejected instead of mutating arbitrary
scene state.

This runtime facade is complemented by a small Editor authoring layer. An
`JadrenUnityEditorScenePlan` is built without side effects and validated by a
dry-run before `JadrenUnityEditorPlanApplier.TryApply` is called explicitly.
The Editor layer accepts only bound scene objects, six Unity primitive types,
project prefab assets, transforms, active state and parent relationships. It
also supports bounded UGUI Canvas, Image panel, Text, Button, Toggle, Slider and
InputField authoring plus explicit text, color and interactable updates. Toggle
checked state and Slider value are bounded and validated before apply. A numeric allowlist also covers
Rigidbody and BoxCollider creation plus mass/contact-offset and bool-property
mutations. `save_prefab_asset` can save a known scene handle as a new
project-relative `Assets/*.prefab` asset in an existing folder; existing assets
and absolute paths are rejected. The same plan can create a bounded
`JadrenUnityUiEventRouter` and attach an explicit `JadrenUnityUiEventSource` to
a known UI handle. The router buffers pointer, click, submit, focus, scroll
and value-change records for a Jadren consumer; it does not invoke arbitrary
Jadren callbacks or discover targets through reflection. It uses one Unity Undo group and marks changed scenes dirty. It does
not infer components or assets through reflection; runtime UI/list/table
updates still use the bounded main-thread executors above. After an explicit
numeric source attachment, `Toggle.onValueChanged` and `Slider.onValueChanged`
are projected as bounded numeric events. The
`attach_ui_text_event_source` operation attaches a separate caller-owned UTF-8
buffer for legacy UGUI `InputField` `TextChanged` readback; `TMP_InputField`
continues to use its separate runtime adapter.

For a practical two-way frame path, add `JadrenUnityUiStateBindingHost` and
declare `JadrenUnityUiStateBinding` entries in the Inspector. Each binding gets
its own bounded snapshot, calls one generated Jadren reducer, and projects the
validated command on Unity's main thread before the host clears the shared
router frame. The supported projections are Toggle-to-active, Slider-to-color,
Slider-to-registered component float/bool, and InputField-to-text. This is an
explicit host schedule, not a hidden subscription graph.
For a custom Windows x64 build, call `TryAttachNativePlugin` with an explicit
`JadrenNativePluginManifest.json` path. Runtime verifies manifest identity, the
five reducer export names and SHA-256/size facts for both DLLs before loading.
`DetachNativePlugin` releases it and restores the compatibility bridge. DLL
discovery, arbitrary exports and hidden callbacks are not part of the contract.

The Tooling window also accepts the strict `jadren-unity-editor-plan-0.1` JSON
manifest. A Jadren file/export step may write this manifest inside the Unity
project; the Editor then offers **Preview plan** and **Apply plan (Undo)**.
Prefab paths are restricted to project-relative `Assets/` prefab assets and
JSON cannot bind arbitrary scene objects. Existing bindings must be supplied
by an explicit C# host reference.

The same boundary is available from Jadren source through
`UnityEditorPlanCommandRecord`: a program can fill a bounded 64-byte command
buffer and a caller-owned UTF-8 asset pool for primitive creation, prefab
instantiation, transforms, active state, parenting, bounded UI text and
allowlisted component and prefab-asset operations. The Unity host validates that
stream and translates it to the same Editor plan
before any Undo-scoped mutation. This is a typed command ABI, not reflection
or an implicit scene write.

The Editor plugin linker reads the explicit `nativeExports` allowlist from the
`JadrenBuildProfile` asset. A successful build writes
`JadrenNativePluginManifest.json` beside the native DLL pair. The deterministic
manifest records the Assets-relative source fingerprint, profile and ABI,
sorted exports (with the first configured export recorded as the primary
bridge), and SHA-256/size facts for both DLLs. The Editor tooling validates this
manifest before a project-local plugin is loaded.

For projects with several independently built Jadren sources, **Write native
release manifest** writes `Assets/JadrenGenerated/JadrenNativePluginReleaseManifest.json`.
The deterministic index contains only validated child manifests, source
identity, child-manifest checksums, and one shared Windows x86-64 profile/ABI.
It never calls `LoadLibrary`; loading remains an explicit per-source action.

The `com.jadren.unity` AgentSimulation sample also includes an optional
`AgentSimulationSoaParallelNativeRunner.cs`. It schedules disjoint SoA chunks
with Unity `IJobParallelFor` over the existing four-lane native ABI. Borrowed
leases remain alive until `Schedule(...).Complete()`; the sample validates
range disjointness and does not create native worker threads. This is a
lifecycle/correctness sample, not a universal ARM64 or Burst speedup claim.

## Package distribution

The public Downloads page provides a versioned **Unity Integration Bundle**.
It contains `com.jadren.unity`, `com.jadren.unity.tooling`,
`com.jadren.animation`, and the optional `com.jadren.agent` package, together
with the native plugins, samples, English documentation, licenses and an
installation manifest. Copy the four directories from `packages/` into a
Unity project's `Packages/` directory, or add them one at a time as local UPM
packages. The archive SHA-256 is listed in the release catalog.

The bundle is an unsigned development preview. Asset Store review, signing,
stable compatibility and production support are separate gates. The complete
development source is also published in this repository under `unity/`, so the
package source, native preview plugins and samples can be inspected or used as
local UPM packages without waiting for an Asset Store submission.

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
