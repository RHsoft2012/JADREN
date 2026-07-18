using System.Runtime.InteropServices;

namespace Jadren.Unity.Samples.AgentSimulation
{
    /// <summary>
    /// Blittable reference layout shared by the managed baseline and future
    /// generated batch kernels.
    /// </summary>
    [StructLayout(LayoutKind.Sequential)]
    public struct AgentState
    {
        public float PositionX;
        public float PositionY;
        public float PositionZ;
        public float VelocityX;
        public float VelocityY;
        public float VelocityZ;
    }
}
