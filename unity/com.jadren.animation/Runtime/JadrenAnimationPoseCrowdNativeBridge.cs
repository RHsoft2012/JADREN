using System;
using UnityEngine;

namespace Jadren.Animation
{
    /// <summary>
    /// Persistent caller-owned staging for one weighted AoSoA8 native call
    /// across many agents. The bridge is pure data code and never touches
    /// Animator, Transform, jobs, or Unity object lifetime.
    /// </summary>
    internal sealed class JadrenAnimationPoseCrowdNativeBridge
    {
        private readonly int boneCount;
        private readonly int tilesPerAgent;
        private readonly int agentCapacity;
        private readonly JadrenAnimationNativePoseTile8[] previousTiles;
        private readonly JadrenAnimationNativePoseTile8[] currentTiles;
        private readonly JadrenAnimationNativePoseTile8[] outputTiles;
        private readonly float[] tileWeights;
        private readonly bool[] sampled;
        private bool enabled;
        private int activeAgentCount;

        public JadrenAnimationPoseCrowdNativeBridge(int boneCount, int agentCapacity)
        {
            this.boneCount = Mathf.Max(0, boneCount);
            this.agentCapacity = Mathf.Max(0, agentCapacity);
            tilesPerAgent = (this.boneCount + 7) / 8;
            var tileCapacity = checked(tilesPerAgent * this.agentCapacity);
            previousTiles = new JadrenAnimationNativePoseTile8[tileCapacity];
            currentTiles = new JadrenAnimationNativePoseTile8[tileCapacity];
            outputTiles = new JadrenAnimationNativePoseTile8[tileCapacity];
            tileWeights = new float[tileCapacity];
            sampled = new bool[checked(this.boneCount * this.agentCapacity)];
            enabled = tileCapacity > 0 && JadrenAnimationNativeBatch.IsAvailable;
        }

        public bool IsAvailable { get { return enabled; } }
        public int AgentCapacity { get { return agentCapacity; } }

        public void Begin(int agentCount)
        {
            if (agentCount < 0 || agentCount > agentCapacity)
            {
                throw new ArgumentOutOfRangeException(nameof(agentCount));
            }
            activeAgentCount = agentCount;
            if (enabled && agentCount > 0)
            {
                Array.Clear(sampled, 0, checked(agentCount * boneCount));
            }
        }

        public void SetLinear(
            int agentIndex,
            int boneIndex,
            float fadeWeight,
            Vector3 previousPosition,
            Quaternion previousRotation,
            Vector3 previousScale,
            Vector3 currentPosition,
            Quaternion currentRotation,
            Vector3 currentScale)
        {
            if (!enabled
                || agentIndex < 0
                || agentIndex >= activeAgentCount
                || boneIndex < 0
                || boneIndex >= boneCount)
            {
                return;
            }

            var tileIndex = checked(agentIndex * tilesPerAgent + (boneIndex >> 3));
            var lane = boneIndex & 7;
            tileWeights[tileIndex] = fadeWeight;
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
            sampled[checked(agentIndex * boneCount + boneIndex)] = true;
        }

        public bool TryApplyLinear(
            JadrenPoseBuffer[] destinations,
            JadrenAnimationLod[] lods,
            int agentCount)
        {
            if (!enabled
                || destinations == null
                || lods == null
                || agentCount < 0
                || agentCount > activeAgentCount
                || agentCount > destinations.Length
                || agentCount > lods.Length)
            {
                return false;
            }

            var tileCount = checked(agentCount * tilesPerAgent);
            int sampledTileCount;
            try
            {
                sampledTileCount = JadrenAnimationNativeBatch.BlendLinearAoSoA8Weighted(
                    previousTiles,
                    currentTiles,
                    outputTiles,
                    tileWeights,
                    tileCount);
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

            if (sampledTileCount != tileCount)
            {
                enabled = false;
                return false;
            }

            for (var agent = 0; agent < agentCount; agent++)
            {
                var destination = destinations[agent];
                if (destination == null || lods[agent] == JadrenAnimationLod.Hidden)
                {
                    continue;
                }
                for (var bone = 0; bone < boneCount; bone++)
                {
                    if (!sampled[checked(agent * boneCount + bone)]
                        || (lods[agent] == JadrenAnimationLod.Reduced && (bone & 1) != 0))
                    {
                        continue;
                    }

                    var tile = outputTiles[checked(agent * tilesPerAgent + (bone >> 3))];
                    var lane = bone & 7;
                    destination.Positions[bone] = new Vector3(
                        GetLane(tile.PositionX, lane),
                        GetLane(tile.PositionY, lane),
                        GetLane(tile.PositionZ, lane));
                    destination.Scales[bone] = new Vector3(
                        GetLane(tile.ScaleX, lane),
                        GetLane(tile.ScaleY, lane),
                        GetLane(tile.ScaleZ, lane));
                }
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
