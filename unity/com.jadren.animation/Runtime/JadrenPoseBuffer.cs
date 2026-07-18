using System;
using UnityEngine;

namespace Jadren.Animation
{
    /// <summary>
    /// Caller-owned scalar pose output. The arrays are resized only when the rig
    /// changes, so a frame step does not allocate managed memory.
    /// </summary>
    public sealed class JadrenPoseBuffer
    {
        private Vector3[] positions = Array.Empty<Vector3>();
        private Quaternion[] rotations = Array.Empty<Quaternion>();
        private Vector3[] scales = Array.Empty<Vector3>();

        public Vector3[] Positions { get { return positions; } }
        public Quaternion[] Rotations { get { return rotations; } }
        public Vector3[] Scales { get { return scales; } }
        public int BoneCount { get { return positions.Length; } }
        public int SampledBoneCount { get; internal set; }
        public Vector3 RootMotionDelta { get; internal set; }
        public ulong Checksum { get; internal set; }

        public void EnsureCapacity(int boneCount)
        {
            var count = Mathf.Max(0, boneCount);
            if (positions.Length == count && rotations.Length == count && scales.Length == count)
            {
                return;
            }

            positions = new Vector3[count];
            rotations = new Quaternion[count];
            scales = new Vector3[count];
        }
    }
}
