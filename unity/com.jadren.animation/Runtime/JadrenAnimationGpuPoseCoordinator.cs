using System;
using UnityEngine;

namespace Jadren.Animation
{
    /// <summary>Non-blocking state returned by the opt-in GPU pose coordinator.</summary>
    public enum JadrenAnimationGpuPoseApplyStatus : byte
    {
        NoPending = 0,
        Pending = 1,
        Applied = 2,
        Failed = 3
    }

    /// <summary>
    /// Double-buffered host boundary for one in-flight GPU pose. A worker
    /// supplies a completed base TRS snapshot and rotation inputs; the main
    /// thread queues them, polls completion and applies the matching snapshot.
    /// The coordinator never mutates the pose currently being evaluated.
    /// </summary>
    public sealed class JadrenAnimationGpuPoseCoordinator : IDisposable
    {
        private readonly JadrenAnimationGpuPoseDispatcher dispatcher;
        private readonly JadrenPoseBuffer[] snapshots =
        {
            new JadrenPoseBuffer(),
            new JadrenPoseBuffer()
        };
        private JadrenAnimationGpuPoseDispatch pendingDispatch;
        private int pendingSnapshot = -1;
        private int nextSnapshot;
        private bool disposed;

        public JadrenAnimationGpuPoseCoordinator(ComputeShader shader)
        {
            dispatcher = new JadrenAnimationGpuPoseDispatcher(shader);
        }

        public bool IsAvailable { get { return !disposed && dispatcher.IsAvailable; } }
        public bool HasPending { get { return pendingDispatch != null; } }
        public string LastFailureReason { get; private set; } = string.Empty;
        public JadrenPoseBuffer LastAppliedPose { get; private set; }

        /// <summary>
        /// Queues one snapshot without blocking. Only one dispatch is in flight;
        /// the second slot preserves the pose until its readback is applied.
        /// </summary>
        public bool TryQueue(
            JadrenPoseBuffer basePose,
            Quaternion[] previous,
            Quaternion[] current,
            float[] weights,
            int boneCount,
            JadrenAnimationLod lod,
            out string failureReason)
        {
            failureReason = string.Empty;
            if (disposed)
            {
                failureReason = "coordinator_disposed";
                return false;
            }
            if (pendingDispatch != null)
            {
                failureReason = "gpu_pose_already_pending";
                return false;
            }
            if (basePose == null || basePose.BoneCount < boneCount || boneCount < 1)
            {
                failureReason = "base_pose_invalid";
                return false;
            }

            var snapshotIndex = nextSnapshot;
            var snapshot = snapshots[snapshotIndex];
            CopyPose(basePose, snapshot, boneCount);
            if (!dispatcher.TryDispatch(
                    previous,
                    current,
                    weights,
                    boneCount,
                    lod,
                    out pendingDispatch,
                    out failureReason))
            {
                LastFailureReason = failureReason;
                pendingDispatch = null;
                return false;
            }

            pendingSnapshot = snapshotIndex;
            nextSnapshot = (snapshotIndex + 1) & 1;
            LastFailureReason = string.Empty;
            return true;
        }

        /// <summary>
        /// Polls without waiting. Call this from the Unity main-thread update
        /// phase; Pending leaves both the snapshot and GPU handle untouched.
        /// </summary>
        public JadrenAnimationGpuPoseApplyStatus PollAndApply(
            JadrenAnimationPoseApplier applier)
        {
            if (disposed || pendingDispatch == null)
            {
                return JadrenAnimationGpuPoseApplyStatus.NoPending;
            }
            if (!pendingDispatch.IsDone)
            {
                return JadrenAnimationGpuPoseApplyStatus.Pending;
            }
            return FinishApply(applier, pendingDispatch.Complete());
        }

        /// <summary>
        /// Blocking completion path for shutdown/editor validation. Runtime
        /// frame loops should use PollAndApply instead.
        /// </summary>
        public JadrenAnimationGpuPoseApplyStatus CompleteAndApply(
            JadrenAnimationPoseApplier applier)
        {
            if (disposed || pendingDispatch == null)
            {
                return JadrenAnimationGpuPoseApplyStatus.NoPending;
            }
            return FinishApply(applier, pendingDispatch.Complete());
        }

        public void CancelPending()
        {
            if (pendingDispatch == null)
            {
                return;
            }
            pendingDispatch.Dispose();
            pendingDispatch = null;
            pendingSnapshot = -1;
            LastFailureReason = "gpu_pose_cancelled";
        }

        public void Dispose()
        {
            if (disposed)
            {
                return;
            }
            CancelPending();
            dispatcher.Dispose();
            disposed = true;
            GC.SuppressFinalize(this);
        }

        private JadrenAnimationGpuPoseApplyStatus FinishApply(
            JadrenAnimationPoseApplier applier,
            JadrenAnimationGpuPoseResult result)
        {
            var dispatch = pendingDispatch;
            var snapshotIndex = pendingSnapshot;
            pendingDispatch = null;
            pendingSnapshot = -1;
            try
            {
                if (result == null || !result.Succeeded)
                {
                    LastFailureReason = result == null
                        ? "gpu_pose_result_missing"
                        : result.FailureReason;
                    return JadrenAnimationGpuPoseApplyStatus.Failed;
                }
                if (applier == null || !applier.ApplyGpuResult(snapshots[snapshotIndex], result))
                {
                    LastFailureReason = "gpu_pose_applier_rejected";
                    return JadrenAnimationGpuPoseApplyStatus.Failed;
                }
                LastAppliedPose = snapshots[snapshotIndex];
                LastFailureReason = string.Empty;
                return JadrenAnimationGpuPoseApplyStatus.Applied;
            }
            finally
            {
                dispatch.Dispose();
            }
        }

        private static void CopyPose(JadrenPoseBuffer source, JadrenPoseBuffer destination, int boneCount)
        {
            destination.EnsureCapacity(boneCount);
            Array.Copy(source.Positions, destination.Positions, boneCount);
            Array.Copy(source.Rotations, destination.Rotations, boneCount);
            Array.Copy(source.Scales, destination.Scales, boneCount);
            destination.SampledBoneCount = source.SampledBoneCount;
            destination.RootMotionDelta = source.RootMotionDelta;
            destination.Checksum = source.Checksum;
        }
    }
}
