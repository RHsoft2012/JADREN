#if UNITY_EDITOR
using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;
using UnityEditor;
using UnityEngine;
using Object = UnityEngine.Object;

namespace Jadren.Animation.Editor
{
    public static class JadrenAnimationBaker
    {
        private const string OutputFolder = "Assets/JadrenAnimationGenerated";
        private const float SampleRate = 30.0f;
        private const int MaxClipsPerBake = 16;
        private const int BakeFormatVersion = 1;
        private const float PositionTolerance = 0.001f;
        private const float RotationToleranceDegrees = 0.1f;

        [MenuItem("Tools/Jadren/Animation/Bake Selected Animator", false, 220)]
        private static void BakeSelectedAnimator()
        {
            var selected = Selection.activeGameObject;
            var animator = selected == null
                ? null
                : selected.GetComponent<Animator>() ?? selected.GetComponentInChildren<Animator>();
            if (animator == null)
            {
                EditorUtility.DisplayDialog("Jadren Animation", "Vyber GameObject s Animator komponentom.", "OK");
                return;
            }
            if (animator.runtimeAnimatorController == null)
            {
                EditorUtility.DisplayDialog("Jadren Animation", "Animator nemá RuntimeAnimatorController.", "OK");
                return;
            }

            EnsureOutputFolder();
            var root = animator.transform;
            var rigData = CollectRig(root);
            var rig = CreateRigAsset(root, rigData);
            var clips = CreateClipAssets(animator, root, rigData, rig.CacheKey);
            var controller = CreateControllerAsset(animator, clips);

            var authoring = animator.GetComponent<JadrenAnimationAuthoring>();
            if (authoring == null)
            {
                authoring = Undo.AddComponent<JadrenAnimationAuthoring>(animator.gameObject);
            }
            authoring.AssignBakedAssets(rig, controller);
            EditorUtility.SetDirty(authoring);

            if (animator.GetComponent<JadrenAnimationPlayer>() == null)
            {
                Undo.AddComponent<JadrenAnimationPlayer>(animator.gameObject);
            }

            AssetDatabase.SaveAssets();
            AssetDatabase.Refresh();
            Selection.activeObject = controller;
            Debug.Log($"Jadren animation bake hotový: rig={rig.BoneCount}, clips={clips.Count}, root='{root.name}'.");
        }

