using System;
using UnityEngine;

namespace Jadren.Animation
{
    [CreateAssetMenu(fileName = "JadrenRig", menuName = "Jadren/Animation/Rig Asset")]
    public sealed class JadrenRigAsset : ScriptableObject
    {
        [SerializeField] private string sourceName;
        [SerializeField] private string cacheKey;
        [SerializeField] private string[] boneNames = Array.Empty<string>();
        [SerializeField] private string[] bonePaths = Array.Empty<string>();
        [SerializeField] private int[] parentIndices = Array.Empty<int>();
        [SerializeField] private Vector3[] bindPositions = Array.Empty<Vector3>();
        [SerializeField] private Quaternion[] bindRotations = Array.Empty<Quaternion>();
        [SerializeField] private Vector3[] bindScales = Array.Empty<Vector3>();

        public string SourceName { get { return sourceName; } }
        public string CacheKey { get { return cacheKey; } }
        public int BoneCount { get { return boneNames == null ? 0 : boneNames.Length; } }
        public IReadOnlyListView BoneNames { get { return new IReadOnlyListView(boneNames); } }

        public string GetBonePath(int index)
        {
            return bonePaths != null && index >= 0 && index < bonePaths.Length ? bonePaths[index] : string.Empty;
        }

        public int GetParentIndex(int index)
        {
            return parentIndices != null && index >= 0 && index < parentIndices.Length ? parentIndices[index] : -1;
        }

        public void GetBindPose(int index, out Vector3 position, out Quaternion rotation, out Vector3 scale)
        {
            position = bindPositions != null && index >= 0 && index < bindPositions.Length
                ? bindPositions[index]
                : Vector3.zero;
            rotation = bindRotations != null && index >= 0 && index < bindRotations.Length
                ? bindRotations[index]
                : Quaternion.identity;
            scale = bindScales != null && index >= 0 && index < bindScales.Length
                ? bindScales[index]
                : Vector3.one;
        }

        public bool TryGetBoneIndex(string path, out int index)
        {
            if (bonePaths != null)
            {
                for (var i = 0; i < bonePaths.Length; i++)
                {
                    if (string.Equals(bonePaths[i], path, StringComparison.Ordinal))
                    {
                        index = i;
                        return true;
                    }
                }
            }

            index = -1;
            return false;
        }

        // Called by the editor baker. Runtime code only reads the resulting asset.
        public void SetBakedData(
            string source,
            string[] names,
            string[] paths,
            int[] parents,
            Vector3[] positions,
            Quaternion[] rotations,
            Vector3[] scales,
            string bakedKey = null)
        {
            sourceName = source ?? string.Empty;
            cacheKey = bakedKey ?? string.Empty;
            boneNames = names ?? Array.Empty<string>();
            bonePaths = paths ?? Array.Empty<string>();
            parentIndices = parents ?? Array.Empty<int>();
            bindPositions = positions ?? Array.Empty<Vector3>();
            bindRotations = rotations ?? Array.Empty<Quaternion>();
            bindScales = scales ?? Array.Empty<Vector3>();
        }

        public readonly struct IReadOnlyListView
        {
            private readonly string[] values;

            public IReadOnlyListView(string[] source)
            {
                values = source ?? Array.Empty<string>();
            }

            public int Count { get { return values.Length; } }
            public string this[int index] { get { return values[index]; } }
        }
    }
}
