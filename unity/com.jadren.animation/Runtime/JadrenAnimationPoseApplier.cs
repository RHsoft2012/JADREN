using UnityEngine;

namespace Jadren.Animation
{
    /// <summary>Main-thread-only sink for a worker-produced pose buffer.</summary>
    [DisallowMultipleComponent]
    [RequireComponent(typeof(JadrenAnimationAuthoring))]
    public sealed class JadrenAnimationPoseApplier : MonoBehaviour
    {
        private Transform[] bones = new Transform[0];
        private int rootBoneIndex = -1;

        [SerializeField]
        private bool applyRootMotion;

        public int BoundBoneCount { get { return bones.Length; } }
        /// <summary>
        /// Keeps the spawned character root at its gameplay position by
        /// default. Enable only when the host explicitly consumes the baked
        /// root-motion track as a Transform position.
        /// </summary>
        public bool ApplyRootMotion
        {
            get { return applyRootMotion; }
            set { applyRootMotion = value; }
        }

        public void RebuildBindings(JadrenRigAsset rig, Transform root)
        {
            if (rig == null || root == null)
            {
                bones = new Transform[0];
                rootBoneIndex = -1;
                return;
            }

            bones = new Transform[rig.BoneCount];
            rootBoneIndex = -1;
            for (var i = 0; i < bones.Length; i++)
            {
                var path = rig.GetBonePath(i);
                if (string.IsNullOrEmpty(path))
                {
                    rootBoneIndex = i;
                    bones[i] = root;
                }
                else
                {
                    bones[i] = root.Find(path);
                }
            }
        }

        public void Apply(JadrenPoseBuffer pose, JadrenAnimationLod lod)
        {
            if (pose == null || lod == JadrenAnimationLod.Hidden)
            {
                return;
            }

            var boneCount = Mathf.Min(pose.BoneCount, bones.Length);
            var reduced = lod == JadrenAnimationLod.Reduced;
            for (var boneIndex = 0; boneIndex < boneCount; boneIndex++)
            {
                if (reduced && (boneIndex & 1) != 0)
                {
                    continue;
                }

                var bone = bones[boneIndex];
                if (bone == null)
                {
                    continue;
                }
                if (applyRootMotion || boneIndex != rootBoneIndex)
                {
                    bone.localPosition = pose.Positions[boneIndex];
                }
                bone.localRotation = pose.Rotations[boneIndex];
                bone.localScale = pose.Scales[boneIndex];
            }
        }

        /// <summary>
        /// Main-thread completion boundary for an optional GPU rotation result.
        /// The result is copied into the caller-owned pose only after a
        /// successful completed readback; pending or failed results are never
        /// applied and return false for an explicit CPU fallback decision.
        /// </summary>
        public bool ApplyGpuResult(JadrenPoseBuffer pose, JadrenAnimationGpuPoseResult result)
        {
            if (pose == null || result == null || !result.IsComplete || !result.Succeeded)
            {
                return false;
            }
            if (!result.TryCopyTo(pose))
            {
                return false;
            }
            Apply(pose, result.Lod);
            return true;
        }
    }
}