        [MenuItem("Tools/Jadren/Animation/Validate Selected Bake", false, 221)]
        private static void ValidateSelectedBake()
        {
            var selected = Selection.activeGameObject;
            var animator = selected == null
                ? null
                : selected.GetComponent<Animator>() ?? selected.GetComponentInChildren<Animator>();
            if (animator == null)
            {
                EditorUtility.DisplayDialog("Jadren Animation", "Vyber GameObject s Animator komponentom.", "OK");
                return;
            }

            var authoring = animator.GetComponent<JadrenAnimationAuthoring>();
            if (authoring == null || authoring.Rig == null || authoring.Controller == null)
            {
                ReportValidationFailure("Vybraný Animator nemá priradený Jadren rig/controller asset.");
                return;
            }
            if (string.IsNullOrEmpty(authoring.Rig.CacheKey) || string.IsNullOrEmpty(authoring.Controller.CacheKey))
            {
                ReportValidationFailure("Bake assety nemajú cache key. Spusť najprv Bake Selected Animator.");
                return;
            }

            var expectedControllerKey = BuildControllerCacheKey(animator, authoring.Controller);
            if (!string.Equals(expectedControllerKey, authoring.Controller.CacheKey, StringComparison.Ordinal))
            {
                ReportValidationFailure("Controller cache key nezodpovedá zdrojovému AnimatorController.");
                return;
            }

            var sourceClips = animator.runtimeAnimatorController == null
                ? Array.Empty<AnimationClip>()
                : animator.runtimeAnimatorController.animationClips;
            var clone = Object.Instantiate(animator.gameObject);
            clone.name = "__JadrenAnimationParity__";
            clone.hideFlags = HideFlags.HideAndDontSave;
            var checkedClips = 0;
            var checkedBones = 0;
            var maxPositionError = 0.0f;
            var maxRotationError = 0.0f;
            var parityPose = new JadrenPoseBuffer();
            var lastChecksum = 0UL;
            try
            {
                clone.SetActive(false);
                for (var stateIndex = 0; stateIndex < authoring.Controller.StateCount; stateIndex++)
                {
                    var state = authoring.Controller.GetState(stateIndex);
                    if (state.clip == null)
                    {
                        continue;
                    }

                    var source = FindSourceClip(sourceClips, state.clip.SourceName);
                    if (source == null)
                    {
                        ReportValidationFailure($"Zdrojový Unity klip '{state.clip.SourceName}' nebol nájdený.");
                        return;
                    }
                    var expectedClipKey = BuildClipCacheKey(source, authoring.Rig.CacheKey);
                    if (!string.Equals(expectedClipKey, state.clip.CacheKey, StringComparison.Ordinal))
                    {
                        ReportValidationFailure($"Clip cache key nezodpovedá zdroju '{source.name}'.");
                        return;
                    }

                    source.SampleAnimation(clone, 0.0f);
                    JadrenPoseKernel.Sample(
                        authoring.Rig,
                        state.clip,
                        0.0f,
                        0.0f,
                        null,
                        0.0f,
                        1.0f,
                        JadrenAnimationLod.Full,
                        parityPose);
                    lastChecksum = parityPose.Checksum;
                    for (var boneIndex = 0; boneIndex < authoring.Rig.BoneCount; boneIndex++)
                    {
                        var path = authoring.Rig.GetBonePath(boneIndex);
                        var sampledTransform = string.IsNullOrEmpty(path)
                            ? clone.transform
                            : clone.transform.Find(path);
                        if (sampledTransform == null || boneIndex >= parityPose.BoneCount)
                        {
                            ReportValidationFailure($"Chýba bone {boneIndex} ({path}) v parity vzorke.");
                            return;
                        }

                        var positionError = Vector3.Distance(sampledTransform.localPosition, parityPose.Positions[boneIndex]);
                        var rotationError = Quaternion.Angle(sampledTransform.localRotation, parityPose.Rotations[boneIndex]);
                        var scaleError = Vector3.Distance(sampledTransform.localScale, parityPose.Scales[boneIndex]);
                        maxPositionError = Mathf.Max(maxPositionError, positionError, scaleError);
                        maxRotationError = Mathf.Max(maxRotationError, rotationError);
                        if (positionError > PositionTolerance || scaleError > PositionTolerance || rotationError > RotationToleranceDegrees)
                        {
                            ReportValidationFailure(
                                $"Parity mismatch: clip='{source.name}', bone={boneIndex}, " +
                                $"position={positionError.ToString("R", CultureInfo.InvariantCulture)}, " +
                                $"scale={scaleError.ToString("R", CultureInfo.InvariantCulture)}, " +
                                $"rotationDeg={rotationError.ToString("R", CultureInfo.InvariantCulture)}.");
                            return;
                        }
                        checkedBones++;
                    }
                    checkedClips++;
                }
                var kernelContractFailure = ValidateKernelContract(authoring.Rig, authoring.Controller);
                if (!string.IsNullOrEmpty(kernelContractFailure))
                {
                    ReportValidationFailure(kernelContractFailure);
                    return;
                }
            }
            finally
            {
                Object.DestroyImmediate(clone);
            }

            var message =
                $"Jadren parity PASS: clips={checkedClips}, bones={checkedBones}, " +
                $"maxPositionOrScaleError={maxPositionError.ToString("R", CultureInfo.InvariantCulture)}, " +
                $"maxRotationErrorDeg={maxRotationError.ToString("R", CultureInfo.InvariantCulture)}, " +
                $"checksum=0x{lastChecksum:X16}.";
            Debug.Log(message, animator);
            EditorUtility.DisplayDialog("Jadren Animation", message, "OK");
        }

