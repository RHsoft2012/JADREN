using System;
using System.IO;
using Jadren.Animation;
using UnityEditor;
using UnityEditor.SceneManagement;
using UnityEngine;

namespace Jadren.Animation.Editor
{
    /// <summary>
    /// Reproducible matrix parity gate for the packed clip -> GPU sampling ->
    /// hierarchy -> skin-matrix path. It compares the GPU buffer with the CPU
    /// fallback built from the same rig, clip, bindposes, agent grid and time.
    /// </summary>
    public static class JadrenAnimationGpuPackedParityBatchRunner
    {
        private const string DefaultCharacterPrefab =
            "Assets/ZombieFemaleAnimations/Prefabs/FemaleZombie.prefab";
        private const string ReportEnvironmentVariable = "JADREN_GPU_PACKED_PARITY_REPORT";
        private const float Tolerance = 0.00001f;

        [Serializable]
        private sealed class Report
        {
            public string schema;
            public string status;
            public string unity_version;
            public string platform;
            public string character_prefab;
            public int agent_count;
            public int mesh_bone_count;
            public int matrix_count;
            public float tolerance;
            public float max_abs_error;
            public double rms_error;
            public int worst_matrix_index;
            public int worst_element;
            public bool gpu_animation_active;
            public string failure_reason;
            public string claim_scope;
        }

        public static void Run()
        {
            var reportPath = ResolveReportPath();
            var prefabPath = Environment.GetEnvironmentVariable("JADREN_CHARACTER_PREFAB");
            if (string.IsNullOrWhiteSpace(prefabPath))
            {
                prefabPath = DefaultCharacterPrefab;
            }
            var report = new Report
            {
                schema = "jadren-unity-animation-gpu-packed-parity-0.1",
                status = "failed",
                unity_version = Application.unityVersion,
                platform = Application.platform.ToString(),
                character_prefab = prefabPath,
                agent_count = 4,
                tolerance = Tolerance,
                failure_reason = string.Empty,
                claim_scope = "local D3D11 packed clip GPU skin-matrix parity against the managed fallback at time zero; no FPS, cross-device, Rukhanka or public speedup claim"
            };

            GameObject host = null;
            try
            {
                EditorSceneManager.NewScene(NewSceneSetup.EmptyScene, NewSceneMode.Single);
                var prefab = AssetDatabase.LoadAssetAtPath<GameObject>(prefabPath);
                if (prefab == null)
                {
                    throw new InvalidOperationException("character_prefab_missing:" + prefabPath);
                }

                host = new GameObject("Jadren GPU Packed Parity");
                var crowd = host.AddComponent<JadrenAnimationGpuCrowdAnimator>();
                crowd.AutoBuild = false;
                crowd.CharacterPrefab = prefab;
                crowd.AgentCount = report.agent_count;
                crowd.AgentColumns = 2;
                crowd.AgentSpacing = 2.0f;
                crowd.UniformSpeed = 0.0f;
                crowd.Animate = true;
                crowd.PreferGpuAnimation = true;
                crowd.PreferNativePoseTiles = false;
                if (!crowd.TryBuild(out var buildFailure))
                {
                    throw new InvalidOperationException("crowd_build_failed:" + buildFailure);
                }
                report.mesh_bone_count = crowd.MeshBoneCount;
                if (!crowd.TryCopyCpuFallbackBoneMatrices(out var managed, out var managedFailure))
                {
                    throw new InvalidOperationException("cpu_reference_failed:" + managedFailure);
                }
                if (!crowd.TryStep(0.0f))
                {
                    throw new InvalidOperationException("gpu_step_failed:" + crowd.FailureReason);
                }
                report.gpu_animation_active = crowd.UsesGpuAnimation;
                if (!report.gpu_animation_active)
                {
                    throw new InvalidOperationException("gpu_animation_not_active:" + crowd.FailureReason);
                }
                if (!crowd.TryReadbackGpuBoneMatrices(out var gpu, out var gpuFailure))
                {
                    throw new InvalidOperationException("gpu_readback_failed:" + gpuFailure);
                }
                if (gpu.Length != managed.Length || gpu.Length == 0)
                {
                    throw new InvalidOperationException(
                        "matrix_count_mismatch:gpu=" + gpu.Length + ",managed=" + managed.Length);
                }

                double squaredError = 0.0;
                var valueCount = 0;
                var maxError = 0.0f;
                var worstMatrix = -1;
                var worstElement = -1;
                for (var matrix = 0; matrix < gpu.Length; matrix++)
                {
                    for (var element = 0; element < 16; element++)
                    {
                        var error = Mathf.Abs(gpu[matrix][element] - managed[matrix][element]);
                        squaredError += (double)error * error;
                        valueCount++;
                        if (error > maxError)
                        {
                            maxError = error;
                            worstMatrix = matrix;
                            worstElement = element;
                        }
                    }
                }

                report.matrix_count = gpu.Length;
                report.max_abs_error = maxError;
                report.rms_error = valueCount == 0 ? 0.0 : Math.Sqrt(squaredError / valueCount);
                report.worst_matrix_index = worstMatrix;
                report.worst_element = worstElement;
                if (maxError > Tolerance)
                {
                    throw new InvalidOperationException("matrix_parity_tolerance_exceeded:" + maxError);
                }
                report.status = "measured-pass";
                WriteReport(reportPath, report);
                EditorApplication.Exit(0);
            }
            catch (Exception error)
            {
                report.failure_reason = error.GetType().Name + ":" + error.Message;
                WriteReport(reportPath, report);
                Debug.LogException(error);
                EditorApplication.Exit(1);
            }
            finally
            {
                if (host != null)
                {
                    UnityEngine.Object.DestroyImmediate(host);
                }
            }
        }

        private static string ResolveReportPath()
        {
            var value = Environment.GetEnvironmentVariable(ReportEnvironmentVariable);
            return Path.GetFullPath(string.IsNullOrWhiteSpace(value)
                ? "jadren-animation-gpu-packed-parity.json"
                : value);
        }

        private static void WriteReport(string path, Report report)
        {
            var directory = Path.GetDirectoryName(path);
            if (!string.IsNullOrEmpty(directory))
            {
                Directory.CreateDirectory(directory);
            }
            File.WriteAllText(path, JsonUtility.ToJson(report, true));
        }
    }
}
