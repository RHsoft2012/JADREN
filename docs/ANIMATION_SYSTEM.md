# Jadren Animation System

The Jadren Animation System is an independent Unity animation runtime developed
for large character groups. It does not depend on third-party animation assets.

## Animation 0.2 bounded status

The 0.2 acceptance scope is complete for the bounded runtime path: Blend Tree
locomotion, action/event overrides, authored transition conditions, bounded
layers and sync, AvatarMask-aware two-bone IK, Rigidbody fixed/render bridging,
and Idle/Walk/Run/Jump cadence at 60/120 Hz are covered by the local Unity,
UPM, Windows and ARM64 evidence gates. Full Mecanim/Animation Rigging graph
parity, rendered 120-Hz/GPU/crowd-FPS parity and public release approval are
explicitly deferred to the 0.3 track.

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

Controller-state SoA callers can select `Scalar`, `Float4` or `Float8` through
`StepSoAWithProfile` for a measured A/B run. `StepSoARecommended` uses Float8,
then Float4, then scalar on ARM64 and keeps scalar on other architectures;
the method returns the profile actually executed so a benchmark cannot mistake
a fallback for the requested ISA. The automatic capability probe is cached
after the first successful selection and is therefore kept out of the per-frame
hot loop; if a hot-reloaded plugin loses the selected export, Jadren re-probes
the available profiles. This is a dispatch contract, not a cross-device
speedup claim.

An aggregate weighted crowd call was also measured. It removed the per-character
native boundary but remained slower because clip sampling and bidirectional
AoS/AoSoA conversion stayed managed. The next performance path must consume a
packed clip stream and write final skin matrices directly; no speedup is claimed
for either experimental tile bridge.

## Unity authoring

Unity Animator and imported clips can remain the authoring source. Runtime data
is converted into Jadren-owned batch structures, processed outside per-character
`MonoBehaviour.Update`, and applied through a controlled bridge.

## First Play-mode scene

Use the following order when preparing a scene. It avoids the common failure in
which a character appears in the editor but has no reliable pose owner in Play
mode.

1. Add a character with an enabled Unity `Animator`, a
   `RuntimeAnimatorController`, its `SkinnedMeshRenderer` and its original
   materials.
2. Select a scene object and run **Tools > Jadren > Animation > Bake Selected
   Animator**. Select a prefab asset instead and use **Bake Selected Prefab**
   when future instances must receive the bake.
3. The baker creates Jadren rig, clip and controller assets below
   `Assets/JadrenAnimationGenerated/` and adds `JadrenAnimationAuthoring`,
   `JadrenAnimationPlayer` and the required pose applier on the Animator root.
4. Run **Validate Selected Bake** and proceed only after `Jadren parity PASS`.
5. Add a `MainCamera`, light and ground, press Play, and confirm the character
   remains visible and `JadrenAnimationPlayer.IsReady` is true.

Leave the Unity Animator enabled for this first correctness check. A crowd host
may turn it off only after `JadrenAnimationPlayer.CanDriveJadren` is true and
Jadren becomes the sole writer of that bone hierarchy. Animator and Jadren must
not apply poses to the same character in the same frame; that produces jitter
and invalid performance results. If the baked route cannot initialize, retain
the Animator fallback and correct the source prefab or controller.

For a crowd experiment, create one empty host object and start with a small
visible count before moving to 250, 500 and 1,000 agents. The procedural GPU
route (`JadrenAnimationGpuCrowdAnimator`) requires a baked source prefab,
supported compute/crowd shaders and a camera. It is an opt-in experimental
route, not a general Animator, IK or production skinning replacement.

## Automated Unity verification

The package ships separate EditMode and PlayMode test assemblies. In a local
development project, add `com.jadren.animation` to the project manifest's
`testables` array so Unity includes the package tests. Run the repository
runner from the Jadren workspace and keep separate result files:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\scripts\check-unity-animation-editmode.ps1 `
  -ProjectRoot <UnityProject> -TestPlatform editmode -RunUnity -RequirePass
pwsh -NoProfile -ExecutionPolicy Bypass -File .\scripts\check-unity-animation-editmode.ps1 `
  -ProjectRoot <UnityProject> -TestPlatform playmode `
  -TestFilter Jadren.Animation.PlayMode.Tests -RunUnity -RequirePass
```