        private static string ValidateKernelContract(JadrenRigAsset rig, JadrenControllerAsset controller)
        {
            var slerpFailure = ValidateQuaternionSlerpParity();
            if (!string.IsNullOrEmpty(slerpFailure))
            {
                return slerpFailure;
            }

            if (rig == null || controller == null || controller.StateCount == 0)
            {
                return "Scalar kernel nemá rig alebo controller state.";
            }

            var firstClip = controller.GetState(0).clip;
            if (firstClip == null)
            {
                return "Scalar kernel nemá prvý clip.";
            }

            var sampleTime = firstClip.Duration > 0.0f ? firstClip.Duration * 0.5f : 0.0f;
            var full = new JadrenPoseBuffer();
            var repeat = new JadrenPoseBuffer();
            JadrenPoseKernel.Sample(
                rig,
                firstClip,
                sampleTime,
                sampleTime,
                null,
                0.0f,
                1.0f,
                JadrenAnimationLod.Full,
                full);
            JadrenPoseKernel.Sample(
                rig,
                firstClip,
                sampleTime,
                sampleTime,
                null,
                0.0f,
                1.0f,
                JadrenAnimationLod.Full,
                repeat);
            if (full.Checksum != repeat.Checksum)
            {
                return "Scalar kernel checksum nie je opakovane deterministický.";
            }

            var worker = new JadrenAnimationPoseWorker(rig, controller);
            var workerPose = new JadrenPoseBuffer();
            worker.Evaluate(
                0,
                sampleTime,
                sampleTime,
                -1,
                0.0f,
                1.0f,
                JadrenAnimationLod.Full,
                workerPose);
            if (workerPose.Checksum != full.Checksum
                || workerPose.SampledBoneCount != full.SampledBoneCount)
            {
                return $"Worker pose parity zlyhala: checksum 0x{workerPose.Checksum:X16} != 0x{full.Checksum:X16}.";
            }

            var reduced = new JadrenPoseBuffer();
            JadrenPoseKernel.Sample(
                rig,
                firstClip,
                sampleTime,
                sampleTime,
                null,
                0.0f,
                1.0f,
                JadrenAnimationLod.Reduced,
                reduced);
            var expectedReducedBones = (rig.BoneCount + 1) / 2;
            if (reduced.SampledBoneCount != expectedReducedBones)
            {
                return $"Reduced kernel sample count {reduced.SampledBoneCount} != {expectedReducedBones}.";
            }

            if (firstClip.Duration > 0.0f
                && firstClip.SampleBone(0, sampleTime, out var rootNow, out _, out _)
                && firstClip.SampleBone(0, 0.0f, out var rootBefore, out _, out _))
            {
                var rootError = Vector3.Distance(reduced.RootMotionDelta, Vector3.zero);
                JadrenPoseKernel.Sample(
                    rig,
                    firstClip,
                    sampleTime,
                    0.0f,
                    null,
                    0.0f,
                    1.0f,
                    JadrenAnimationLod.Full,
                    full);
                rootError = Vector3.Distance(full.RootMotionDelta, rootNow - rootBefore);
                if (rootError > 0.00001f)
                {
                    return $"Root-motion delta chyba {rootError.ToString("R", CultureInfo.InvariantCulture)}.";
                }
            }

            if (controller.StateCount > 1)
            {
                var secondClip = controller.GetState(1).clip;
                if (secondClip != null)
                {
                    var previousOnly = new JadrenPoseBuffer();
                    var previousReference = new JadrenPoseBuffer();
                    var currentReference = new JadrenPoseBuffer();
                    JadrenPoseKernel.Sample(
                        rig,
                        firstClip,
                        sampleTime,
                        sampleTime,
                        null,
                        0.0f,
                        1.0f,
                        JadrenAnimationLod.Full,
                        previousReference);
                    JadrenPoseKernel.Sample(
                        rig,
                        secondClip,
                        sampleTime,
                        sampleTime,
                        null,
                        0.0f,
                        1.0f,
                        JadrenAnimationLod.Full,
                        currentReference);
                    JadrenPoseKernel.Sample(
                        rig,
                        secondClip,
                        sampleTime,
                        sampleTime,
                        firstClip,
                        sampleTime,
                        0.0f,
                        JadrenAnimationLod.Full,
                        previousOnly);
                    if (previousOnly.Checksum != previousReference.Checksum)
                    {
                        return "Cross-fade weight 0 nevracia predchádzajúci clip.";
                    }

                    JadrenPoseKernel.Sample(
                        rig,
                        secondClip,
                        sampleTime,
                        sampleTime,
                        firstClip,
                        sampleTime,
                        1.0f,
                        JadrenAnimationLod.Full,
                        previousOnly);
                    if (previousOnly.Checksum != currentReference.Checksum)
                    {
                        return "Cross-fade weight 1 nevracia aktuálny clip.";
                    }

                    worker.Evaluate(
                        1,
                        sampleTime,
                        sampleTime,
                        0,
                        sampleTime,
                        0.5f,
                        JadrenAnimationLod.Full,
                        previousOnly);
                    var workerCrossFade = new JadrenPoseBuffer();
                    JadrenPoseKernel.Sample(
                        rig,
                        secondClip,
                        sampleTime,
                        sampleTime,
                        firstClip,
                        sampleTime,
                        0.5f,
                        JadrenAnimationLod.Full,
                        workerCrossFade);
                    if (previousOnly.Checksum != workerCrossFade.Checksum)
                    {
                        return $"Worker cross-fade parity zlyhala: checksum 0x{previousOnly.Checksum:X16} != 0x{workerCrossFade.Checksum:X16}.";
                    }
                }
            }
            return string.Empty;
        }

