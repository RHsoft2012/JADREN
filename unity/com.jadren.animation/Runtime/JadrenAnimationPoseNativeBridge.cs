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
        private readonly JadrenAnimationNativePoseTile8[] previousTiles;
        private readonly JadrenAnimationNativePoseTile8[] currentTiles;
        private readonly JadrenAnimationNativePoseTile8[] outputTiles;
        private readonly bool[] sampled;
        private bool slerpEnabled;
        private bool tileEnabled;

        public JadrenAnimationPoseNativeBridge(int boneCount)
        {
            var count = Mathf.Max(0, boneCount);
            previous = new JadrenAnimationNativePose[count];
            current = new JadrenAnimationNativePose[count];
            output = new JadrenAnimationNativePose[count];
            var tileCount = (count + 7) / 8;
            previousTiles = new JadrenAnimationNativePoseTile8[tileCount];
            currentTiles = new JadrenAnimationNativePoseTile8[tileCount];
            outputTiles = new JadrenAnimationNativePoseTile8[tileCount];
            sampled = new bool[count];
            slerpEnabled = count > 0 && JadrenAnimationNativeBatch.IsAvailable;
            tileEnabled = tileCount > 0 && slerpEnabled;
        }

        public bool IsAvailable { get { return slerpEnabled; } }
        public bool IsTileAvailable { get { return tileEnabled; } }

        public void Begin()
        {
            if (!slerpEnabled && !tileEnabled)
            {
                return;
            }

            Array.Clear(sampled, 0, sampled.Length);
        }

        public void Set(int boneIndex, Quaternion previousRotation, Quaternion currentRotation)
        {
            if (!slerpEnabled || boneIndex < 0 || boneIndex >= sampled.Length)
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

        public void SetLinear(
            int boneIndex,
            Vector3 previousPosition,
            Quaternion previousRotation,
            Vector3 previousScale,
            Vector3 currentPosition,
            Quaternion currentRotation,
            Vector3 currentScale)
        {
            if (!tileEnabled || boneIndex < 0 || boneIndex >= sampled.Length)
            {
                return;
            }

            var tileIndex = boneIndex >> 3;
            var lane = boneIndex & 7;
            ref var previousTile = ref previousTiles[tileIndex];
            ref var currentTile = ref currentTiles[tileIndex];
            SetLane(ref previousTile.PositionX, lane, previousPosition.x);
            SetLane(ref previousTile.PositionY, lane, previousPosition.y);
            SetLane(ref previousTile.PositionZ, lane, previousPosition.z);
            SetLane(ref previousTile.RotationX, lane, previousRotation.x);
            SetLane(ref previousTile.RotationY, lane, previousRotation.y);
            SetLane(ref previousTile.RotationZ, lane, previousRotation.z);
            SetLane(ref previousTile.RotationW, lane, previousRotation.w);
            SetLane(ref previousTile.ScaleX, lane, previousScale.x);
            SetLane(ref previousTile.ScaleY, lane, previousScale.y);
            SetLane(ref previousTile.ScaleZ, lane, previousScale.z);
            SetLane(ref currentTile.PositionX, lane, currentPosition.x);
            SetLane(ref currentTile.PositionY, lane, currentPosition.y);
            SetLane(ref currentTile.PositionZ, lane, currentPosition.z);
            SetLane(ref currentTile.RotationX, lane, currentRotation.x);
            SetLane(ref currentTile.RotationY, lane, currentRotation.y);
            SetLane(ref currentTile.RotationZ, lane, currentRotation.z);
            SetLane(ref currentTile.RotationW, lane, currentRotation.w);
            SetLane(ref currentTile.ScaleX, lane, currentScale.x);
            SetLane(ref currentTile.ScaleY, lane, currentScale.y);
            SetLane(ref currentTile.ScaleZ, lane, currentScale.z);
            sampled[boneIndex] = true;
        }

        public bool TryApplyLinear(
            Vector3[] destinationPositions,
            Vector3[] destinationScales,
            int boneCount,
            float fadeWeight,
            JadrenAnimationLod lod)
        {
            if (!tileEnabled || destinationPositions == null || destinationScales == null)
            {
                return false;
            }

            var count = Mathf.Min(
                Mathf.Max(0, boneCount),
                Mathf.Min(destinationPositions.Length, destinationScales.Length));
            var tileCount = (count + 7) / 8;
            int sampledTileCount;
            try
            {
                sampledTileCount = JadrenAnimationNativeBatch.BlendLinearAoSoA8(
                    previousTiles,
                    currentTiles,
                    outputTiles,
                    tileCount,
                    fadeWeight);
            }
            catch (DllNotFoundException)
            {
                tileEnabled = false;
                return false;
            }
            catch (EntryPointNotFoundException)
            {
                tileEnabled = false;
                return false;
            }
            catch (BadImageFormatException)
            {
                tileEnabled = false;
                return false;
            }

            if (sampledTileCount != tileCount)
            {
                tileEnabled = false;
                return false;
            }

            for (var boneIndex = 0; boneIndex < count; boneIndex++)
            {
                if (!sampled[boneIndex]
                    || lod == JadrenAnimationLod.Hidden
                    || (lod == JadrenAnimationLod.Reduced && (boneIndex & 1) != 0))
                {
                    continue;
                }

                var tile = outputTiles[boneIndex >> 3];
                var lane = boneIndex & 7;
                destinationPositions[boneIndex] = new Vector3(
                    GetLane(tile.PositionX, lane),
                    GetLane(tile.PositionY, lane),
                    GetLane(tile.PositionZ, lane));
                destinationScales[boneIndex] = new Vector3(
                    GetLane(tile.ScaleX, lane),
                    GetLane(tile.ScaleY, lane),
                    GetLane(tile.ScaleZ, lane));
            }
            return true;
        }

        public bool TryApply(Quaternion[] destination, int boneCount, float fadeWeight, JadrenAnimationLod lod)
        {
            if (!slerpEnabled || destination == null)
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
                slerpEnabled = false;
                return false;
            }
            catch (EntryPointNotFoundException)
            {
                slerpEnabled = false;
                return false;
            }
            catch (BadImageFormatException)
            {
                slerpEnabled = false;
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
                slerpEnabled = false;
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

        private static void SetLane(ref JadrenAnimationNativeFloat8 value, int lane, float laneValue)
        {
            switch (lane)
            {
                case 0: value.Lane0 = laneValue; break;
                case 1: value.Lane1 = laneValue; break;
                case 2: value.Lane2 = laneValue; break;
                case 3: value.Lane3 = laneValue; break;
                case 4: value.Lane4 = laneValue; break;
                case 5: value.Lane5 = laneValue; break;
                case 6: value.Lane6 = laneValue; break;
                case 7: value.Lane7 = laneValue; break;
                default: throw new ArgumentOutOfRangeException(nameof(lane));
            }
        }

        private static float GetLane(JadrenAnimationNativeFloat8 value, int lane)
        {
            switch (lane)
            {
                case 0: return value.Lane0;
                case 1: return value.Lane1;
                case 2: return value.Lane2;
                case 3: return value.Lane3;
                case 4: return value.Lane4;
                case 5: return value.Lane5;
                case 6: return value.Lane6;
                case 7: return value.Lane7;
                default: throw new ArgumentOutOfRangeException(nameof(lane));
            }
        }
    }
}
