using System;
using UnityEngine;

namespace Jadren.Animation
{
    /// <summary>
    /// Worker-owned immutable clip snapshot. It contains no ScriptableObject,
    /// Transform or Animator reference and may be evaluated off the Unity main
    /// thread by a future Job/Task scheduler.
    /// </summary>
    public sealed class JadrenAnimationClipSnapshot
    {
        private readonly Vector3[] translations;
        private readonly Quaternion[] rotations;
        private readonly Vector3[] scales;

        public int RigBoneCount { get; private set; }
        public int FrameCount { get; private set; }
        public float SampleRate { get; private set; }
        public float Duration { get; private set; }
        public bool Loop { get; private set; }

        private JadrenAnimationClipSnapshot(
            int boneCount,
            int frameCount,
            float sampleRate,
            float duration,
            bool loop,
            Vector3[] sourceTranslations,
            Quaternion[] sourceRotations,
            Vector3[] sourceScales)
        {
            RigBoneCount = Mathf.Max(0, boneCount);
            FrameCount = Mathf.Max(0, frameCount);
            SampleRate = Mathf.Max(1.0f, sampleRate);
            Duration = Mathf.Max(0.0f, duration);
            Loop = loop;
            translations = sourceTranslations ?? Array.Empty<Vector3>();
            rotations = sourceRotations ?? Array.Empty<Quaternion>();
            scales = sourceScales ?? Array.Empty<Vector3>();
        }

        public static JadrenAnimationClipSnapshot FromAsset(JadrenClipAsset asset)
        {
            if (asset == null)
            {
                return null;
            }

            asset.CopyBakedData(out var copiedTranslations, out var copiedRotations, out var copiedScales);
            return new JadrenAnimationClipSnapshot(
                asset.RigBoneCount,
                asset.FrameCount,
                asset.SampleRate,
                asset.Duration,
                asset.Loop,
                copiedTranslations,
                copiedRotations,
                copiedScales);
        }

        public bool SampleBone(
            int boneIndex,
            float time,
            out Vector3 position,
            out Quaternion rotation,
            out Vector3 scale)
        {
            position = Vector3.zero;
            rotation = Quaternion.identity;
            scale = Vector3.one;
            if (RigBoneCount <= 0 || FrameCount <= 0 || boneIndex < 0 || boneIndex >= RigBoneCount)
            {
                return false;
            }

            var sampleTime = Duration <= 0.0f ? 0.0f : time;
            if (Loop && Duration > 0.0f)
            {
                sampleTime %= Duration;
                if (sampleTime < 0.0f)
                {
                    sampleTime += Duration;
                }
            }
            else
            {
                sampleTime = Mathf.Clamp(sampleTime, 0.0f, Duration);
            }

            var frame = sampleTime * SampleRate;
            var first = Mathf.Clamp(Mathf.FloorToInt(frame), 0, FrameCount - 1);
            var second = Mathf.Min(first + 1, FrameCount - 1);
            var weight = Mathf.Clamp01(frame - first);
            var firstIndex = first * RigBoneCount + boneIndex;
            var secondIndex = second * RigBoneCount + boneIndex;
            if (secondIndex >= translations.Length || secondIndex >= rotations.Length || secondIndex >= scales.Length)
            {
                return false;
            }

            position = Vector3.LerpUnclamped(translations[firstIndex], translations[secondIndex], weight);
            rotation = JadrenQuaternionMath.SlerpUnclamped(rotations[firstIndex], rotations[secondIndex], weight);
            scale = Vector3.LerpUnclamped(scales[firstIndex], scales[secondIndex], weight);
            return true;
        }
    }

    /// <summary>
    /// Pure pose evaluator. The constructor is called on the main thread to
    /// snapshot assets; Evaluate only touches worker-owned arrays and the
    /// caller-owned output buffer. Unity object access is intentionally absent.
    /// </summary>
    public sealed class JadrenAnimationPoseWorker
    {
        private readonly int boneCount;
        private readonly JadrenAnimationClipSnapshot[] clips;
        private readonly JadrenAnimationPoseNativeBridge nativeSlerp;