        private static string ValidateQuaternionSlerpParity()
        {
            var pairs = new[]
            {
                new QuaternionPair(Quaternion.identity, Quaternion.Euler(25.0f, 70.0f, -15.0f)),
                new QuaternionPair(Quaternion.Euler(0.0f, 179.0f, 0.0f), Quaternion.Euler(0.0f, -179.0f, 0.0f)),
                new QuaternionPair(Quaternion.Euler(-45.0f, 10.0f, 90.0f), Quaternion.Euler(80.0f, -20.0f, 5.0f))
            };
            var weights = new[] { -0.25f, 0.0f, 0.25f, 0.5f, 1.0f, 1.25f };
            for (var pairIndex = 0; pairIndex < pairs.Length; pairIndex++)
            {
                for (var weightIndex = 0; weightIndex < weights.Length; weightIndex++)
                {
                    var expected = Quaternion.SlerpUnclamped(
                        pairs[pairIndex].a,
                        pairs[pairIndex].b,
                        weights[weightIndex]);
                    var actual = JadrenQuaternionMath.SlerpUnclamped(
                        pairs[pairIndex].a,
                        pairs[pairIndex].b,
                        weights[weightIndex]);
                    var error = Quaternion.Angle(expected, actual);
                    if (error > 0.001f)
                    {
                        return $"Jadren Slerp parity chyba pair={pairIndex}, weight={weights[weightIndex].ToString("R", CultureInfo.InvariantCulture)}, degrees={error.ToString("R", CultureInfo.InvariantCulture)}.";
                    }
                }
            }
            return string.Empty;
        }

        private readonly struct QuaternionPair
        {
            public readonly Quaternion a;
            public readonly Quaternion b;

            public QuaternionPair(Quaternion first, Quaternion second)
            {
                a = first;
                b = second;
            }
        }

        [MenuItem("Tools/Jadren/Animation/Validate Selected Bake", true)]
        private static bool ValidateSelectedBakeMenu()
        {
            var selected = Selection.activeGameObject;
            return selected != null && selected.GetComponentInChildren<Animator>() != null;
        }

        [MenuItem("Tools/Jadren/Animation/Bake Selected Animator", true)]
        private static bool ValidateBakeSelectedAnimator()
        {
            var selected = Selection.activeGameObject;
            return selected != null && selected.GetComponentInChildren<Animator>() != null;
        }

