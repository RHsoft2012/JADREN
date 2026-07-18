using System;
using UnityEngine;

namespace Jadren.Animation
{
    [CreateAssetMenu(fileName = "JadrenClip", menuName = "Jadren/Animation/Clip Asset")]
    public sealed class JadrenClipAsset : ScriptableObject
    {
        [SerializeField] private string sourceName;
        [SerializeField] private string cacheKey;
        [SerializeField] private int rigBoneCount;
        [SerializeField] private int frameCount;
        [SerializeField] private float sampleRate = 30.0f;
        [SerializeField] private float duration;
        [SerializeField] private bool loop = true;
        [SerializeField] private Vector3[] translations = Array.Empty<Vector3>();
        [SerializeField] private Quaternion[] rotations = Array.Empty<Quaternion>();
        [SerializeField] private Vector3[] scales = Array.Empty<Vector3>();

        public string SourceName { get { return sourceName; } }
        public string CacheKey { get { return cacheKey; } }
        public int RigBoneCount { get { return rigBoneCount; } }
        public int FrameCount { get { return frameCount; } }
        public float SampleRate { get { return sampleRate; } }
        public float Duration { get { return duration; } }
        public bool Loop { get { return loop; } }

        /// <summary>
        /// Copies baked arrays once into a worker-owned snapshot. Runtime
        /// worker code must not read ScriptableObject fields or Unity assets.
        /// </summary>
        public void CopyBakedData(
            out Vector3[] bakedTranslations,
            out Quaternion[] bakedRotations,
            out Vector3[] bakedScales)
        {
            bakedTranslations = translations == null
                ? Array.Empty<Vector3>()
                : (Vector3[])translations.Clone();
            bakedRotations = rotations == null
                ? Array.Empty<Quaternion>()
                : (Quaternion[])rotations.Clone();
            bakedScales = scales == null
                ? Array.Empty<Vector3>()
                : (Vector3[])scales.Clone();
        }

        public void SetBakedData(
            string source,
            int boneCount,
            int frames,
            float rate,
            float clipDuration,
            bool shouldLoop,
            Vector3[] bakedTranslations,
            Quaternion[] bakedRotations,
            Vector3[] bakedScales,
            string bakedKey = null)
        {
            sourceName = source ?? string.Empty;
            cacheKey = bakedKey ?? string.Empty;
            rigBoneCount = Mathf.Max(0, boneCount);
            frameCount = Mathf.Max(0, frames);
            sampleRate = Mathf.Max(1.0f, rate);
            duration = Mathf.Max(0.0f, clipDuration);
            loop = shouldLoop;
            translations = bakedTranslations ?? Array.Empty<Vector3>();
            rotations = bakedRotations ?? Array.Empty<Quaternion>();
            scales = bakedScales ?? Array.Empty<Vector3>();
        }

        public bool SampleBone(
            int boneIndex,
            float time,
            out Vector3 position,
            out Quaternion rotation,
            out Vector3 scale)
        {
            position = Vector3.zero;
            rotation = Quaternion.identity;
            scale = Vector3.one;
            if (rigBoneCount <= 0 || frameCount <= 0 || boneIndex < 0 || boneIndex >= rigBoneCount)
            {
                return false;
            }

            var sampleTime = duration <= 0.0f ? 0.0f : time;
            if (loop && duration > 0.0f)
            {
                sampleTime %= duration;
                if (sampleTime < 0.0f)
                {
                    sampleTime += duration;
                }
            }
            else
            {
                sampleTime = Mathf.Clamp(sampleTime, 0.0f, duration);
            }

            var frame = sampleTime * sampleRate;
            var first = Mathf.Clamp(Mathf.FloorToInt(frame), 0, frameCount - 1);
            var second = Mathf.Min(first + 1, frameCount - 1);
            var weight = Mathf.Clamp01(frame - first);
            var firstIndex = first * rigBoneCount + boneIndex;
            var secondIndex = second * rigBoneCount + boneIndex;

            if (translations != null && secondIndex < translations.Length)
            {
                position = Vector3.LerpUnclamped(translations[firstIndex], translations[secondIndex], weight);
            }
            if (rotations != null && secondIndex < rotations.Length)
            {
                // Keep the asset/reference path bit-for-bit on the same
                // shortest-arc contract as the worker snapshot. This avoids
                // a managed fallback silently using a different quaternion
                // implementation than the worker/applier path.
                rotation = JadrenQuaternionMath.SlerpUnclamped(
                    rotations[firstIndex],
                    rotations[secondIndex],
                    weight);
            }
            if (scales != null && secondIndex < scales.Length)
            {
                scale = Vector3.LerpUnclamped(scales[firstIndex], scales[secondIndex], weight);
            }
            return true;
        }
    }
}
