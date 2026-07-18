using System;
using System.IO;
using UnityEditor;
using UnityEngine;
using UnityEngine.Rendering;
using Jadren.Animation;

namespace Jadren.Animation.Editor
{
    /// <summary>
    /// Runs deterministic vertex/bone-matrix skinning on the compute shader and
    /// compares completed readback with the managed reference. This is a
    /// correctness gate only; it makes no frame-time or throughput claim.
    /// </summary>
    public static class JadrenAnimationGpuSkinningBatchRunner
    {
        private const string ReportFileName = "jadren-animation-gpu-skinning-smoke.json";
        private const float Tolerance = 0.0005f;

        [Serializable]
        private sealed class Report
        {
            public string schema;
            public string status;
            public string unity_version;
            public string platform;
            public string graphics_device;
            public bool supports_compute_shaders;
            public bool completion_verified;
            public bool gpu_execution_claim;
            public string claim_scope;
            public string shader_resource;
            public int vertex_count;
            public float max_position_error;
            public float tolerance;
            public string failure_reason;
            public string report_path;
        }

        public static void Run()
        {
            var reportPath = Path.Combine(
                Directory.GetParent(Application.dataPath).FullName,
                ReportFileName);
            var capabilities = JadrenAnimationGpuCapabilities.Probe();
            var report = new Report
            {
                schema = "jadren-unity-animation-gpu-skinning-0.1",
                unity_version = Application.unityVersion,
                platform = Application.platform.ToString(),
                graphics_device = capabilities.GraphicsDevice.ToString(),
                supports_compute_shaders = capabilities.ComputeShadersSupported,
                completion_verified = false,
                gpu_execution_claim = false,
                claim_scope = "completed ComputeShader skinning parity only; no FPS, frame-time or throughput claim",
                shader_resource = "JadrenAnimationGpuSkinning",
                vertex_count = 0,
                max_position_error = 0.0f,
                tolerance = Tolerance,
                failure_reason = string.Empty,
                report_path = reportPath
            };

            if (!capabilities.ComputeShadersSupported
                || capabilities.GraphicsDevice == GraphicsDeviceType.Null)
            {
                report.status = "skip-no-gpu";
                report.failure_reason = "compute_or_graphics_device_unavailable";
                WriteReport(reportPath, report);
                Debug.Log("JADREN animation GPU skinning report=" + reportPath + " status=skip-no-gpu");
                EditorApplication.Exit(0);
                return;
            }

            try
            {
                var shader = Resources.Load<ComputeShader>(report.shader_resource);
                if (shader == null)
                {
                    throw new InvalidOperationException("GPU skinning ComputeShader resource is missing.");
                }
                var kernel = shader.FindKernel("SkinVertices");
                shader.GetKernelThreadGroupSizes(kernel, out var threadsX, out var threadsY, out var threadsZ);
                if (threadsX != 64 || threadsY != 1 || threadsZ != 1)
                {
                    throw new InvalidOperationException("GPU skinning kernel workgroup contract is invalid.");
                }

                var matrices = CreateMatrices();
                var vertices = CreateVertices();
                report.vertex_count = vertices.Length;
                var expected = new Vector3[vertices.Length];
                for (var index = 0; index < vertices.Length; index++)
                {
                    expected[index] = SkinCpu(vertices[index], matrices);
                }

                using (var dispatcher = new JadrenAnimationGpuSkinningDispatcher(shader))
                {
                    if (!dispatcher.TryDispatch(vertices, matrices, out var dispatch, out var failureReason))
                    {
                        throw new InvalidOperationException("GPU skinning dispatch was rejected: " + failureReason);
                    }
                    using (dispatch)
                    {
                        var result = dispatch.Complete();
                        if (!result.Succeeded)
                        {
                            throw new InvalidOperationException(
                                "GPU skinning readback failed: " + result.FailureReason);
                        }
                        var maxError = 0.0f;
                        for (var index = 0; index < vertices.Length; index++)
                        {
                            if (!result.TryGetPosition(index, out var actual))
                            {
                                throw new InvalidOperationException(
                                    "GPU skinning result omitted vertex " + index + ".");
                            }
                            var error = Vector3.Distance(actual, expected[index]);
                            if (float.IsNaN(error) || float.IsInfinity(error) || error > Tolerance)
                            {
                                throw new InvalidOperationException(
                                    "GPU skinning parity mismatch at vertex " + index
                                    + ": " + error + " units.");
                            }
                            maxError = Mathf.Max(maxError, error);
                        }
                        report.max_position_error = maxError;
                    }
                }

                report.status = "measured";
                report.completion_verified = true;
                report.gpu_execution_claim = true;
                WriteReport(reportPath, report);
                Debug.Log(
                    "JADREN animation GPU skinning report=" + reportPath
                    + " status=measured vertices=" + report.vertex_count
                    + " max_position_error=" + report.max_position_error.ToString("R"));
                EditorApplication.Exit(0);
            }
            catch (Exception error)
            {
                report.status = "failed";
                report.failure_reason = error.GetType().Name + ":" + error.Message;
                WriteReport(reportPath, report);
                Debug.LogException(error);
                EditorApplication.Exit(1);
            }
        }

