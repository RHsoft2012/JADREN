# Unity project guide

This guide is for a Unity user who wants to add Jadren to an existing project,
create a small animation test scene, and run the supplied validation scripts.
The examples target Unity 6; the current development fixtures were checked with
Unity `6000.4.8f1`. Jadren 0.1 is a development preview, so keep a backup or a
separate test branch before changing a production project.

## 1. Install the packages

Install the packages from the Unity Asset Store package supplied for the same
Jadren release line, then use Unity Package Manager to enable them:

1. Open **Window → Package Manager**.
2. Select **+ → Add package from disk...**.
3. Select the downloaded Jadren Unity package and follow its included
   `Documentation~/` installation notes.
4. Enable `com.jadren.animation` when animation or crowd rendering is required.
5. Let Unity finish importing and resolve the package assemblies before
   entering Play mode.

The native package is the general C#/native bridge. The animation package is
independent and adds the baked-rig, batch pose, GPU pose and procedural crowd
components. Do not copy DLLs from an old `Jadren_win` installer into a newer
Unity project; use the plugin files from the same package revision.

## 2. Minimal scene setup

Create a new scene or use an existing scene with these objects:

- **Main Camera** with a valid view of the test area;
- a **Directional Light** (or the lights required by the render pipeline);
- an empty root object named for the test, for example `Jadren Animation Test`;
- one `JadrenAnimationGpuCrowdAnimator` component on that root;
- the `CharacterPrefab` field assigned to a baked Jadren character prefab.

The prefab must contain an `Animator`, a `JadrenAnimationAuthoring` component
with a configured rig/controller, and at least one readable
`SkinnedMeshRenderer` with valid bones, bind poses and materials. The runtime
creates the crowd buffers; do not create one GameObject or one Animator per
agent for the GPU path.

Recommended first values:

```text
AgentCount          250
AgentColumns        25
AgentSpacing        1.6
AutoBuild           enabled
Animate             enabled
PreferGpuAnimation  enabled
PreferNativeSlerp   enabled
SourceLodLevel      0
DeltaTime           0.0166667
```

Start with 250 agents. Increase to 500 and 1,000 only after the scene is
visible and the fallback path has been checked. A missing GPU capability must
fall back to the managed evaluator; it must not make the scene invisible.

## 3. Ready-made SampleScene 3

The development Unity project contains `Assets/Scenes/SampleScene 3.unity`.
It already has the camera, light/volume setup, a
`Jadren SampleScene 3 Animation Test` root, the crowd animator, and a small
runtime ground/diagnostic overlay. The ground and the 250-agent crowd are
created at runtime, so no per-agent scene objects are required.

Open the scene and press **Play**. The overlay reports the selected route,
agent count, pose updates and draw submissions. If the Game view is empty,
first check the prefab reference, baked authoring data, camera position and the
Console for a `character_prefab_missing` or `character_baked_authoring_missing`
diagnostic.

## 4. Run the validation scripts

The editor runners are available from the Unity menu after scripts compile:

- **Jadren → Validation → SampleScene 3 Animation** checks scene loading,
  crowd build, GPU/CPU fallback, pose updates, draw submissions and visible
  pixels.
- **Jadren → Validation → SampleScene 3 Multi-Material + LOD** makes an
  in-memory clone of the real character, adds a second submesh/material and a
  two-level `LODGroup`, then checks the LOD-1 selection and rendered output.

The same runners can be used in automation without opening a second scene:

```powershell
Unity.exe -batchmode -force-d3d11 `
  -projectPath <unity-project> `
  -executeMethod Jadren.Unity.Samples.AgentSimulation.Editor.JadrenSampleScene3BatchRunner.Run `
  -logFile <output-log>

Unity.exe -batchmode -force-d3d11 `
  -projectPath <unity-project> `
  -executeMethod Jadren.Unity.Samples.AgentSimulation.Editor.JadrenSampleScene3MultiMaterialLodBatchRunner.Run `
  -logFile <output-log>
```

The runners write JSON reports in the project root unless the corresponding
report environment variable is supplied. Reports are fixture evidence only;
they are not a Rukhanka parity or public speedup claim.

## 5. Choosing the runtime path

Use the normal `JadrenAnimationGpuCrowdAnimator` path for a large compatible
crowd. Use the managed evaluator or the existing Animator bridge for a small
number of characters, special controller features, inverse kinematics, or
assets that are not baked for Jadren. Multi-material meshes produce one
procedural draw per submesh while sharing the pose stream. `SourceLodLevel`
selects the requested source LOD and safely falls back to an available renderer.

Keep scene objects and Unity API calls on the main thread. Native workers and
GPU dispatches must receive caller-owned contiguous data and must complete
before the main-thread applier consumes their results.

## 6. Test order and troubleshooting

Use this order when diagnosing a new project:

1. one character with the original Animator;
2. one character with Jadren authoring validated;
3. 250 agents with GPU preferred and managed fallback available;
4. multi-material/LOD validation;
5. Release Player build on the target architecture;
6. only then compare the same fixture with Burst or another animation system.

Common fixes:

- **MissingReference/empty Game view:** reassign the prefab and verify its
  `JadrenAnimationAuthoring` rig/controller assets.
- **Input System exception:** do not use legacy `UnityEngine.Input` in the
  sample controller; use the active Input System or no input for a benchmark.
- **Black or invisible crowd:** verify the camera frustum, draw bounds,
  material/shader support and that the GPU route has a CPU fallback.
- **Very low FPS:** inspect triangles, texture memory, Animator count and
  draw submissions before changing the language backend. A GPU animation path
  reduces update overhead but cannot make an overdraw-heavy scene cheap.
- **Android build succeeds but device evidence is missing:** an APK build is
  not a device/thermal/throughput result. Run the device matrix only with
  physical ARM64 devices and retain the reported class/ABI information.
- **The thermal harness exits early:** short benchmark players intentionally
  quit after their report. For a sustained workload, use the opt-in Intent
  launch (`LaunchBenchmarkMode=measured`, `LaunchBenchmarkMeasureSeconds=300`,
  and an explicit layout/activity); the default launch remains unchanged.
  A short probe is diagnostic only and cannot satisfy the strict 300-second
  thermal contract.

## 7. What is not promised by this guide

This preview does not promise that every Unity Animator feature, controller,
constraint, IK graph, shader, render pipeline or asset import is supported.
Measure Release Players on identical fixtures and hardware. Keep public claims
limited to the exact report scope; a local editor FPS number is not a general
compiler speedup claim.