        private static void EnsureOutputFolder()
        {
            if (!AssetDatabase.IsValidFolder(OutputFolder))
            {
                AssetDatabase.CreateFolder("Assets", "JadrenAnimationGenerated");
            }
        }

        private static JadrenRigAsset CreateRigAsset(Transform root, RigBakeData data)
        {
            var path = $"{OutputFolder}/{Sanitize(root.name)}_Rig.asset";
            var asset = LoadOrCreateAsset<JadrenRigAsset>(path);
            asset.SetBakedData(
                root.name,
                data.Names.ToArray(),
                data.Paths.ToArray(),
                data.Parents.ToArray(),
                data.Positions.ToArray(),
                data.Rotations.ToArray(),
                data.Scales.ToArray(),
                data.CacheKey);
            EditorUtility.SetDirty(asset);
            return asset;
        }

        private static List<JadrenClipAsset> CreateClipAssets(Animator animator, Transform root, RigBakeData rig, string rigCacheKey)
        {
            var result = new List<JadrenClipAsset>();
            var sourceClips = animator.runtimeAnimatorController.animationClips;
            var seen = new HashSet<AnimationClip>();
            for (var i = 0; i < sourceClips.Length && result.Count < MaxClipsPerBake; i++)
            {
                var sourceClip = sourceClips[i];
                if (sourceClip == null || !seen.Add(sourceClip))
                {
                    continue;
                }

                var path = $"{OutputFolder}/{Sanitize(root.name)}_{Sanitize(sourceClip.name)}.asset";
                var asset = LoadOrCreateAsset<JadrenClipAsset>(path);
                BakeClip(asset, sourceClip, root, rig, rigCacheKey);
                EditorUtility.SetDirty(asset);
                result.Add(asset);
            }
            return result;
        }

        private static void BakeClip(JadrenClipAsset destination, AnimationClip source, Transform root, RigBakeData rig, string rigCacheKey)
        {
            var duration = Mathf.Max(source.length, 1.0f / SampleRate);
            var frameCount = Mathf.Max(2, Mathf.CeilToInt(duration * SampleRate) + 1);
            var valueCount = frameCount * rig.Paths.Count;
            var translations = new Vector3[valueCount];
            var rotations = new Quaternion[valueCount];
            var scales = new Vector3[valueCount];
            var sampleObject = Object.Instantiate(root.gameObject);
            sampleObject.name = "__JadrenAnimationBake__";
            sampleObject.hideFlags = HideFlags.HideAndDontSave;

            try
            {
                for (var frame = 0; frame < frameCount; frame++)
                {
                    var time = Mathf.Min(duration, frame / SampleRate);
                    source.SampleAnimation(sampleObject, time);
                    for (var bone = 0; bone < rig.Paths.Count; bone++)
                    {
                        var transform = rig.Paths[bone].Length == 0
                            ? sampleObject.transform
                            : sampleObject.transform.Find(rig.Paths[bone]);
                        var index = frame * rig.Paths.Count + bone;
                        if (transform == null)
                        {
                            translations[index] = rig.Positions[bone];
                            rotations[index] = rig.Rotations[bone];
                            scales[index] = rig.Scales[bone];
                            continue;
                        }
                        translations[index] = transform.localPosition;
                        rotations[index] = transform.localRotation;
                        scales[index] = transform.localScale;
                    }
                }
            }
            finally
            {
                Object.DestroyImmediate(sampleObject);
            }

            var settings = AnimationUtility.GetAnimationClipSettings(source);
            destination.SetBakedData(
                source.name,
                rig.Paths.Count,
                frameCount,
                SampleRate,
                duration,
                settings.loopTime,
                translations,
                rotations,
                scales,
                BuildClipCacheKey(source, rigCacheKey));
        }

