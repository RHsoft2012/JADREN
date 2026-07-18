using System;
using Unity.Collections;

namespace Jadren.Unity.Samples.AgentSimulation
{
    /// <summary>
    /// Explicit structure-of-arrays storage for the vectorization benchmark.
    /// Each component has its own caller-owned NativeArray and no hidden
    /// header or implicit AoS/SoA conversion is involved.
    /// </summary>
    public sealed class AgentSimulationSoaState : IDisposable
    {
        public NativeArray<float> PositionX;
        public NativeArray<float> PositionY;
        public NativeArray<float> PositionZ;
        public NativeArray<float> VelocityX;
        public NativeArray<float> VelocityY;
        public NativeArray<float> VelocityZ;

        public AgentSimulationSoaState(int count, Allocator allocator)
        {
            if (count <= 0)
            {
                throw new ArgumentOutOfRangeException(nameof(count));
            }

            PositionX = new NativeArray<float>(count, allocator);
            PositionY = new NativeArray<float>(count, allocator);
            PositionZ = new NativeArray<float>(count, allocator);
            VelocityX = new NativeArray<float>(count, allocator);
            VelocityY = new NativeArray<float>(count, allocator);
            VelocityZ = new NativeArray<float>(count, allocator);
        }

        public int Length => PositionX.Length;

        public void Dispose()
        {
            if (PositionX.IsCreated) PositionX.Dispose();
            if (PositionY.IsCreated) PositionY.Dispose();
            if (PositionZ.IsCreated) PositionZ.Dispose();
            if (VelocityX.IsCreated) VelocityX.Dispose();
            if (VelocityY.IsCreated) VelocityY.Dispose();
            if (VelocityZ.IsCreated) VelocityZ.Dispose();
        }
    }
}