The PlayMode fixture verifies the fixed-timestep Rigidbody/pose ownership
boundary only. A passing test run does not prove display refresh, GPU
throughput, physical-device parity or a public performance result. The runner
accepts only `editmode` or `playmode` and records the selected platform in its
contract report, preventing an EditMode report from being mistaken for a
runtime test.

## Rigidbody and fixed-timestep integration

For a physics-owned character, add `JadrenAnimationFixedUpdateBridge` together
with a `Rigidbody` and a configured `JadrenAnimationPlayer`. The bridge reads
only planar Rigidbody velocity in `FixedUpdate` and forwards it as the Jadren
locomotion speed; it does not move the Rigidbody or write the gameplay root.
The player samples and applies the pose in `LateUpdate`, keeping physics and
render presentation on their respective Unity loops. Gameplay actions can be
queued with `TryEnqueueState(name, durationSeconds)` and cleared with
`TryEnqueueClearState()`; the bounded queue is consumed on the next fixed tick.

## Continuous locomotion blend

The baked one-axis speed thresholds are evaluated as a continuous blend. When
the current speed is between two locomotion samples, Jadren interpolates their
position, rotation and scale pose (including root-motion delta). Action clips
such as jump or attack are excluded from this speed blend and remain selected
through the named override API. This is a managed correctness path; no general
GPU performance claim is attached to it yet.
When a bounded numeric transition is baked, its `fadeSeconds` value controls
the action cross-fade. Unity's normalized transition durations are marked by
`usesNormalizedDuration` and converted against the source clip duration and
playback speed for both base and overlay routes; older assets retain the
fixed-seconds interpretation. If no
matching route exists, the runtime uses a 0.15-second fallback.
State transitions may also carry a bounded normalized `exitTime`; the runtime
waits for that clip position before consuming the transition conditions.
Authored self-transitions carry Unity's `canTransitionToSelf` gate; disabled
self routes are ignored by the Jadren runtime. During an active cross-fade,
the bounded `interruptionSource` and `orderedInterruption` metadata select
permitted source/destination transitions. If none is permitted, the active
destination is held until the current fade completes; complete nested-machine
parity remains outside this 0.2 path. Direct overlay layers use the same bounded
route. Synced layers mirror the source layer's bounded transition/fade state;
when `syncedAffectsTiming` is enabled, their clock is copied from the source
without an extra per-frame advance. Full synced-state graph and IK interruption
semantics remain outside the preview.
Bool/trigger graph semantics are bounded; additional layer composition and
generic Avatar Mask entries are supported, while complete layer graph and IK
semantics remain outside this 0.2 path.

## Bounded transition parameters

