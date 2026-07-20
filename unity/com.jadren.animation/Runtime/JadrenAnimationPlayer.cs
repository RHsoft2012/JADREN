using UnityEngine;

namespace Jadren.Animation
{
    [DisallowMultipleComponent]
    [RequireComponent(typeof(JadrenAnimationAuthoring))]
    [RequireComponent(typeof(JadrenAnimationPoseApplier))]
    public sealed class JadrenAnimationPlayer : MonoBehaviour
    {
        [SerializeField] private JadrenAnimationAuthoring authoring;
        [SerializeField] private JadrenAnimationPoseApplier applier;
        [SerializeField] private bool playOnEnable = true;
        [SerializeField] private bool preferNativeSlerp;
        [SerializeField] private float speedInput;
        [SerializeField] private ComputeShader gpuRotationShader;

        private JadrenAnimationPoseWorker poseWorker;
        private JadrenAnimationGpuPoseCoordinator gpuPoseCoordinator;
        private Quaternion[] gpuPreviousRotations;
        private Quaternion[] gpuCurrentRotations;
        private float[] gpuWeights;
        private JadrenAnimationState state = JadrenAnimationState.Default;
        private readonly JadrenPoseBuffer poseBuffer = new JadrenPoseBuffer();
        private int activeState = -1;
        private float stateTime;
        private float previousStateTime;
        private float transitionWeight = 1.0f;
        private int previousState = -1;
        private int frameCounter;

        public JadrenAnimationState State { get { return state; } }
        public bool IsReady { get { return authoring != null && authoring.IsConfigured && activeState >= 0; } }
        /// <summary>
        /// Indicates that the baked authoring, pose worker and main-thread
        /// applier are all available before a host disables Unity Animator.
        /// This is a capability check, not a claim that a frame was applied.
        /// </summary>
        public bool CanDriveJadren
        {
            get
            {
                EnsureInitialized();
                return authoring != null
                    && authoring.IsConfigured
                    && poseWorker != null
                    && applier != null
                    && applier.BoundBoneCount == authoring.Rig.BoneCount;
            }
        }
        public Vector3 RootMotionDelta { get; private set; }
        public ulong PoseChecksum { get; private set; }
        public int SampledBoneCount { get; private set; }
        public bool UsesNativeSlerp { get { return poseWorker != null && poseWorker.UsesNativeSlerp; } }
        public bool HasPendingGpuPose
        {
            get { return gpuPoseCoordinator != null && gpuPoseCoordinator.HasPending; }
        }
        public string LastGpuPoseFailureReason
        {
            get { return gpuPoseCoordinator == null ? string.Empty : gpuPoseCoordinator.LastFailureReason; }
        }

        private void Awake()
        {
            EnsureInitialized();
        }

        private void OnEnable()
        {
            if (playOnEnable)
            {
                ResetPlayback();
            }
        }

        private void LateUpdate()
        {
            Step(Time.deltaTime);
        }

        private void OnDestroy()
        {
            DisposeGpuPoseCoordinator();
        }

        public void SetSpeed(float value)
        {
            speedInput = Mathf.Max(0.0f, value);
        }

        /// <summary>
        /// Enables the opt-in GPU rotation path. The normal managed Step path
        /// remains unchanged until TryQueueGpuPose is called explicitly.
        /// </summary>
        public void SetGpuRotationShader(ComputeShader shader)
        {
            DisposeGpuPoseCoordinator();
            gpuRotationShader = shader;
            if (shader != null)
            {
                TryCreateGpuPoseCoordinator();
            }
        }

        /// <summary>
        /// Captures the pose just evaluated by Step and queues only its
        /// quaternion blend for the GPU. Call PollGpuPose or CompleteGpuPose
        /// on the Unity main thread to apply the completed result.
        /// </summary>
        public bool TryQueueGpuPose()
        {
            EnsureInitialized();
            if (!IsReady || poseWorker == null || gpuRotationShader == null)
            {
                return false;
            }
            if (!TryCreateGpuPoseCoordinator() || gpuPoseCoordinator == null || !gpuPoseCoordinator.IsAvailable)
            {
                return false;
            }

            var boneCount = authoring.Rig.BoneCount;
            EnsureGpuRotationArrays(boneCount);
            var sampled = poseWorker.PrepareGpuRotationInputs(
                activeState,
                stateTime,
                previousState,
                previousStateTime,
                state.fadeWeight,
                authoring.DefaultLod,
                gpuPreviousRotations,
                gpuCurrentRotations,
                gpuWeights);
            var expected = ExpectedSampleCount(boneCount, authoring.DefaultLod);
            if (sampled != expected)
            {
                return false;
            }

            return gpuPoseCoordinator.TryQueue(
                poseBuffer,
                gpuPreviousRotations,
                gpuCurrentRotations,
                gpuWeights,
                boneCount,
                authoring.DefaultLod,
                out _);
        }

