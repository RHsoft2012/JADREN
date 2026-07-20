using System;
using System.Runtime.CompilerServices;
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
        private static readonly ConditionalWeakTable<JadrenClipAsset, JadrenAnimationClipSnapshot> Cache
            = new ConditionalWeakTable<JadrenClipAsset, JadrenAnimationClipSnapshot>();

        internal struct SampleCursor
        {
            public int First;
            public int Second;
            public float Weight;
        }

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

            // A crowd commonly shares one controller/clip asset. Keep the
            // snapshot immutable and reuse it across all workers instead of
            // copying every TRS frame once per instantiated character.
            return Cache.GetValue(asset, CreateFromAsset);
        }

        private static JadrenAnimationClipSnapshot CreateFromAsset(JadrenClipAsset asset)
        {
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

            var cursor = PrepareSample(time);
            return SampleBone(cursor, boneIndex, out position, out rotation, out scale);
        }

        internal SampleCursor PrepareSample(float time)
        {
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
            return new SampleCursor
            {
                First = first,
                Second = second,
                Weight = Mathf.Clamp01(frame - first)
            };
        }

        internal bool SampleBone(
            SampleCursor cursor,
            int boneIndex,
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

            var first = cursor.First;
            var second = cursor.Second;
            var weight = cursor.Weight;
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

    /// <summary>Caller-owned input for one agent in an aggregated pose pass.</summary>
    public struct JadrenAnimationPoseBatchRequest
    {
        public int CurrentState;
        public float CurrentTime;
        public float CurrentPreviousTime;
        public int PreviousState;
        public float PreviousTime;
        public float FadeWeight;
        public JadrenAnimationLod Lod;
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
        private readonly JadrenAnimationPoseNativeBridge nativePoseBridge;
        private readonly bool preferNativeSlerp;
        private readonly bool preferNativePoseTiles;
        private JadrenAnimationPoseCrowdNativeBridge nativeCrowdPoseBridge;
        private JadrenAnimationLod[] batchLods = Array.Empty<JadrenAnimationLod>();
        private bool[] batchUsesAggregate = Array.Empty<bool>();

        public JadrenAnimationPoseWorker(
            JadrenRigAsset rig,
            JadrenControllerAsset controller,
            bool preferNativeSlerp = false,
            bool preferNativePoseTiles = false)
        {
            this.preferNativeSlerp = preferNativeSlerp;
            this.preferNativePoseTiles = preferNativePoseTiles;
            boneCount = rig == null ? 0 : rig.BoneCount;
            var stateCount = controller == null ? 0 : controller.StateCount;
            clips = new JadrenAnimationClipSnapshot[stateCount];
            for (var i = 0; i < stateCount; i++)
            {
                clips[i] = JadrenAnimationClipSnapshot.FromAsset(controller.GetState(i).clip);
            }
            nativePoseBridge = preferNativeSlerp || preferNativePoseTiles
                ? new JadrenAnimationPoseNativeBridge(boneCount)
                : null;
        }

        public bool UsesNativeSlerp
        {
            get
            {
                return preferNativeSlerp
                    && nativePoseBridge != null
                    && nativePoseBridge.IsAvailable;
            }
        }

        public bool UsesNativePoseTiles
        {
            get
            {
                return preferNativePoseTiles
                    && nativePoseBridge != null
                    && nativePoseBridge.IsTileAvailable;
            }
        }

        public bool UsesNativeCrowdPoseTiles
        {
            get
            {
                return preferNativePoseTiles
                    && nativeCrowdPoseBridge != null
                    && nativeCrowdPoseBridge.IsAvailable;
            }
        }

        /// <summary>
        /// Evaluates a caller-owned crowd and crosses the weighted AoSoA8
        /// boundary once for all interior transition position/scale values.
        /// Rotation keeps the managed exact Slerp contract in this first crowd
        /// path; scalar endpoints and extrapolation remain managed.
        /// </summary>
        public int EvaluateBatch(
            JadrenAnimationPoseBatchRequest[] requests,
            JadrenPoseBuffer[] outputs,
            int count)
        {
            if (requests == null) throw new ArgumentNullException(nameof(requests));
            if (outputs == null) throw new ArgumentNullException(nameof(outputs));
            if (count < 0 || count > requests.Length || count > outputs.Length)
            {
                throw new ArgumentOutOfRangeException(nameof(count));
            }
            if (count == 0)
            {
                return 0;
            }

            EnsureBatchCapacity(count);
            var aggregateAvailable = preferNativePoseTiles
                && nativeCrowdPoseBridge != null
                && nativeCrowdPoseBridge.IsAvailable;
            if (!aggregateAvailable)
            {
                var fallbackTotal = 0;
                for (var agent = 0; agent < count; agent++)
                {
                    var request = requests[agent];
                    fallbackTotal += Evaluate(
                        request.CurrentState,
                        request.CurrentTime,
                        request.CurrentPreviousTime,
                        request.PreviousState,
                        request.PreviousTime,
                        request.FadeWeight,
                        request.Lod,
                        outputs[agent]);
                }
                return fallbackTotal;
            }

            nativeCrowdPoseBridge.Begin(count);
            var sampledTotal = 0;
            for (var agent = 0; agent < count; agent++)
            {
                var request = requests[agent];
                ValidateFadeWeight(request.FadeWeight);
                batchLods[agent] = request.Lod;
                batchUsesAggregate[agent] = false;
                var output = outputs[agent];
                if (output == null)
                {
                    throw new ArgumentNullException(nameof(outputs));
                }
                output.EnsureCapacity(boneCount);
                output.SampledBoneCount = 0;
                output.RootMotionDelta = Vector3.zero;
                output.Checksum = 0UL;
                if (boneCount == 0 || request.Lod == JadrenAnimationLod.Hidden)
                {
                    continue;
                }

                var currentClip = GetClip(request.CurrentState);
                if (currentClip == null)
                {
                    continue;
                }
                var previousClip = GetClip(request.PreviousState);
                var currentCursor = currentClip.PrepareSample(request.CurrentTime);
                var previousCursor = previousClip == null
                    ? default(JadrenAnimationClipSnapshot.SampleCursor)
                    : previousClip.PrepareSample(request.PreviousTime);
                var hasPrevious = previousClip != null;
                var blend = request.FadeWeight;
                var useAggregate = hasPrevious && blend > 0.0f && blend < 1.0f;
                batchUsesAggregate[agent] = useAggregate;

                for (var bone = 0; bone < boneCount; bone++)
                {
                    if (request.Lod == JadrenAnimationLod.Reduced && (bone & 1) != 0)
                    {
                        continue;
                    }
                    if (!currentClip.SampleBone(
                            currentCursor,
                            bone,
                            out var currentPosition,
                            out var currentRotation,
                            out var currentScale))
                    {
                        continue;
                    }

                    var position = currentPosition;
                    var rotation = currentRotation;
                    var scale = currentScale;
                    if (hasPrevious
                        && previousClip.SampleBone(
                            previousCursor,
                            bone,
                            out var previousPosition,
                            out var previousRotation,
                            out var previousScale))
                    {
                        if (blend == 0.0f)
                        {
                            position = previousPosition;
                            rotation = previousRotation;
                            scale = previousScale;
                        }
                        else
                        {
                            if (useAggregate)
                            {
                                nativeCrowdPoseBridge.SetLinear(
                                    agent,
                                    bone,
                                    blend,
                                    previousPosition,
                                    previousRotation,
                                    previousScale,
                                    currentPosition,
                                    currentRotation,
                                    currentScale);
                            }
                            else
                            {
                                position = Vector3.LerpUnclamped(previousPosition, currentPosition, blend);
                                scale = Vector3.LerpUnclamped(previousScale, currentScale, blend);
                            }
                            rotation = JadrenQuaternionMath.SlerpUnclamped(
                                previousRotation,
                                currentRotation,
                                blend);
                        }
                    }

                    output.Positions[bone] = position;
                    output.Rotations[bone] = rotation;
                    output.Scales[bone] = scale;
                    output.SampledBoneCount++;
                }

                var rootBeforeCursor = currentClip.PrepareSample(request.CurrentPreviousTime);
                if (currentClip.SampleBone(currentCursor, 0, out var rootNow, out _, out _)
                    && currentClip.SampleBone(rootBeforeCursor, 0, out var rootBefore, out _, out _))
                {
                    output.RootMotionDelta = rootNow - rootBefore;
                }
                sampledTotal += output.SampledBoneCount;
            }

            var nativeApplied = nativeCrowdPoseBridge.TryApplyLinear(outputs, batchLods, count);
            if (!nativeApplied)
            {
                RecomputeBatchLinearFallback(requests, outputs, count);
            }
            for (var agent = 0; agent < count; agent++)
            {
                outputs[agent].Checksum = JadrenPoseKernel.ComputeChecksum(
                    outputs[agent],
                    boneCount,
                    requests[agent].Lod);
            }
            return sampledTotal;
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
            var currentCursor = currentClip.PrepareSample(currentTime);
            var previousCursor = previousClip == null
                ? default(JadrenAnimationClipSnapshot.SampleCursor)
                : previousClip.PrepareSample(previousTime);
            ValidateFadeWeight(fadeWeight);
            // Keep the value unclamped. The public pose contract deliberately
            // supports extrapolation, so both negative and >1 weights must
            // reach the same Slerp/Lerp math as the native bridge.
            var blend = fadeWeight;
            var hasPrevious = previousClip != null;
            var useNativeSlerp = hasPrevious
                && preferNativeSlerp
                && nativePoseBridge != null
                && nativePoseBridge.IsAvailable;
            var useNativePoseTiles = hasPrevious
                && preferNativePoseTiles
                && blend > 0.0f
                && blend < 1.0f
                && nativePoseBridge != null
                && nativePoseBridge.IsTileAvailable;
            if (useNativeSlerp || useNativePoseTiles)
            {
                nativePoseBridge.Begin();
            }
            for (var boneIndex = 0; boneIndex < boneCount; boneIndex++)
            {
                if (lod == JadrenAnimationLod.Reduced && (boneIndex & 1) != 0)
                {
                    continue;
                }
                if (!currentClip.SampleBone(currentCursor, boneIndex, out var currentPosition, out var currentRotation, out var currentScale))
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
                        previousCursor,
                        boneIndex,
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
                        if (useNativePoseTiles)
                        {
                            nativePoseBridge.SetLinear(
                                boneIndex,
                                previousPosition,
                                previousRotation,
                                previousScale,
                                currentPosition,
                                currentRotation,
                                currentScale);
                        }
                        else
                        {
                            position = Vector3.LerpUnclamped(previousPosition, currentPosition, blend);
                            scale = Vector3.LerpUnclamped(previousScale, currentScale, blend);
                        }
                        if (useNativeSlerp)
                        {
                            nativePoseBridge.Set(boneIndex, previousRotation, currentRotation);
                        }
                        else
                        {
                            rotation = JadrenQuaternionMath.SlerpUnclamped(previousRotation, currentRotation, blend);
                        }
                    }
                }
                else if (useNativeSlerp)
                {
                    nativePoseBridge.Set(boneIndex, currentRotation, currentRotation);
                }

                output.Positions[boneIndex] = position;
                output.Rotations[boneIndex] = rotation;
                output.Scales[boneIndex] = scale;
                output.SampledBoneCount++;
            }

            var nativeTilesApplied = !useNativePoseTiles
                || nativePoseBridge.TryApplyLinear(
                    output.Positions,
                    output.Scales,
                    boneCount,
                    blend,
                    lod);
            if (useNativePoseTiles && !nativeTilesApplied)
            {
                // Preserve the exact managed pose if a validated tile export
                // disappears between capability probing and execution.
                for (var boneIndex = 0; boneIndex < boneCount; boneIndex++)
                {
                    if (lod == JadrenAnimationLod.Reduced && (boneIndex & 1) != 0)
                    {
                        continue;
                    }
                    if (previousClip.SampleBone(
                            previousCursor,
                            boneIndex,
                            out var previousPosition,
                            out _,
                            out var previousScale)
                        && currentClip.SampleBone(
                            currentCursor,
                            boneIndex,
                            out var currentPosition,
                            out _,
                            out var currentScale))
                    {
                        output.Positions[boneIndex] = Vector3.LerpUnclamped(
                            previousPosition,
                            currentPosition,
                            blend);
                        output.Scales[boneIndex] = Vector3.LerpUnclamped(
                            previousScale,
                            currentScale,
                            blend);
                    }
                }
            }

            var nativeApplied = !useNativeSlerp
                || nativePoseBridge.TryApply(output.Rotations, boneCount, blend, lod);
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
                    if (previousClip.SampleBone(previousCursor, boneIndex, out _, out var previousRotation, out _)
                        && currentClip.SampleBone(currentCursor, boneIndex, out _, out var currentRotation, out _))
                    {
                        output.Rotations[boneIndex] = blend == 0.0f
                            ? previousRotation
                            : JadrenQuaternionMath.SlerpUnclamped(previousRotation, currentRotation, blend);
                    }
                }
            }

            var rootBeforeCursor = currentClip.PrepareSample(currentPreviousTime);
            if (currentClip.SampleBone(currentCursor, 0, out var rootNow, out _, out _)
                && currentClip.SampleBone(rootBeforeCursor, 0, out var rootBefore, out _, out _))
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
            var currentCursor = currentClip.PrepareSample(currentTime);
            var previousCursor = previousClip == null
                ? default(JadrenAnimationClipSnapshot.SampleCursor)
                : previousClip.PrepareSample(previousTime);
            var sampledCount = 0;
            for (var boneIndex = 0; boneIndex < boneCount; boneIndex++)
            {
                if (lod == JadrenAnimationLod.Reduced && (boneIndex & 1) != 0)
                {
                    continue;
                }
                if (!currentClip.SampleBone(
                        currentCursor,
                        boneIndex,
                        out _,
                        out var currentRotation,
                        out _))
                {
                    continue;
                }

                var previousRotation = currentRotation;
                if (previousClip != null
                    && previousClip.SampleBone(
                        previousCursor,
                        boneIndex,
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

        private void EnsureBatchCapacity(int count)
        {
            if (batchLods.Length < count)
            {
                batchLods = new JadrenAnimationLod[count];
                batchUsesAggregate = new bool[count];
            }
            if (nativeCrowdPoseBridge == null || nativeCrowdPoseBridge.AgentCapacity < count)
            {
                nativeCrowdPoseBridge = new JadrenAnimationPoseCrowdNativeBridge(boneCount, count);
            }
        }

        private void RecomputeBatchLinearFallback(
            JadrenAnimationPoseBatchRequest[] requests,
            JadrenPoseBuffer[] outputs,
            int count)
        {
            for (var agent = 0; agent < count; agent++)
            {
                if (!batchUsesAggregate[agent])
                {
                    continue;
                }
                var request = requests[agent];
                var currentClip = GetClip(request.CurrentState);
                var previousClip = GetClip(request.PreviousState);
                if (currentClip == null || previousClip == null)
                {
                    continue;
                }
                var currentCursor = currentClip.PrepareSample(request.CurrentTime);
                var previousCursor = previousClip.PrepareSample(request.PreviousTime);
                for (var bone = 0; bone < boneCount; bone++)
                {
                    if (request.Lod == JadrenAnimationLod.Reduced && (bone & 1) != 0)
                    {
                        continue;
                    }
                    if (previousClip.SampleBone(
                            previousCursor,
                            bone,
                            out var previousPosition,
                            out _,
                            out var previousScale)
                        && currentClip.SampleBone(
                            currentCursor,
                            bone,
                            out var currentPosition,
                            out _,
                            out var currentScale))
                    {
                        outputs[agent].Positions[bone] = Vector3.LerpUnclamped(
                            previousPosition,
                            currentPosition,
                            request.FadeWeight);
                        outputs[agent].Scales[bone] = Vector3.LerpUnclamped(
                            previousScale,
                            currentScale,
                            request.FadeWeight);
                    }
                }
            }
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
