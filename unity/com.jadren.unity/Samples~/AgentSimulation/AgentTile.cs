using System.Runtime.InteropServices;

namespace Jadren.Unity.Samples.AgentSimulation
{
    /// <summary>
    /// Internal AoSoA8 tile: six Float8 fields, eight agents per record.
    /// The public AgentState sequential AoS ABI is unchanged.
    /// </summary>
    [StructLayout(LayoutKind.Sequential, Pack = 4)]
    public struct AgentTile
    {
        public AgentFloat8 PositionX;
        public AgentFloat8 PositionY;
        public AgentFloat8 PositionZ;
        public AgentFloat8 VelocityX;
        public AgentFloat8 VelocityY;
        public AgentFloat8 VelocityZ;
    }
}
