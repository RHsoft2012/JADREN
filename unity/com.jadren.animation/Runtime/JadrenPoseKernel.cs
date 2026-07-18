using System;
using UnityEngine;

namespace Jadren.Animation
{
    /// <summary>
    /// Reference scalar pose kernel. It is deliberately independent of MonoBehaviour,
    /// Animator and Unity scene state so the same contract can be ported to AVX2/NEON.
    /// </summary>
    public static class JadrenPoseKernel
    {
        private const ulong FnvOffset = 14695981039346656037UL;
        private const ulong FnvPrime = 1099511628211UL;

        public static int Sample(
            JadrenRigAsset rig,
            JadrenClipAsset currentClip,
            float currentTime,
            float currentPreviousTime,
            JadrenClipAsset previousClip,
            float previousTime,
            float fadeWeight,
            JadrenAnimationLod lod,
            JadrenPoseBuffer output)
        {
            if (output == null)
            {
                throw new ArgumentNullException(nameof(output));
            }

            var boneCount = rig == null ? 0 : rig.BoneCount;
            output.EnsureCapacity(boneCount);
            output.SampledBoneCount = 0;
            output.RootMotionDelta = Vector3.zero;
            output.Checksum = 0UL;
            if (boneCount == 0 || currentClip == null || lod == JadrenAnimationLod.Hidden)
            {
                return 0;
            }

            ValidateFadeWeight(fadeWeight);
            var blend = fadeWeight;
            var hasPrevious = previousClip != null;
            for (var boneIndex = 0; boneIndex < boneCount; boneIndex++)
            {
                if (lod == JadrenAnimationLod.Reduced && (boneIndex & 1) != 0)
                {
                    continue;
                }

                if (!currentClip.SampleBone(
                        boneIndex,
                        currentTime,
                        out var currentPosition,
                        out var currentRotation,
                        out var currentScale))
                {
                    continue;
                }

                var position = currentPosition;
                var rotation = currentRotation;
                var scale = currentScale;
                if (hasPrevious && previousClip.SampleBone(
                        boneIndex,
                        previousTime,
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
                        position = Vector3.LerpUnclamped(previousPosition, currentPosition, blend);
                        rotation = JadrenQuaternionMath.SlerpUnclamped(previousRotation, currentRotation, blend);
                        scale = Vector3.LerpUnclamped(previousScale, currentScale, blend);
                    }
                }

                output.Positions[boneIndex] = position;
                output.Rotations[boneIndex] = rotation;
                output.Scales[boneIndex] = scale;
                output.SampledBoneCount++;
            }

            if (currentClip.SampleBone(0, currentTime, out var rootPosition, out _, out _)
                && currentClip.SampleBone(0, currentPreviousTime, out var previousRootPosition, out _, out _))
            {
                output.RootMotionDelta = rootPosition - previousRootPosition;
            }
            output.Checksum = ComputeChecksum(output, boneCount, lod);
            return output.SampledBoneCount;
        }

        public static ulong ComputeChecksum(JadrenPoseBuffer pose, int boneCount, JadrenAnimationLod lod)
        {
            if (pose == null)
            {
                return 0UL;
            }

            var count = Mathf.Min(Mathf.Max(0, boneCount), pose.BoneCount);
            var hash = FnvOffset;
            Mix(ref hash, (uint)count);
            Mix(ref hash, (uint)lod);
            for (var boneIndex = 0; boneIndex < count; boneIndex++)
            {
                if (lod == JadrenAnimationLod.Reduced && (boneIndex & 1) != 0)
                {
                    continue;
                }
                Mix(ref hash, (uint)BitConverter.SingleToInt32Bits(pose.Positions[boneIndex].x));
                Mix(ref hash, (uint)BitConverter.SingleToInt32Bits(pose.Positions[boneIndex].y));
                Mix(ref hash, (uint)BitConverter.SingleToInt32Bits(pose.Positions[boneIndex].z));
                Mix(ref hash, (uint)BitConverter.SingleToInt32Bits(pose.Rotations[boneIndex].x));
                Mix(ref hash, (uint)BitConverter.SingleToInt32Bits(pose.Rotations[boneIndex].y));
                Mix(ref hash, (uint)BitConverter.SingleToInt32Bits(pose.Rotations[boneIndex].z));
                Mix(ref hash, (uint)BitConverter.SingleToInt32Bits(pose.Rotations[boneIndex].w));
                Mix(ref hash, (uint)BitConverter.SingleToInt32Bits(pose.Scales[boneIndex].x));
                Mix(ref hash, (uint)BitConverter.SingleToInt32Bits(pose.Scales[boneIndex].y));
                Mix(ref hash, (uint)BitConverter.SingleToInt32Bits(pose.Scales[boneIndex].z));
            }
            return hash;
        }

        private static void Mix(ref ulong hash, uint value)
        {
            hash ^= value;
            hash *= FnvPrime;
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
