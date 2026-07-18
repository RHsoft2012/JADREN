using System;
using UnityEngine;

namespace Jadren.Animation
{
    /// <summary>
    /// Completed GPU skinning positions. The result is published only after
    /// readback completion and never owns a ComputeBuffer or scene object.
    /// </summary>
    public sealed class JadrenAnimationGpuSkinningResult : IDisposable
    {
        private readonly Vector3[] positions;
        private readonly int vertexCount;
        private bool completed;
        private bool succeeded;
        private string failureReason = string.Empty;

        public JadrenAnimationGpuSkinningResult(int vertexCount)
        {
            if (vertexCount < 0)
            {
                throw new ArgumentOutOfRangeException(nameof(vertexCount));
            }
            this.vertexCount = vertexCount;
            positions = new Vector3[vertexCount];
        }

        public int VertexCount { get { return vertexCount; } }
        public bool IsComplete { get { return completed; } }
        public bool Succeeded { get { return completed && succeeded; } }
        public string FailureReason { get { return failureReason; } }

        public bool TryPublishCompleted(Vector3[] source)
        {
            if (completed || source == null || source.Length < vertexCount)
            {
                return false;
            }
            for (var index = 0; index < vertexCount; index++)
            {
                var position = source[index];
                if (float.IsNaN(position.x) || float.IsInfinity(position.x)
                    || float.IsNaN(position.y) || float.IsInfinity(position.y)
                    || float.IsNaN(position.z) || float.IsInfinity(position.z))
                {
                    failureReason = "position_non_finite";
                    return false;
                }
                positions[index] = position;
            }
            completed = true;
            succeeded = true;
            failureReason = string.Empty;
            return true;
        }

        public bool TryPublishFailure(string reason)
        {
            if (completed)
            {
                return false;
            }
            completed = true;
            succeeded = false;
            failureReason = string.IsNullOrEmpty(reason) ? "gpu_skinning_failed" : reason;
            return true;
        }

        public bool TryGetPosition(int vertexIndex, out Vector3 position)
        {
            position = Vector3.zero;
            if (!Succeeded || vertexIndex < 0 || vertexIndex >= vertexCount)
            {
                return false;
            }
            position = positions[vertexIndex];
            return true;
        }

        public void Dispose()
        {
            completed = true;
            succeeded = false;
            failureReason = "disposed";
            GC.SuppressFinalize(this);
        }
    }
}
