using Unity.Collections;

namespace Jadren.Unity.Samples.AgentSimulation
{
    /// <summary>Shared deterministic data setup and managed reference loop.</summary>
    public static class AgentSimulationWorkload
    {
        public static void Initialize(NativeArray<AgentState> agents)
        {
            for (var index = 0; index < agents.Length; index++)
            {
                agents[index] = new AgentState
                {
                    PositionX = index,
                    PositionY = index * 0.5f,
                    PositionZ = -index * 0.25f,
                    VelocityX = 1.0f + index * 0.01f,
                    VelocityY = 0.5f,
                    VelocityZ = -0.25f
                };
            }
        }

        public static void StepManaged(NativeArray<AgentState> agents, float deltaTime)
        {
            for (var index = 0; index < agents.Length; index++)
            {
                var agent = agents[index];
                AgentSimulationMath.Integrate(ref agent, deltaTime);
                agents[index] = agent;
            }
        }

        public static double Checksum(NativeArray<AgentState> agents)
        {
            var checksum = 0.0;
            for (var index = 0; index < agents.Length; index++)
            {
                var agent = agents[index];
                checksum += agent.PositionX;
                checksum += agent.PositionY * 3.0;
                checksum += agent.PositionZ * 5.0;
            }
            return checksum;
        }
    }
}
