using System;
using UnityEngine;

namespace Jadren.Animation
{
    /// <summary>
    /// Optional packed quaternion bridge used by the worker when a validated
    /// Jadren native animation plugin is present. It owns fixed-size staging
    /// arrays and never touches Unity objects or allocates during a frame.
    /// The managed Slerp path remains the safe default.
    /// </summary>
    internal sealed class JadrenAnimationPoseNativeBridge
    {
        private readonly JadrenAnimationNativePose[] previous;
        private readonly JadrenAnimationNativePose[] current;
        private readonly JadrenAnimationNativePose[] output;
        private readonly bool[] sampled;
        private bool enabled;

        public JadrenAnimationPoseNativeBridge(int boneCount)
        {
            var count = Mathf.Max(0, boneCount);
            previous = new JadrenAnimationNativePose[count];
            current = new JadrenAnimationNativePose[count];
            output = new JadrenAnimationNativePose[count];
            sampled = new bool[count];
            enabled = count > 0 && JadrenAnimationNativeBatch.IsAvailable;
        }

        public bool IsAvailable { get { return enabled; } }

        public void Begin()
        {
            if (!enabled)
            {
                return;
            }

            Array.Clear(sampled, 0, sampled.Length);
        }

        public void Set(int boneIndex, Quaternion previousRotation, Quaternion currentRotation)
        {
            if (!enabled || boneIndex < 0 || boneIndex >= sampled.Length)
            {
                return;
            }

            previous[boneIndex].RotationX = previousRotation.x;
            previous[boneIndex].RotationY = previousRotation.y;
            previous[boneIndex].RotationZ = previousRotation.z;
            previous[boneIndex].RotationW = previousRotation.w;
            current[boneIndex].RotationX = currentRotation.x;
            current[boneIndex].RotationY = currentRotation.y;
            current[boneIndex].RotationZ = currentRotation.z;
            current[boneIndex].RotationW = currentRotation.w;
            sampled[boneIndex] = true;
        }

        public bool TryApply(Quaternion[] destination, int boneCount, float fadeWeight, JadrenAnimationLod lod)
        {
            if (!enabled || destination == null)
            {
                return false;
            }

            var count = Mathf.Min(Mathf.Max(0, boneCount), destination.Length);
            int sampledCount;
            try
            {
                sampledCount = JadrenAnimationNativeBatch.BlendSlerpUnclamped(
                    previous,
                    current,
                    output,
                    count,
                    fadeWeight,
                    lod);
            }
            catch (DllNotFoundException)
            {
                enabled = false;
                return false;
            }
            catch (EntryPointNotFoundException)
            {
                enabled = false;
                return false;
            }
            catch (BadImageFormatException)
            {
                enabled = false;
                return false;
            }

            // The worker can safely fall back to managed Slerp only when the
            // native call did not publish a partial/short result. Treat an
            // unexpected count as an invalid backend response and disable the
            // bridge so subsequent frames stay on the managed path.
            var expectedCount = lod == JadrenAnimationLod.Hidden
                ? 0
                : lod == JadrenAnimationLod.Reduced
                    ? (count + 1) / 2
                    : count;
            if (sampledCount != expectedCount)
            {
                enabled = false;
                return false;
            }

            for (var boneIndex = 0; boneIndex < count; boneIndex++)
            {
                if (!sampled[boneIndex]
                    || (lod == JadrenAnimationLod.Reduced && (boneIndex & 1) != 0))
                {
                    continue;
                }

                var pose = output[boneIndex];
                destination[boneIndex] = new Quaternion(
                    pose.RotationX,
                    pose.RotationY,
                    pose.RotationZ,
                    pose.RotationW);
            }
            return true;
        }
    }
}
