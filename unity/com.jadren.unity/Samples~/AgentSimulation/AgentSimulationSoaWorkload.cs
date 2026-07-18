using Unity.Collections;

namespace Jadren.Unity.Samples.AgentSimulation
{
    /// <summary>Deterministic SoA setup and scalar reference loop.</summary>
    public static class AgentSimulationSoaWorkload
    {
        public static void Initialize(AgentSimulationSoaState agents)
        {
            for (var index = 0; index < agents.Length; index++)
            {
                agents.PositionX[index] = index;
                agents.PositionY[index] = index * 0.5f;
                agents.PositionZ[index] = -index * 0.25f;
                agents.VelocityX[index] = 1.0f + index * 0.01f;
                agents.VelocityY[index] = 0.5f;
                agents.VelocityZ[index] = -0.25f;
            }
        }

        public static void StepManaged(AgentSimulationSoaState agents, float deltaTime)
        {
            for (var index = 0; index < agents.Length; index++)
            {
                agents.PositionX[index] += agents.VelocityX[index] * deltaTime;
                agents.PositionY[index] += agents.VelocityY[index] * deltaTime;
                agents.PositionZ[index] += agents.VelocityZ[index] * deltaTime;
            }
        }

        public static double Checksum(AgentSimulationSoaState agents)
        {
            var checksum = 0.0;
            for (var index = 0; index < agents.Length; index++)
            {
                checksum += agents.PositionX[index];
                checksum += agents.PositionY[index] * 3.0;
                checksum += agents.PositionZ[index] * 5.0;
            }
            return checksum;
        }
    }
}
