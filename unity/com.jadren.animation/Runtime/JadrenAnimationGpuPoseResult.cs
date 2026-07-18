using System;
using UnityEngine;

namespace Jadren.Animation
{
    /// <summary>
    /// Completed GPU rotation snapshot crossing back to the Unity main thread.
    /// GPU code publishes a copy only after readback completion; the result
    /// never stores a ComputeBuffer, NativeArray or Unity scene object.
    /// </summary>
    public sealed class JadrenAnimationGpuPoseResult : IDisposable
    {
        private readonly Quaternion[] rotations;
        private readonly int boneCount;
        private readonly JadrenAnimationLod lod;
        private bool completed;
        private bool succeeded;
        private int sampledBoneCount;
        private string failureReason = string.Empty;

        public JadrenAnimationGpuPoseResult(int boneCount, JadrenAnimationLod lod)
        {
            if (boneCount < 0)
            {
                throw new ArgumentOutOfRangeException(nameof(boneCount));
            }

            this.boneCount = boneCount;
            this.lod = lod;
            rotations = new Quaternion[boneCount];
        }

        public int BoneCount { get { return boneCount; } }
        public JadrenAnimationLod Lod { get { return lod; } }
        public bool IsComplete { get { return completed; } }
        public bool Succeeded { get { return completed && succeeded; } }
        public int SampledBoneCount { get { return sampledBoneCount; } }
        public string FailureReason { get { return failureReason; } }

        /// <summary>
        /// Publishes a fully read-back rotation array. The source is copied so
        /// the caller can release its GPU/readback storage immediately after
        /// this call. A partial LOD result is rejected before publication.
        /// </summary>
        public bool TryPublishCompleted(Quaternion[] source, int sampleCount)
        {
            if (completed || source == null || source.Length < boneCount)
            {
                return false;
            }

            var expectedCount = ExpectedSampleCount(boneCount, lod);
            if (sampleCount != expectedCount)
            {
                failureReason = "sample_count_invalid";
                return false;
            }

            for (var boneIndex = 0; boneIndex < boneCount; boneIndex++)
            {
                if (lod == JadrenAnimationLod.Reduced && (boneIndex & 1) != 0)
                {
                    continue;
                }
                rotations[boneIndex] = source[boneIndex];
            }

            sampledBoneCount = sampleCount;
            succeeded = true;
            completed = true;
            failureReason = string.Empty;
            return true;
        }

        /// <summary>
        /// Completes a failed dispatch without exposing partially written
        /// rotations to the applier. A host may then choose CPU fallback.
        /// </summary>
        public bool TryPublishFailure(string reason)
        {
            if (completed)
            {
                return false;
            }

            completed = true;
            succeeded = false;
            sampledBoneCount = 0;
            failureReason = string.IsNullOrEmpty(reason) ? "gpu_dispatch_failed" : reason;
            return true;
        }

        /// <summary>Reads one completed rotation without exposing the backing array.</summary>
        public bool TryGetRotation(int boneIndex, out Quaternion rotation)
        {
            rotation = Quaternion.identity;
            if (!Succeeded || boneIndex < 0 || boneIndex >= boneCount)
            {
                return false;
            }
            if (lod == JadrenAnimationLod.Reduced && (boneIndex & 1) != 0)
            {
                return false;
            }
            rotation = rotations[boneIndex];
            return true;
        }

        internal bool TryCopyTo(JadrenPoseBuffer pose)
        {
            if (!Succeeded || pose == null || pose.BoneCount < boneCount)
            {
                return false;
            }

            for (var boneIndex = 0; boneIndex < boneCount; boneIndex++)
            {
                if (lod == JadrenAnimationLod.Reduced && (boneIndex & 1) != 0)
                {
                    continue;
                }
                pose.Rotations[boneIndex] = rotations[boneIndex];
            }
            pose.Checksum = JadrenPoseKernel.ComputeChecksum(pose, boneCount, lod);
            return true;
        }

        public void Dispose()
        {
            completed = true;
            succeeded = false;
            sampledBoneCount = 0;
            failureReason = "disposed";
            GC.SuppressFinalize(this);
        }

        private static int ExpectedSampleCount(int count, JadrenAnimationLod lod)
        {
            return lod == JadrenAnimationLod.Hidden
                ? 0
                : lod == JadrenAnimationLod.Reduced
                    ? (count + 1) / 2
                    : count;
        }
    }
}