        /// <summary>Polls GPU readback without blocking and applies it when ready.</summary>
        public JadrenAnimationGpuPoseApplyStatus PollGpuPose()
        {
            if (gpuPoseCoordinator == null)
            {
                return JadrenAnimationGpuPoseApplyStatus.NoPending;
            }

            var status = gpuPoseCoordinator.PollAndApply(applier);
            SyncGpuPoseMetadata(status);
            return status;
        }

        /// <summary>Completes and applies a queued GPU pose, blocking if needed.</summary>
        public JadrenAnimationGpuPoseApplyStatus CompleteGpuPose()
        {
            if (gpuPoseCoordinator == null)
            {
                return JadrenAnimationGpuPoseApplyStatus.NoPending;
            }

            var status = gpuPoseCoordinator.CompleteAndApply(applier);
            SyncGpuPoseMetadata(status);
            return status;
        }

        /// <summary>Abandons a pending readback without applying it.</summary>
        public void CancelGpuPose()
        {
            if (gpuPoseCoordinator != null)
            {
                gpuPoseCoordinator.CancelPending();
            }
        }

        public void ResetPlayback()
        {
            state = JadrenAnimationState.Default;
            activeState = -1;
            previousStateTime = 0.0f;
            previousState = -1;
            stateTime = 0.0f;
            transitionWeight = 1.0f;
            frameCounter = 0;
            RootMotionDelta = Vector3.zero;
            PoseChecksum = 0UL;
            SampledBoneCount = 0;
            RebuildBoneBindings();
            RebuildPoseWorker();
        }

        public void Step(float deltaTime)
        {
            // Unity calls Awake for normal scene instances, but editor tools
            // and runtime spawners may add this component dynamically. Resolve
            // the same worker/applier boundary lazily so those instances do
            // not silently fall back to an uninitialized no-op.
            EnsureInitialized();
            if (authoring == null || !authoring.IsConfigured || authoring.DefaultLod == JadrenAnimationLod.Hidden)
            {
                return;
            }

            if (applier == null || applier.BoundBoneCount != authoring.Rig.BoneCount || poseWorker == null)
            {
                RebuildBoneBindings();
                RebuildPoseWorker();
            }

            var nextState = authoring.Controller.ResolveState(activeState < 0 ? 0 : activeState, speedInput);
            if (nextState < 0)
            {
                return;
            }
            if (nextState != activeState)
            {
                previousState = activeState;
                previousStateTime = stateTime;
                activeState = nextState;
                stateTime = 0.0f;
                transitionWeight = previousState < 0 ? 1.0f : 0.0f;
            }

            var definition = authoring.Controller.GetState(activeState);
            if (definition.clip == null)
            {
                return;
            }

            var playbackSpeed = Mathf.Approximately(definition.playbackSpeed, 0.0f)
                ? 1.0f
                : definition.playbackSpeed;
            var currentPreviousTime = stateTime;
            var step = Mathf.Max(0.0f, deltaTime);
            if (previousState >= 0 && transitionWeight < 1.0f)
            {
                var previousDefinition = authoring.Controller.GetState(previousState);
                var previousPlaybackSpeed = Mathf.Approximately(previousDefinition.playbackSpeed, 0.0f)
                    ? 1.0f
                    : previousDefinition.playbackSpeed;
                previousStateTime += step * previousPlaybackSpeed;
            }
            stateTime += step * playbackSpeed;
            if (transitionWeight < 1.0f)
            {
                transitionWeight = Mathf.MoveTowards(transitionWeight, 1.0f, step / 0.15f);
            }

            state.stateIndex = activeState;
            state.time = stateTime;
            state.speed = speedInput;
            state.fadeWeight = transitionWeight;
            state.lod = authoring.DefaultLod;
            var previousClip = previousState >= 0
                ? authoring.Controller.GetState(previousState).clip
                : null;
            if (poseWorker != null)
            {
                poseWorker.Evaluate(
                    activeState,
                    stateTime,
                    currentPreviousTime,
                    previousState,
                    previousStateTime,
                    transitionWeight,
                    authoring.DefaultLod,
                    poseBuffer);
            }
            else
            {
                // Keep the scalar asset path as a defensive fallback if a
                // caller changes authoring data while the component is live.
                JadrenPoseKernel.Sample(
                    authoring.Rig,
                    definition.clip,
                    stateTime,
                    currentPreviousTime,
                    previousClip,
                    previousStateTime,
                    transitionWeight,
                    authoring.DefaultLod,
                    poseBuffer);
            }
            ApplyPose();
            if (transitionWeight >= 1.0f && previousState >= 0)
            {
                previousState = -1;
                previousStateTime = 0.0f;
            }
        }