Gameplay code can drive authored transition conditions without a new
animation player: use `SetBool`, `SetFloat`, `SetInt` or `SetTrigger` on
`JadrenAnimationPlayer`. Multiple conditions on one transition are combined
with logical AND. A trigger is consumed when its transition is selected; float
conditions support strict and inclusive greater/less/equal comparisons. This
bounded API does not import a complete Animator graph or synced-state graph.
Int conditions support integral Equal/NotEqual/Greater/Less comparisons;
authored non-integral thresholds are rejected rather than coerced.
Conditioned AnyState transitions are supported as a bounded wildcard action
route. Entry/Exit and layer-specific semantics are bounded to the rules below.
Transitions targeting a sub-state machine may resolve to that machine's authored
default direct clip. This is an explicit bounded import rule, not full Entry/Exit
or machine-to-machine graph parity.
Conditioned Entry transitions are also imported as one-shot initialization
transitions for the relevant layer. Exit transitions and nested machine routing
are bounded to the layer's authored default direct clip; nested machine Exit
and machine-to-machine transitions are imported when their source/destination
resolve through bounded unconditional default/Entry endpoints across nested
machines. An Exit from a non-default nested state is flattened to its parent
machine's direct/default endpoint only when that parent has exactly one
outgoing machine route. Conditions on that single parent route are appended to
the child Exit with logical AND; ambiguous or unresolved routes are skipped
rather than mapped to the layer default. Authored state-machine
transitions with no conditions or exit-time
gate are retained as an explicit unconditional route; AnyState still requires
conditions unless its destination resolves to a bounded direct/default clip
endpoint, in which case it is retained as an unconditional wildcard route.
Malformed or unsupported destinations remain rejected. Conditional deep graph
routing remains outside the contract.
The baker also records Animator layers. Additional layers can compose their
authored default direct clip over the base pose using Override or Additive mode;
generic transform Avatar Masks are resolved using the most-specific matching
path: active ancestors authorize descendants, while an explicit inactive child
overrides that authorization (and an active child can re-enable it). Layer
condition transitions are evaluated against the shared parameter bridge; synced
layers carry a bounded source-layer/timing contract, and humanoid body-part mask
entries are mapped through the source avatar into rig bones. A synced layer also
mirrors the bounded source transition state and avoids double-stepping the source
clock when timing is shared. Runtime gameplay can
adjust a baked layer with `JadrenAnimationPlayer.SetLayerWeight(index, weight)`;
the finite value is clamped to `[0, 1]` and can be read with `GetLayerWeight`.
Full synced-state graph, IK mask and complete Animator graph parity remain outside
this preview.
The bounded runtime supports a chain of synced layers, including a forward
authored reference, by resolving the source before per-frame evaluation. It
propagates state and (when timing sync is enabled) its clock without
double-stepping; a malformed cycle falls back to an independent layer. This is
not a claim of complete Unity synced-state graph parity.

Static `AnimatorState.speed` and `speedParameter` metadata are preserved for
supported direct clip states, and baked Animator parameter defaults initialize
the player without overwriting values assigned by a host before the first step.
Static `AnimatorState.cycleOffset` is also imported as a normalized phase for
looping states, then composed with a transition destination offset on entry.
Dynamic `cycleOffsetParameter` values are imported as a bounded finite-float
parameter and added to the authored phase on entry. Missing or non-finite values
use the neutral `0` fallback; ambiguous shared-clip metadata is rejected. This
remains a bounded slice rather than full Mecanim graph parity.
Dynamic `AnimatorState.timeParameter` values are also imported for direct-clip
states. The finite float is interpreted as normalized clip time: looping clips
wrap and non-looping clips clamp. A missing or non-finite parameter keeps the
ordinary advancing clock, and conflicting metadata on a shared clip is rejected.
When either side of a continuous blend uses a time parameter, the player uses the
regular state path instead of inventing a blended clock. This is a bounded clock
bridge, not complete Mecanim time-parameter parity.
The aggregate `JadrenAnimationBatchPoseEvaluator` exposes the same bounded
shared-float bridge through `SetFloat`/`GetFloat`. Playback-speed and cycle-phase
time-parameter bindings are cached per baked state, so a crowd host can update one parameter
set without per-agent string lookup. This does not imply complete Animator
graph or per-agent parameter parity.
Authored Unity `solo` transition sets are also respected for bounded Entry,
state, AnyState and machine routes: when an enabled solo route exists in a
group, non-solo routes in that group are not baked. Muted solo routes do not
activate the set. This is deterministic route selection, not complete Unity
editor-preview or Animator graph parity.

## Bounded two-bone IK

