using System;
using UnityEngine;

namespace Jadren.Animation
{
    [Serializable]
    public struct JadrenAnimationStateDefinition
    {
        public string name;
        public JadrenClipAsset clip;
        public float speedThreshold;
        public float playbackSpeed;
        public bool loop;
    }

    [Serializable]
    public struct JadrenAnimationTransition
    {
        public int fromState;
        public int toState;
        public float minimumSpeed;
        public float fadeSeconds;
    }

    [CreateAssetMenu(fileName = "JadrenController", menuName = "Jadren/Animation/Controller Asset")]
    public sealed class JadrenControllerAsset : ScriptableObject
    {
        [SerializeField] private string cacheKey;
        [SerializeField] private JadrenAnimationStateDefinition[] states = Array.Empty<JadrenAnimationStateDefinition>();
        [SerializeField] private JadrenAnimationTransition[] transitions = Array.Empty<JadrenAnimationTransition>();

        public string CacheKey { get { return cacheKey; } }
        public int StateCount { get { return states == null ? 0 : states.Length; } }

        public JadrenAnimationStateDefinition GetState(int index)
        {
            if (states == null || index < 0 || index >= states.Length)
            {
                return default;
            }
            return states[index];
        }

        public int ResolveState(int currentState, float speed)
        {
            if (states == null || states.Length == 0)
            {
                return -1;
            }

            var nextState = Mathf.Clamp(currentState, 0, states.Length - 1);
            var bestThreshold = float.NegativeInfinity;
            for (var i = 0; i < states.Length; i++)
            {
                var threshold = states[i].speedThreshold;
                if (speed >= threshold && threshold >= bestThreshold)
                {
                    nextState = i;
                    bestThreshold = threshold;
                }
            }
            return Mathf.Clamp(nextState, 0, states.Length - 1);
        }

        // Called by the editor baker. Runtime code only evaluates the arrays.
        public void SetBakedData(
            JadrenAnimationStateDefinition[] bakedStates,
            JadrenAnimationTransition[] bakedTransitions,
            string bakedKey = null)
        {
            cacheKey = bakedKey ?? string.Empty;
            states = bakedStates ?? Array.Empty<JadrenAnimationStateDefinition>();
            transitions = bakedTransitions ?? Array.Empty<JadrenAnimationTransition>();
        }
    }
}