        public JadrenAnimationPoseWorker(
            JadrenRigAsset rig,
            JadrenControllerAsset controller,
            bool preferNativeSlerp = false)
        {
            boneCount = rig == null ? 0 : rig.BoneCount;
            var stateCount = controller == null ? 0 : controller.StateCount;
            clips = new JadrenAnimationClipSnapshot[stateCount];
            for (var i = 0; i < stateCount; i++)
            {
                clips[i] = JadrenAnimationClipSnapshot.FromAsset(controller.GetState(i).clip);
            }
            nativeSlerp = preferNativeSlerp
                ? new JadrenAnimationPoseNativeBridge(boneCount)
                : null;
        }

        public bool UsesNativeSlerp
        {
            get { return nativeSlerp != null && nativeSlerp.IsAvailable; }
        }

        public int Evaluate(
            int currentState,
            float currentTime,
            float currentPreviousTime,
            int previousState,
            float previousTime,
            float fadeWeight,
            JadrenAnimationLod lod,
            JadrenPoseBuffer output)
        {
            if (output == null)
            {
                throw new ArgumentNullException(nameof(output));
            }

            output.EnsureCapacity(boneCount);
            output.SampledBoneCount = 0;
            output.RootMotionDelta = Vector3.zero;
            output.Checksum = 0UL;
            if (boneCount == 0 || lod == JadrenAnimationLod.Hidden)
            {
                return 0;
            }

            var currentClip = GetClip(currentState);
            if (currentClip == null)
            {
                return 0;
            }

            var previousClip = GetClip(previousState);
            ValidateFadeWeight(fadeWeight);
            // Keep the value unclamped. The public pose contract deliberately
            // supports extrapolation, so both negative and >1 weights must
            // reach the same Slerp/Lerp math as the native bridge.
            var blend = fadeWeight;
            var hasPrevious = previousClip != null;
            var useNativeSlerp = hasPrevious && nativeSlerp != null && nativeSlerp.IsAvailable;
            if (useNativeSlerp)
            {
                nativeSlerp.Begin();
            }
            for (var boneIndex = 0; boneIndex < boneCount; boneIndex++)
            {
                if (lod == JadrenAnimationLod.Reduced && (boneIndex & 1) != 0)
                {
                    continue;
                }
                if (!currentClip.SampleBone(boneIndex, currentTime, out var currentPosition, out var currentRotation, out var currentScale))
                {
                    continue;
                }

                var position = currentPosition;
                var rotation = currentRotation;
                var scale = currentScale;
                var previousPosition = Vector3.zero;
                var previousRotation = Quaternion.identity;
                var previousScale = Vector3.one;
                var hasSampledPrevious = hasPrevious
                    && previousClip.SampleBone(
                        boneIndex,
                        previousTime,
                        out previousPosition,
                        out previousRotation,
                        out previousScale);
                if (hasSampledPrevious)
                {
                    if (blend == 0.0f)
                    {
                        position = previousPosition;
                        rotation = previousRotation;
                        scale = previousScale;
                    }
                    else
                    {
                        position = Vector3.LerpUnclamped(previousPosition, currentPosition, blend);
                        if (useNativeSlerp)
                        {
                            nativeSlerp.Set(boneIndex, previousRotation, currentRotation);
                        }
                        else
                        {
                            rotation = JadrenQuaternionMath.SlerpUnclamped(previousRotation, currentRotation, blend);
                        }
                        scale = Vector3.LerpUnclamped(previousScale, currentScale, blend);
                    }
                }
                else if (useNativeSlerp)
                {
                    nativeSlerp.Set(boneIndex, currentRotation, currentRotation);
                }

                output.Positions[boneIndex] = position;
                output.Rotations[boneIndex] = rotation;
                output.Scales[boneIndex] = scale;
                output.SampledBoneCount++;
            }

            var nativeApplied = !useNativeSlerp
                || nativeSlerp.TryApply(output.Rotations, boneCount, blend, lod);
            if (useNativeSlerp && !nativeApplied)
            {
                // The bridge disables itself when a plugin/export disappears
                // between capability probing and the call. Recompute only the
                // affected rotations so a late native failure cannot publish
                // default quaternions into the main-thread applier.
                for (var boneIndex = 0; boneIndex < boneCount; boneIndex++)
                {
                    if (lod == JadrenAnimationLod.Reduced && (boneIndex & 1) != 0)
                    {
                        continue;
                    }
                    if (previousClip.SampleBone(boneIndex, previousTime, out _, out var previousRotation, out _)
                        && currentClip.SampleBone(boneIndex, currentTime, out _, out var currentRotation, out _))
                    {
                        output.Rotations[boneIndex] = blend == 0.0f
                            ? previousRotation
                            : JadrenQuaternionMath.SlerpUnclamped(previousRotation, currentRotation, blend);
                    }
                }
            }

            if (currentClip.SampleBone(0, currentTime, out var rootNow, out _, out _)
                && currentClip.SampleBone(0, currentPreviousTime, out var rootBefore, out _, out _))
            {
                output.RootMotionDelta = rootNow - rootBefore;
            }
            output.Checksum = JadrenPoseKernel.ComputeChecksum(output, boneCount, lod);
            return output.SampledBoneCount;
        }

