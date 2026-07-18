using System;
using System.Runtime.InteropServices;
using UnityEngine;

namespace Jadren.Animation
{
    /// <summary>
    /// Explicit 44-byte GPU skinning input layout. Bone indices are stored as
    /// integer-valued floats so the Unity and HLSL structured-buffer ABI stays
    /// portable without unsafe pointer casts.
    /// </summary>
    [StructLayout(LayoutKind.Sequential, Pack = 4)]
    public struct JadrenGpuSkinningVertex
    {
        public Vector3 Position;
        public Vector4 BoneWeights;
        public Vector4 BoneIndices;

        public JadrenGpuSkinningVertex(
            Vector3 position,
            Vector4 boneWeights,
            Vector4 boneIndices)
        {
            Position = position;
            BoneWeights = boneWeights;
            BoneIndices = boneIndices;
        }

        public static int StrideBytes { get { return 44; } }

        /// <summary>Validates host data before it reaches a ComputeBuffer.</summary>
        public bool TryValidate(int boneCount, out string failureReason)
        {
            failureReason = string.Empty;
            if (boneCount < 1)
            {
                failureReason = "bone_count_invalid";
                return false;
            }
            if (!IsFinite(Position.x) || !IsFinite(Position.y) || !IsFinite(Position.z))
            {
                failureReason = "position_non_finite";
                return false;
            }

            var weights = BoneWeights;
            var indices = BoneIndices;
            for (var lane = 0; lane < 4; lane++)
            {
                var weight = GetLane(weights, lane);
                var index = GetLane(indices, lane);
                if (!IsFinite(weight) || weight < 0.0f)
                {
                    failureReason = "weight_invalid";
                    return false;
                }
                if (!IsFinite(index)
                    || index < 0.0f
                    || index >= boneCount
                    || Mathf.Abs(index - Mathf.Round(index)) > 0.0001f)
                {
                    failureReason = "bone_index_invalid";
                    return false;
                }
            }
            return true;
        }

        private static float GetLane(Vector4 value, int lane)
        {
            switch (lane)
            {
                case 0: return value.x;
                case 1: return value.y;
                case 2: return value.z;
                default: return value.w;
            }
        }

        private static bool IsFinite(float value)
        {
            return !float.IsNaN(value) && !float.IsInfinity(value);
        }
    }
}
