using System.Runtime.CompilerServices;

namespace Jadren.Unity.Samples.AgentSimulation
{
    /// <summary>Pure blittable update shared by the managed and Burst samples.</summary>
    public static class AgentSimulationMath
    {
        [MethodImpl(MethodImplOptions.AggressiveInlining)]
        public static void Integrate(ref AgentState agent, float deltaTime)
        {
            agent.PositionX += agent.VelocityX * deltaTime;
            agent.PositionY += agent.VelocityY * deltaTime;
            agent.PositionZ += agent.VelocityZ * deltaTime;
        }
    }
}
