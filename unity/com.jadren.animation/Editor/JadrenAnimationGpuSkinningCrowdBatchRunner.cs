using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using Jadren.Animation;
using UnityEditor;
using UnityEngine;
using UnityEngine.Rendering;
using Stopwatch = System.Diagnostics.Stopwatch;
using UnityDebug = UnityEngine.Debug;

namespace Jadren.Animation.Editor
{
    /// <summary>
    /// Measures completed GPU deformation for a single batched crowd stream.
    /// Each synthetic agent owns three vertices and one translated bone. The
    /// runner validates every returned position, so this is a completion and
    /// scaling gate rather than a command-submission-only smoke.
    /// </summary>
    public static class JadrenAnimationGpuSkinningCrowdBatchRunner
    {
        private const string ReportFileName = "jadren-animation-gpu-skinning-crowd-smoke.json";
        private const string ShaderResource = "JadrenAnimationGpuSkinning";
        private const int VerticesPerAgent = 3;
        private const int WarmupCount = 2;
        private const int MeasurementCount = 5;
        private const float PositionTolerance = 0.0005f;

        [Serializable]
        private sealed class WorkloadReport
        {
            public int agent_count;
            public int vertex_count;
            public int warmup_count;
            public int sample_count;
            public double median_ms;
            public double max_ms;
            public float max_position_error;
            public int buffer_allocation_count;
            public bool completion_verified;
        }

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
            public int workload_count;
            public int measurement_count;
            public int vertices_per_agent;
            public float position_tolerance;
            public WorkloadReport[] workloads;
            public string shader_resource;
            public string failure_reason;
            public string claim_scope;
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
                schema = "jadren-unity-animation-gpu-skinning-crowd-0.1",
                unity_version = Application.unityVersion,
                platform = Application.platform.ToString(),
                graphics_device = capabilities.GraphicsDevice.ToString(),
                supports_compute_shaders = capabilities.ComputeShadersSupported,
                completion_verified = false,
                gpu_execution_claim = false,
                workload_count = 0,
                measurement_count = MeasurementCount,
                vertices_per_agent = VerticesPerAgent,
                position_tolerance = PositionTolerance,
                workloads = Array.Empty<WorkloadReport>(),
                shader_resource = ShaderResource,
                failure_reason = string.Empty,
                claim_scope = "local completed GPU skinning crowd batch timing only; no FPS, frame-time, throughput or cross-device claim",
                report_path = reportPath
            };

            if (!capabilities.ComputeShadersSupported
                || capabilities.GraphicsDevice == GraphicsDeviceType.Null)
            {
                report.status = "skip-no-gpu";
                report.failure_reason = "compute_or_graphics_device_unavailable";
                WriteReport(reportPath, report);
                UnityDebug.Log("JADREN GPU skinning crowd report=" + reportPath + " status=skip-no-gpu");
                EditorApplication.Exit(0);
                return;
            }