        private static JadrenControllerAsset CreateControllerAsset(Animator animator, List<JadrenClipAsset> clips)
        {
            var states = new JadrenAnimationStateDefinition[clips.Count];
            for (var i = 0; i < clips.Count; i++)
            {
                var clip = clips[i];
                states[i] = new JadrenAnimationStateDefinition
                {
                    name = clip.SourceName,
                    clip = clip,
                    speedThreshold = GuessSpeedThreshold(clip.SourceName, i),
                    playbackSpeed = 1.0f,
                    loop = clip.Loop
                };
            }

            var path = $"{OutputFolder}/{Sanitize(animator.name)}_Controller.asset";
            var controller = LoadOrCreateAsset<JadrenControllerAsset>(path);
            controller.SetBakedData(
                states,
                Array.Empty<JadrenAnimationTransition>(),
                BuildControllerCacheKey(animator, clips));
            EditorUtility.SetDirty(controller);
            return controller;
        }

        private static T LoadOrCreateAsset<T>(string path) where T : ScriptableObject
        {
            var asset = AssetDatabase.LoadAssetAtPath<T>(path);
            if (asset != null)
            {
                return asset;
            }

            asset = ScriptableObject.CreateInstance<T>();
            AssetDatabase.CreateAsset(asset, path);
            return asset;
        }

        private static string BuildControllerCacheKey(Animator animator, JadrenControllerAsset controller)
        {
            var clips = new List<JadrenClipAsset>();
            for (var i = 0; i < controller.StateCount; i++)
            {
                var state = controller.GetState(i);
                if (state.clip != null)
                {
                    clips.Add(state.clip);
                }
            }
            return BuildControllerCacheKey(animator, clips);
        }

        private static string BuildControllerCacheKey(Animator animator, List<JadrenClipAsset> clips)
        {
            var source = animator.runtimeAnimatorController;
            var sourcePath = source == null ? string.Empty : AssetDatabase.GetAssetPath(source);
            var sourceGuid = string.IsNullOrEmpty(sourcePath) ? string.Empty : AssetDatabase.AssetPathToGUID(sourcePath);
            var dependency = string.IsNullOrEmpty(sourcePath)
                ? string.Empty
                : AssetDatabase.GetAssetDependencyHash(sourcePath).ToString();
            var builder = new StringBuilder();
            builder.Append("JADREN-CONTROLLER|v").Append(BakeFormatVersion)
                .Append("|name=").Append(source == null ? string.Empty : source.name)
                .Append("|guid=").Append(sourceGuid)
                .Append("|dep=").Append(dependency)
                .Append("|count=").Append(clips.Count);
            for (var i = 0; i < clips.Count; i++)
            {
                builder.Append("|clip=").Append(clips[i] == null ? string.Empty : clips[i].CacheKey);
            }
            return StableHash(builder.ToString());
        }

        private static string BuildClipCacheKey(AnimationClip source, string rigCacheKey)
        {
            var sourcePath = source == null ? string.Empty : AssetDatabase.GetAssetPath(source);
            var sourceGuid = string.IsNullOrEmpty(sourcePath) ? string.Empty : AssetDatabase.AssetPathToGUID(sourcePath);
            var dependency = string.IsNullOrEmpty(sourcePath)
                ? string.Empty
                : AssetDatabase.GetAssetDependencyHash(sourcePath).ToString();
            var settings = source == null ? default : AnimationUtility.GetAnimationClipSettings(source);
            var builder = new StringBuilder();
            builder.Append("JADREN-CLIP|v").Append(BakeFormatVersion)
                .Append("|rig=").Append(rigCacheKey ?? string.Empty)
                .Append("|name=").Append(source == null ? string.Empty : source.name)
                .Append("|guid=").Append(sourceGuid)
                .Append("|dep=").Append(dependency)
                .Append("|rate=").Append(SampleRate.ToString("R", CultureInfo.InvariantCulture))
                .Append("|duration=").Append(source == null ? "0" : source.length.ToString("R", CultureInfo.InvariantCulture))
                .Append("|loop=").Append(settings.loopTime ? "1" : "0");
            return StableHash(builder.ToString());
        }

        private static string StableHash(string input)
        {
            return Hash128.Compute(input ?? string.Empty).ToString();
        }

