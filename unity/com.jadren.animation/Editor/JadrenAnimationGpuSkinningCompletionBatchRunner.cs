using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using Jadren.Animation;
using UnityEditor;
using UnityEngine;
using UnityEngine.Rendering;
using Stopwatch = System.Diagnostics.Stopwatch;
using UnityDebug = UnityEngine.Debug;

namespace Jadren.Animation.Editor
{
    /// <summary>
    /// Measures completed compute-to-readback timing for the reusable GPU
    /// skinning stream. This is deliberately separate from FPS/rendering: it
    /// waits for each AsyncGPUReadback and validates deterministic positions.
    /// </summary>
    public static class JadrenAnimationGpuSkinningCompletionBatchRunner
    {
        private const string ReportFileName = "jadren-animation-gpu-skinning-completion-smoke.json";
        private const int WarmupCount = 2;
        private const int MeasurementCount = 5;
        private const float PositionTolerance = 0.0005f;

        [Serializable]
        private sealed class WorkloadReport
        {
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
            public string graphics_device;
            public bool supports_compute_shaders;
            public bool completion_verified;
            public bool gpu_execution_claim;
            public int workload_count;
            public int measurement_count;
            public float position_tolerance;
            public WorkloadReport[] workloads;
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
                schema = "jadren-unity-animation-gpu-skinning-completion-0.1",
                unity_version = Application.unityVersion,
                graphics_device = capabilities.GraphicsDevice.ToString(),
                supports_compute_shaders = capabilities.ComputeShadersSupported,
                completion_verified = false,
                gpu_execution_claim = false,
                workload_count = 0,
                measurement_count = MeasurementCount,
                position_tolerance = PositionTolerance,
                workloads = Array.Empty<WorkloadReport>(),
                failure_reason = string.Empty,
                claim_scope = "local completed compute-to-readback timing only; no FPS, frame-time, throughput or cross-device claim",
                report_path = reportPath
            };

            if (!capabilities.ComputeShadersSupported
                || capabilities.GraphicsDevice == GraphicsDeviceType.Null)
            {
                report.status = "skip-no-gpu";
                report.failure_reason = "compute_or_graphics_device_unavailable";
                WriteReport(reportPath, report);
                UnityDebug.Log("JADREN GPU skinning completion report=" + reportPath + " status=skip-no-gpu");
                EditorApplication.Exit(0);
                return;
            }

            JadrenAnimationGpuSkinningGraphicsStream stream = null;
            try
            {
                var shader = Resources.Load<ComputeShader>("JadrenAnimationGpuSkinning");
                if (shader == null)
                {
                    throw new InvalidOperationException("GPU skinning ComputeShader resource is missing.");
                }
                stream = new JadrenAnimationGpuSkinningGraphicsStream(shader);
                var propertyBlock = new MaterialPropertyBlock();
                var workloads = new List<WorkloadReport>();
                foreach (var vertexCount in new[] { 1024, 4096, 16384 })
                {
                    var vertices = CreateVertices(vertexCount);
                    var matrices = new[] { Matrix4x4.Translate(new Vector3(0.25f, 0.5f, -0.75f)) };
                    for (var warmup = 0; warmup < WarmupCount; warmup++)
                    {
                        RunCompletedSample(stream, propertyBlock, vertices, matrices, out _);
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
                        vertex_count = vertexCount,
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
                            "GPU completion workload failed: vertices=" + vertexCount
                            + ", error=" + maxError + ", median_ms=" + workload.median_ms);
                    }
                    workloads.Add(workload);
                }

                report.workloads = workloads.ToArray();
                report.workload_count = report.workloads.Length;
                report.completion_verified = report.workload_count == 3;
                for (var index = 0; index < report.workloads.Length; index++)
                {
                    report.completion_verified &= report.workloads[index].completion_verified;
                }
                report.status = "measured";
                report.gpu_execution_claim = report.completion_verified;
                WriteReport(reportPath, report);
                UnityDebug.Log(
                    "JADREN GPU skinning completion report=" + reportPath
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
            var expectedTranslation = new Vector3(0.25f, 0.5f, -0.75f);
            var maxError = 0.0f;
            for (var index = 0; index < vertices.Length; index++)
            {
                var expected = vertices[index].Position + expectedTranslation;
                var actual = data[index];
                maxError = Mathf.Max(maxError, Vector3.Distance(expected, actual));
            }
            failureReason = string.Empty;
            return maxError;
        }

        private static JadrenGpuSkinningVertex[] CreateVertices(int count)
        {
            var vertices = new JadrenGpuSkinningVertex[count];
            for (var index = 0; index < count; index++)
            {
                var x = (index % 128) * 0.01f;
                var y = ((index / 128) % 128) * 0.01f;
                var z = (index / (128 * 128)) * 0.01f;
                vertices[index] = new JadrenGpuSkinningVertex(
                    new Vector3(x, y, z),
                    new Vector4(1.0f, 0.0f, 0.0f, 0.0f),
                    Vector4.zero);
            }
            return vertices;
        }

        private static void WriteReport(string reportPath, Report report)
        {
            File.WriteAllText(reportPath, JsonUtility.ToJson(report, true));
        }
    }
}