            JadrenAnimationGpuSkinningGraphicsStream stream = null;
            try
            {
                var shader = Resources.Load<ComputeShader>(ShaderResource);
                if (shader == null)
                {
                    throw new InvalidOperationException("GPU skinning ComputeShader resource is missing.");
                }
                stream = new JadrenAnimationGpuSkinningGraphicsStream(shader);
                var propertyBlock = new MaterialPropertyBlock();
                var workloads = new List<WorkloadReport>();
                foreach (var agentCount in new[] { 100, 1000, 10000 })
                {
                    var vertices = CreateVertices(agentCount);
                    var matrices = CreateMatrices(agentCount);
                    for (var warmup = 0; warmup < WarmupCount; warmup++)
                    {
                        RunCompletedSample(stream, propertyBlock, vertices, matrices, out var warmupFailure);
                        if (warmupFailure.Length > 0)
                        {
                            throw new InvalidOperationException(warmupFailure);
                        }
                    }

                    var timings = new double[MeasurementCount];
                    var maxError = 0.0f;
                    for (var sample = 0; sample < MeasurementCount; sample++)
                    {
                        var stopwatch = Stopwatch.StartNew();
                        var sampleError = RunCompletedSample(
                            stream,
                            propertyBlock,
                            vertices,
                            matrices,
                            out var failureReason);
                        stopwatch.Stop();
                        if (failureReason.Length > 0)
                        {
                            throw new InvalidOperationException(failureReason);
                        }
                        timings[sample] = stopwatch.Elapsed.TotalMilliseconds;
                        maxError = Mathf.Max(maxError, sampleError);
                    }

                    Array.Sort(timings);
                    var workload = new WorkloadReport
                    {
                        agent_count = agentCount,
                        vertex_count = vertices.Length,
                        warmup_count = WarmupCount,
                        sample_count = MeasurementCount,
                        median_ms = timings[timings.Length / 2],
                        max_ms = timings[timings.Length - 1],
                        max_position_error = maxError,
                        buffer_allocation_count = stream.BufferAllocationCount,
                        completion_verified = maxError <= PositionTolerance
                            && timings[timings.Length / 2] > 0.0
                    };
                    if (!workload.completion_verified)
                    {
                        throw new InvalidOperationException(
                            "GPU crowd completion workload failed: agents=" + agentCount
                            + ", error=" + maxError + ", median_ms=" + workload.median_ms);
                    }
                    workloads.Add(workload);
                }

                report.workloads = workloads.ToArray();
                report.workload_count = report.workloads.Length;
                report.completion_verified = report.workload_count == 3
                    && report.workloads.All(workload => workload.completion_verified);
                report.status = "measured";
                report.gpu_execution_claim = report.completion_verified;
                WriteReport(reportPath, report);
                UnityDebug.Log(
                    "JADREN GPU skinning crowd report=" + reportPath
                    + " status=measured workloads=" + report.workload_count);
                EditorApplication.Exit(report.completion_verified ? 0 : 1);
            }
            catch (Exception error)
            {
                report.status = "failed";
                report.failure_reason = error.GetType().Name + ":" + error.Message;
                WriteReport(reportPath, report);
                UnityDebug.LogException(error);
                EditorApplication.Exit(1);
            }
            finally
            {
                if (stream != null)
                {
                    stream.Dispose();
                }
            }
        }

        private static float RunCompletedSample(
            JadrenAnimationGpuSkinningGraphicsStream stream,
            MaterialPropertyBlock propertyBlock,
            JadrenGpuSkinningVertex[] vertices,
            Matrix4x4[] matrices,
            out string failureReason)
        {
            if (!stream.TryDispatchAndBind(vertices, matrices, propertyBlock, out failureReason))
            {
                return float.PositiveInfinity;
            }
            if (!stream.TryRequestReadback(out var request, out failureReason))
            {
                return float.PositiveInfinity;
            }
            request.WaitForCompletion();
            if (request.hasError)
            {
                failureReason = "unity_compute_readback_error";
                return float.PositiveInfinity;
            }

            var data = request.GetData<Vector3>();
            if (data.Length < vertices.Length)
            {
                failureReason = "gpu_readback_count_invalid:" + data.Length;
                return float.PositiveInfinity;
            }
            var maxError = 0.0f;
            for (var index = 0; index < vertices.Length; index++)
            {
                var agent = index / VerticesPerAgent;
                var expected = vertices[index].Position
                    + AgentTranslation(agent);
                var error = Vector3.Distance(expected, data[index]);
                if (float.IsNaN(error) || float.IsInfinity(error))
                {
                    failureReason = "gpu_readback_nonfinite_error:" + index;
                    return float.PositiveInfinity;
                }
                maxError = Mathf.Max(maxError, error);
            }
            failureReason = string.Empty;
            return maxError;
        }

        private static JadrenGpuSkinningVertex[] CreateVertices(int agentCount)
        {
            var vertices = new JadrenGpuSkinningVertex[agentCount * VerticesPerAgent];
            for (var agent = 0; agent < agentCount; agent++)
            {
                var boneIndex = new Vector4(agent, agent, agent, agent);
                var offset = agent * VerticesPerAgent;
                vertices[offset] = new JadrenGpuSkinningVertex(
                    new Vector3(0.0f, 0.0f, 0.0f),
                    new Vector4(1.0f, 0.0f, 0.0f, 0.0f),
                    boneIndex);
                vertices[offset + 1] = new JadrenGpuSkinningVertex(
                    new Vector3(0.5f, 0.0f, 0.0f),
                    new Vector4(1.0f, 0.0f, 0.0f, 0.0f),
                    boneIndex);
                vertices[offset + 2] = new JadrenGpuSkinningVertex(
                    new Vector3(0.0f, 0.5f, 0.0f),
                    new Vector4(1.0f, 0.0f, 0.0f, 0.0f),
                    boneIndex);
            }
            return vertices;
        }

        private static Matrix4x4[] CreateMatrices(int agentCount)
        {
            var matrices = new Matrix4x4[agentCount];
            for (var agent = 0; agent < agentCount; agent++)
            {
                matrices[agent] = Matrix4x4.Translate(AgentTranslation(agent));
            }
            return matrices;
        }

        private static Vector3 AgentTranslation(int agent)
        {
            return new Vector3(
                (agent % 100) * 0.01f,
                (agent / 100) * 0.01f,
                (agent % 17) * 0.0025f);
        }

        private static void WriteReport(string reportPath, Report report)
        {
            File.WriteAllText(reportPath, JsonUtility.ToJson(report, true));
        }
    }
}
