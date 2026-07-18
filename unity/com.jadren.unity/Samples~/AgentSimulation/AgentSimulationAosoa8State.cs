using System;
using Unity.Collections;

namespace Jadren.Unity.Samples.AgentSimulation
{
    /// <summary>Caller-owned AoSoA8 storage with a logical agent count.</summary>
    public sealed class AgentSimulationAosoa8State : IDisposable
    {
        public const int Lanes = 8;
        public NativeArray<AgentTile> Tiles;

        public AgentSimulationAosoa8State(int count, Allocator allocator)
        {
            if (count <= 0)
            {
                throw new ArgumentOutOfRangeException(nameof(count));
            }

            Count = count;
            TileCount = checked((count + Lanes - 1) / Lanes);
            Tiles = new NativeArray<AgentTile>(TileCount, allocator);
        }

        public int Count { get; }
        public int TileCount { get; }

        public void Dispose()
        {
            if (Tiles.IsCreated)
            {
                Tiles.Dispose();
            }
        }
    }
}
