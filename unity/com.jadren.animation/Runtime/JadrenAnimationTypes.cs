using System;

namespace Jadren.Animation
{
    public enum JadrenAnimationLod : byte
    {
        Full = 0,
        Reduced = 1,
        Hidden = 2
    }

    [Serializable]
    public struct JadrenAnimationState
    {
        public int stateIndex;
        public float time;
        public float speed;
        public float fadeWeight;
        public JadrenAnimationLod lod;

        public static JadrenAnimationState Default
        {
            get
            {
                return new JadrenAnimationState
                {
                    stateIndex = 0,
                    time = 0.0f,
                    speed = 0.0f,
                    fadeWeight = 1.0f,
                    lod = JadrenAnimationLod.Full
                };
            }
        }
    }
}