        private static Matrix4x4[] CreateMatrices()
        {
            return new[]
            {
                Matrix4x4.identity,
                Matrix4x4.TRS(
                    new Vector3(2.0f, -1.0f, 0.5f),
                    Quaternion.Euler(0.0f, 35.0f, 0.0f),
                    Vector3.one),
                Matrix4x4.TRS(
                    new Vector3(-1.0f, 0.25f, 1.5f),
                    Quaternion.Euler(20.0f, 0.0f, -15.0f),
                    new Vector3(1.0f, 0.8f, 1.2f)),
                Matrix4x4.TRS(
                    new Vector3(0.5f, 1.25f, -2.0f),
                    Quaternion.Euler(-10.0f, 15.0f, 25.0f),
                    Vector3.one)
            };
        }

        private static JadrenGpuSkinningVertex[] CreateVertices()
        {
            return new[]
            {
                new JadrenGpuSkinningVertex(
                    new Vector3(0.0f, 0.0f, 0.0f),
                    new Vector4(1.0f, 0.0f, 0.0f, 0.0f),
                    new Vector4(0.0f, 0.0f, 0.0f, 0.0f)),
                new JadrenGpuSkinningVertex(
                    new Vector3(1.0f, 2.0f, 3.0f),
                    new Vector4(1.0f, 0.0f, 0.0f, 0.0f),
                    new Vector4(1.0f, 1.0f, 1.0f, 1.0f)),
                new JadrenGpuSkinningVertex(
                    new Vector3(-2.0f, 0.5f, 1.0f),
                    new Vector4(0.25f, 0.75f, 0.0f, 0.0f),
                    new Vector4(0.0f, 1.0f, 0.0f, 0.0f)),
                new JadrenGpuSkinningVertex(
                    new Vector3(0.25f, -1.0f, 2.0f),
                    new Vector4(0.2f, 0.3f, 0.5f, 0.0f),
                    new Vector4(0.0f, 1.0f, 2.0f, 0.0f)),
                new JadrenGpuSkinningVertex(
                    new Vector3(4.0f, -2.0f, 0.25f),
                    Vector4.zero,
                    new Vector4(0.0f, 1.0f, 2.0f, 3.0f)),
                new JadrenGpuSkinningVertex(
                    new Vector3(-0.75f, 1.5f, -2.5f),
                    new Vector4(0.1f, 0.2f, 0.3f, 0.4f),
                    new Vector4(0.0f, 1.0f, 2.0f, 3.0f))
            };
        }

        private static Vector3 SkinCpu(JadrenGpuSkinningVertex vertex, Matrix4x4[] matrices)
        {
            var weights = vertex.BoneWeights;
            var indices = vertex.BoneIndices;
            var weightSum = weights.x + weights.y + weights.z + weights.w;
            if (weightSum <= 0.000001f)
            {
                return vertex.Position;
            }
            var result = Vector3.zero;
            result += matrices[Mathf.RoundToInt(indices.x)].MultiplyPoint3x4(vertex.Position) * weights.x;
            result += matrices[Mathf.RoundToInt(indices.y)].MultiplyPoint3x4(vertex.Position) * weights.y;
            result += matrices[Mathf.RoundToInt(indices.z)].MultiplyPoint3x4(vertex.Position) * weights.z;
            result += matrices[Mathf.RoundToInt(indices.w)].MultiplyPoint3x4(vertex.Position) * weights.w;
            return result / weightSum;
        }

        private static void WriteReport(string reportPath, Report report)
        {
            File.WriteAllText(reportPath, JsonUtility.ToJson(report, true));
        }
    }
}
