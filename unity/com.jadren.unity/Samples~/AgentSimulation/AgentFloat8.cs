using System.Runtime.InteropServices;

namespace Jadren.Unity.Samples.AgentSimulation
{
    /// <summary>Blittable eight-f32 tile lane group matching Jadren Float8.</summary>
    [StructLayout(LayoutKind.Sequential, Pack = 4)]
    public struct AgentFloat8
    {
        public float Lane0;
        public float Lane1;
        public float Lane2;
        public float Lane3;
        public float Lane4;
        public float Lane5;
        public float Lane6;
        public float Lane7;

        public float Get(int lane)
        {
            switch (lane)
            {
                case 0: return Lane0;
                case 1: return Lane1;
                case 2: return Lane2;
                case 3: return Lane3;
                case 4: return Lane4;
                case 5: return Lane5;
                case 6: return Lane6;
                case 7: return Lane7;
                default: return 0.0f;
            }
        }

        public void Set(int lane, float value)
        {
            switch (lane)
            {
                case 0: Lane0 = value; break;
                case 1: Lane1 = value; break;
                case 2: Lane2 = value; break;
                case 3: Lane3 = value; break;
                case 4: Lane4 = value; break;
                case 5: Lane5 = value; break;
                case 6: Lane6 = value; break;
                case 7: Lane7 = value; break;
                default: break;
            }
        }
    }
}
