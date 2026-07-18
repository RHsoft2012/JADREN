using Unity.Collections;

namespace Jadren.Unity.Samples.AgentSimulation
{
    /// <summary>Deterministic AoSoA8 setup, scalar reference and checksum.</summary>
    public static class AgentSimulationAosoa8Workload
    {
        public static void Initialize(AgentSimulationAosoa8State agents)
        {
            for (var tileIndex = 0; tileIndex < agents.TileCount; tileIndex++)
            {
                var tile = default(AgentTile);
                for (var lane = 0; lane < AgentSimulationAosoa8State.Lanes; lane++)
                {
                    var index = tileIndex * AgentSimulationAosoa8State.Lanes + lane;
                    if (index >= agents.Count)
                    {
                        break;
                    }

                    tile.PositionX.Set(lane, index);
                    tile.PositionY.Set(lane, index * 0.5f);
                    tile.PositionZ.Set(lane, -index * 0.25f);
                    tile.VelocityX.Set(lane, 1.0f + index * 0.01f);
                    tile.VelocityY.Set(lane, 0.5f);
                    tile.VelocityZ.Set(lane, -0.25f);
                }
                agents.Tiles[tileIndex] = tile;
            }
        }

        public static void StepManaged(AgentSimulationAosoa8State agents, float deltaTime)
        {
            for (var tileIndex = 0; tileIndex < agents.TileCount; tileIndex++)
            {
                var tile = agents.Tiles[tileIndex];
                for (var lane = 0; lane < AgentSimulationAosoa8State.Lanes; lane++)
                {
                    var index = tileIndex * AgentSimulationAosoa8State.Lanes + lane;
                    if (index >= agents.Count)
                    {
                        break;
                    }

                    tile.PositionX.Set(lane, tile.PositionX.Get(lane) + tile.VelocityX.Get(lane) * deltaTime);
                    tile.PositionY.Set(lane, tile.PositionY.Get(lane) + tile.VelocityY.Get(lane) * deltaTime);
                    tile.PositionZ.Set(lane, tile.PositionZ.Get(lane) + tile.VelocityZ.Get(lane) * deltaTime);
                }
                agents.Tiles[tileIndex] = tile;
            }
        }

        public static double Checksum(AgentSimulationAosoa8State agents)
        {
            var checksum = 0.0;
            for (var tileIndex = 0; tileIndex < agents.TileCount; tileIndex++)
            {
                var tile = agents.Tiles[tileIndex];
                for (var lane = 0; lane < AgentSimulationAosoa8State.Lanes; lane++)
                {
                    var index = tileIndex * AgentSimulationAosoa8State.Lanes + lane;
                    if (index >= agents.Count)
                    {
                        break;
                    }

                    checksum += tile.PositionX.Get(lane);
                    checksum += tile.PositionY.Get(lane) * 3.0;
                    checksum += tile.PositionZ.Get(lane) * 5.0;
                }
            }
            return checksum;
        }
    }
}
