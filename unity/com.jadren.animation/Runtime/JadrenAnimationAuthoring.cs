using UnityEngine;

namespace Jadren.Animation
{
    [DisallowMultipleComponent]
    public sealed class JadrenAnimationAuthoring : MonoBehaviour
    {
        [SerializeField] private JadrenRigAsset rig;
        [SerializeField] private JadrenControllerAsset controller;
        [SerializeField] private JadrenAnimationLod defaultLod = JadrenAnimationLod.Full;

        public JadrenRigAsset Rig { get { return rig; } }
        public JadrenControllerAsset Controller { get { return controller; } }
        public JadrenAnimationLod DefaultLod { get { return defaultLod; } set { defaultLod = value; } }
        public bool IsConfigured { get { return rig != null && controller != null && rig.BoneCount > 0; } }

        // Called by the editor baker and intentionally kept as a simple asset assignment.
        public void AssignBakedAssets(JadrenRigAsset bakedRig, JadrenControllerAsset bakedController)
        {
            rig = bakedRig;
            controller = bakedController;
        }
    }
}
