using UnityEngine;

namespace Jadren.Animation
{
    /// <summary>Main-thread-only sink for a worker-produced pose buffer.</summary>
    [DisallowMultipleComponent]
    [RequireComponent(typeof(JadrenAnimationAuthoring))]
    public sealed class JadrenAnimationPoseApplier : MonoBehaviour
    {
        private Transform[] bones = new Transform[0];

        public int BoundBoneCount { get { return bones.Length; } }

        public void RebuildBindings(JadrenRigAsset rig, Transform root)
        {
            if (rig == null || root == null)
            {
                bones = new Transform[0];
                return;
            }

            bones = new Transform[rig.BoneCount];
            for (var i = 0; i < bones.Length; i++)
            {
                var path = rig.GetBonePath(i);
                bones[i] = string.IsNullOrEmpty(path) ? root : root.Find(path);
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
                bone.localPosition = pose.Positions[boneIndex];
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