        /// <summary>
        /// Fills caller-owned bone-indexed rotation inputs for the optional GPU
        /// coordinator. It touches only worker snapshots and value arrays;
        /// dispatch and Unity API calls remain on the host main thread.
        /// </summary>
        public int PrepareGpuRotationInputs(
            int currentState,
            float currentTime,
            int previousState,
            float previousTime,
            float fadeWeight,
            JadrenAnimationLod lod,
            Quaternion[] previous,
            Quaternion[] current,
            float[] weights)
        {
            if (previous == null) throw new ArgumentNullException(nameof(previous));
            if (current == null) throw new ArgumentNullException(nameof(current));
            if (weights == null) throw new ArgumentNullException(nameof(weights));
            if (previous.Length < boneCount || current.Length < boneCount || weights.Length < boneCount)
            {
                throw new ArgumentException("GPU rotation input arrays are shorter than the worker rig.");
            }
            ValidateFadeWeight(fadeWeight);
            Array.Clear(weights, 0, boneCount);
            if (boneCount == 0 || lod == JadrenAnimationLod.Hidden)
            {
                return 0;
            }

            var currentClip = GetClip(currentState);
            if (currentClip == null)
            {
                return 0;
            }

            var previousClip = GetClip(previousState);
            var sampledCount = 0;
            for (var boneIndex = 0; boneIndex < boneCount; boneIndex++)
            {
                if (lod == JadrenAnimationLod.Reduced && (boneIndex & 1) != 0)
                {
                    continue;
                }
                if (!currentClip.SampleBone(
                        boneIndex,
                        currentTime,
                        out _,
                        out var currentRotation,
                        out _))
                {
                    continue;
                }

                var previousRotation = currentRotation;
                if (previousClip != null
                    && previousClip.SampleBone(
                        boneIndex,
                        previousTime,
                        out _,
                        out var sampledPreviousRotation,
                        out _))
                {
                    previousRotation = sampledPreviousRotation;
                }

                previous[boneIndex] = previousRotation;
                current[boneIndex] = currentRotation;
                weights[boneIndex] = fadeWeight;
                sampledCount++;
            }
            return sampledCount;
        }

        private JadrenAnimationClipSnapshot GetClip(int stateIndex)
        {
            return stateIndex >= 0 && stateIndex < clips.Length ? clips[stateIndex] : null;
        }

        private static void ValidateFadeWeight(float fadeWeight)
        {
            if (float.IsNaN(fadeWeight) || float.IsInfinity(fadeWeight))
            {
                throw new ArgumentOutOfRangeException(nameof(fadeWeight));
            }
        }
    }
}