For explicit arm or leg targets, add `JadrenAnimationTwoBoneIk` below the
`JadrenAnimationPlayer` and assign its root, mid, tip and target transforms.
An optional hint selects the bend plane and `Weight` blends the result. The
player caches these constraints and applies an allocation-free analytic solve
after the baked pose on the Unity main thread. Assign an optional generic
`AvatarMask` and `MaskRoot` when the chain must be explicitly authorized; Jadren
requires active transform paths for root, mid and tip before applying the
constraint. Active ancestor paths authorize descendants, while a more specific
inactive path overrides that authorization. This is a reachable two-bone
post-pose capability with bounded generic mask gating, not full Mecanim IK,
humanoid retargeting or body-part IK mask parity.
For multiple constraints, set `ExecutionOrder` explicitly on each
`JadrenAnimationTwoBoneIk`; lower values run first. Equal orders use the stable
transform hierarchy path as a deterministic tie-breaker. This is a bounded
composition order, not a full Unity constraint-graph import.
The IK component also exposes an opt-in `MatchTargetRotation` flag with a
bounded `TargetRotationWeight`. When enabled, the tip orientation is blended
toward the target after the positional solve; the default is off, so existing
scenes keep their previous behavior. This is not full Animation Rigging or
Mecanim constraint parity.
The optional `HintWeight` blends the pole-vector hint against the chain's
authored bend plane from `0` (automatic bend plane) to `1` (full hint). Its
default is `1`, preserving the existing solver behavior and keeping the blend
allocation-free.

For humanoid assets, the same component can optionally resolve a standard
goal without manually wiring the three deform bones. Set `HumanoidGoal` to
`LeftFoot`, `RightFoot`, `LeftHand` or `RightHand` and assign `HumanoidAnimator`
(the parent `Animator` is used when it is empty). Jadren maps the goal to the
corresponding Unity `HumanBodyBones` chain once and then uses the same bounded
post-pose solver. A missing or non-humanoid avatar is rejected safely. This is
convenience mapping for four two-bone goals, not full Mecanim IK, retargeting or
constraint-graph parity. The pole-vector can use the same discovery path: set
`HumanoidHint` to `Auto`, `LeftElbow`, `RightElbow`, `LeftKnee` or `RightKnee`.
`Auto` selects the matching elbow/knee from `HumanoidGoal`; an explicit value
resolves the corresponding lower-arm/leg bone once during setup. `None` leaves
the explicit `Hint` transform in control. Missing/ambiguous/non-humanoid
resolution fails closed; this bounded mapping does not claim full Mecanim hint
or constraint-graph parity.
When a pure humanoid `AvatarMask` is assigned to the IK component, Jadren also
requires the matching IK-only pass flag (`LeftFootIK`, `RightFootIK`,
`LeftHandIK` or `RightHandIK`) for the selected goal. Mixed masks with explicit
transform paths keep path precedence, so this bounded gate does not claim full
Mecanim IK graph or retargeting parity.

## GPU scope

The current GPU work validates compute dispatch, completion, readback, reusable
buffers, material bindings, and procedural crowd rendering in bounded samples.
It is not yet a general replacement for every `SkinnedMeshRenderer`, Animator
Controller feature, inverse-kinematics system, or production rendering pipeline.

## Robot cadence benchmark

The package includes a bounded editor runner,
`JadrenAnimationRobotCadenceBenchmarkBatchRunner.Run`, for a baked robot
prefab. It explicitly steps Idle, Walk, Run and Jump phases at 60 and 120 Hz
and records requested phases, observed controller state names, applied action
overrides, pose checksum readiness and p50/p95 CPU time for
`JadrenAnimationPlayer.Step`. Each phase also records first/last pose checksums;
`pose_changes_observed` confirms that all four phases changed pose data, rather
than only changing a state label. Phase
completion and controller-state-name completion are reported separately. The
report uses `manual-player-step` cadence, so it must not be read as
display-refresh, GPU, physical-device or public speedup evidence. Validate the JSON with
`scripts/check-unity-animation-robot-cadence.ps1`; use
`-RequirePoseChanges` when a strict phase-pose change gate is required. If the authored controller
uses different state names, configure `JADREN_ROBOT_IDLE_STATE`,
`JADREN_ROBOT_WALK_STATE`, `JADREN_ROBOT_RUN_STATE` and
`JADREN_ROBOT_JUMP_STATE`; these aliases only describe the mapping and never
rename or mutate the Unity controller.

## Performance expectations

The system should reduce Animator and GameObject overhead when many compatible
characters share batched data and update policy. Actual gains depend on rig
complexity, visible character count, skinning path, render pipeline, and target
hardware. A public performance claim requires a clean, reproducible benchmark.