        private static AnimationClip FindSourceClip(AnimationClip[] sourceClips, string sourceName)
        {
            if (sourceClips == null)
            {
                return null;
            }
            for (var i = 0; i < sourceClips.Length; i++)
            {
                if (sourceClips[i] != null && string.Equals(sourceClips[i].name, sourceName, StringComparison.Ordinal))
                {
                    return sourceClips[i];
                }
            }
            return null;
        }

        private static void ReportValidationFailure(string message)
        {
            Debug.LogError($"Jadren parity FAIL: {message}");
            EditorUtility.DisplayDialog("Jadren Animation", $"Parity FAIL: {message}", "OK");
        }

        private static float GuessSpeedThreshold(string clipName, int index)
        {
            var normalized = (clipName ?? string.Empty).ToLowerInvariant();
            if (normalized.Contains("idle") || normalized.Contains("rest"))
            {
                return 0.0f;
            }
            if (normalized.Contains("walk") || normalized.Contains("crawl"))
            {
                return 0.1f;
            }
            if (normalized.Contains("run") || normalized.Contains("sprint"))
            {
                return 1.5f;
            }
            return index == 0 ? 0.0f : index * 0.5f;
        }

        private static RigBakeData CollectRig(Transform root)
        {
            var data = new RigBakeData();
            CollectTransformRecursive(root, root, -1, data);
            var builder = new StringBuilder();
            builder.Append("JADREN-RIG|v").Append(BakeFormatVersion);
            for (var i = 0; i < data.Paths.Count; i++)
            {
                builder.Append("|name=").Append(data.Names[i])
                    .Append("|path=").Append(data.Paths[i])
                    .Append("|parent=").Append(data.Parents[i])
                    .Append("|p=").Append(FormatVector(data.Positions[i]))
                    .Append("|r=").Append(FormatQuaternion(data.Rotations[i]))
                    .Append("|s=").Append(FormatVector(data.Scales[i]));
            }
            data.CacheKey = StableHash(builder.ToString());
            return data;
        }

        private static string FormatVector(Vector3 value)
        {
            return string.Join(",", value.x.ToString("R", CultureInfo.InvariantCulture), value.y.ToString("R", CultureInfo.InvariantCulture), value.z.ToString("R", CultureInfo.InvariantCulture));
        }

        private static string FormatQuaternion(Quaternion value)
        {
            return string.Join(",", value.x.ToString("R", CultureInfo.InvariantCulture), value.y.ToString("R", CultureInfo.InvariantCulture), value.z.ToString("R", CultureInfo.InvariantCulture), value.w.ToString("R", CultureInfo.InvariantCulture));
        }

        private static void CollectTransformRecursive(Transform root, Transform current, int parentIndex, RigBakeData data)
        {
            var index = data.Paths.Count;
            data.Names.Add(current.name);
            data.Paths.Add(AnimationUtility.CalculateTransformPath(current, root));
            data.Parents.Add(parentIndex);
            data.Positions.Add(current.localPosition);
            data.Rotations.Add(current.localRotation);
            data.Scales.Add(current.localScale);

            for (var child = 0; child < current.childCount; child++)
            {
                CollectTransformRecursive(root, current.GetChild(child), index, data);
            }
        }

        private static string Sanitize(string value)
        {
            if (string.IsNullOrEmpty(value))
            {
                return "Unnamed";
            }
            var chars = value.ToCharArray();
            for (var i = 0; i < chars.Length; i++)
            {
                if (!char.IsLetterOrDigit(chars[i]) && chars[i] != '-' && chars[i] != '_')
                {
                    chars[i] = '_';
                }
            }
            return new string(chars);
        }

        private sealed class RigBakeData
        {
            public string CacheKey;
            public readonly List<string> Names = new List<string>();
            public readonly List<string> Paths = new List<string>();
            public readonly List<int> Parents = new List<int>();
            public readonly List<Vector3> Positions = new List<Vector3>();
            public readonly List<Quaternion> Rotations = new List<Quaternion>();
            public readonly List<Vector3> Scales = new List<Vector3>();
        }
    }
}
#endif
