using UnityEngine;

namespace Jadren.Animation.Samples
{
    /// <summary>Small sample hook that reports the selected capability route.</summary>
    public sealed class JadrenAnimationSample : MonoBehaviour
    {
        [SerializeField] private int agentCount = 1;
        [SerializeField] private bool boundsReady;
        [SerializeField] private bool bufferResident;

        public JadrenAnimationGpuPlan LastPlan { get; private set; }

        private void OnEnable()
        {
            RefreshPlan();
        }

        public void RefreshPlan()
        {
            LastPlan = JadrenAnimationGpuAdapter.Plan(
                new JadrenAnimationGpuRequest(
                    JadrenAnimationGpuTarget.Auto,
                    Mathf.Max(1, agentCount),
                    boundsReady,
                    bufferResident,
                    true));
        }
    }
}