        private void ApplyPose()
        {
            if (applier != null)
            {
                applier.Apply(poseBuffer, authoring.DefaultLod);
            }
            RootMotionDelta = poseBuffer.RootMotionDelta;
            PoseChecksum = poseBuffer.Checksum;
            SampledBoneCount = poseBuffer.SampledBoneCount;
            frameCounter++;
        }

        private void RebuildBoneBindings()
        {
            if (applier == null)
            {
                return;
            }
            applier.RebuildBindings(authoring == null ? null : authoring.Rig, transform);
        }

        private void EnsureInitialized()
        {
            if (authoring == null)
            {
                authoring = GetComponent<JadrenAnimationAuthoring>();
            }
            if (applier == null)
            {
                applier = GetComponent<JadrenAnimationPoseApplier>();
            }
            if (applier == null)
            {
                // Existing scene/prefab instances may predate the
                // RequireComponent declaration. Add the sink once so those
                // instances also use the worker/applier path immediately.
                applier = gameObject.AddComponent<JadrenAnimationPoseApplier>();
            }

            var requiredBoneCount = authoring == null || authoring.Rig == null
                ? 0
                : authoring.Rig.BoneCount;
            if (applier.BoundBoneCount != requiredBoneCount)
            {
                RebuildBoneBindings();
            }
            if (poseWorker == null && authoring != null && authoring.IsConfigured)
            {
                RebuildPoseWorker();
            }
        }

        private void RebuildPoseWorker()
        {
            if (authoring == null || !authoring.IsConfigured)
            {
                poseWorker = null;
                return;
            }
            poseWorker = new JadrenAnimationPoseWorker(
                authoring.Rig,
                authoring.Controller,
                preferNativeSlerp);
        }

        private bool TryCreateGpuPoseCoordinator()
        {
            if (gpuPoseCoordinator != null)
            {
                return true;
            }
            if (gpuRotationShader == null)
            {
                return false;
            }
            try
            {
                gpuPoseCoordinator = new JadrenAnimationGpuPoseCoordinator(gpuRotationShader);
                return true;
            }
            catch (System.Exception error)
            {
                gpuPoseCoordinator = null;
                Debug.LogWarning("Jadren GPU pose disabled: " + error.Message, this);
                return false;
            }
        }

        private void DisposeGpuPoseCoordinator()
        {
            if (gpuPoseCoordinator != null)
            {
                gpuPoseCoordinator.Dispose();
                gpuPoseCoordinator = null;
            }
            gpuPreviousRotations = null;
            gpuCurrentRotations = null;
            gpuWeights = null;
        }

        private void EnsureGpuRotationArrays(int boneCount)
        {
            if (gpuPreviousRotations == null || gpuPreviousRotations.Length != boneCount)
            {
                gpuPreviousRotations = new Quaternion[boneCount];
                gpuCurrentRotations = new Quaternion[boneCount];
                gpuWeights = new float[boneCount];
            }
        }

        private void SyncGpuPoseMetadata(JadrenAnimationGpuPoseApplyStatus status)
        {
            if (status != JadrenAnimationGpuPoseApplyStatus.Applied
                || gpuPoseCoordinator == null
                || gpuPoseCoordinator.LastAppliedPose == null)
            {
                return;
            }
            var applied = gpuPoseCoordinator.LastAppliedPose;
            RootMotionDelta = applied.RootMotionDelta;
            PoseChecksum = applied.Checksum;
            SampledBoneCount = applied.SampledBoneCount;
        }

        private static int ExpectedSampleCount(int boneCount, JadrenAnimationLod lod)
        {
            return lod == JadrenAnimationLod.Hidden
                ? 0
                : lod == JadrenAnimationLod.Reduced
                    ? (boneCount + 1) / 2
                    : boneCount;
        }
    }
}
