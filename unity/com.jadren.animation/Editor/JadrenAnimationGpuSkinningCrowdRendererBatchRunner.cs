using System;
using System.IO;
using Jadren.Animation;
using UnityEditor;
using UnityEngine;
using UnityEngine.Rendering;

namespace Jadren.Animation.Editor
{
    /// <summary>
    /// Graphics smoke for the procedural crowd renderer. It submits one
    /// DrawMeshInstancedProcedural call, checks visible pixels and then waits
    /// for a readback of the same flat output buffer.
    /// </summary>
    public static class JadrenAnimationGpuSkinningCrowdRendererBatchRunner
    {
        private const string ReportFileName = "jadren-animation-gpu-skinning-crowd-renderer-smoke.json";
        private const int AgentCount = 100;
        private const int VerticesPerAgent = 3;
        private const float PositionTolerance = 0.0005f;

        [Serializable]
        private sealed class Report
        {
            public string schema;
            public string status;
            public string unity_version;
            public string platform;
            public string graphics_device;
            public bool supports_compute_shaders;
            public bool renderer_binding_verified;
            public bool completion_verified;
            public bool gpu_execution_claim;
            public int agent_count;
            public int vertices_per_agent;
            public int draw_submission_count;
            public int pixel_count;
            public int minimum_pixel_count;
            public float max_position_error;
            public float position_tolerance;
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
                schema = "jadren-unity-animation-gpu-skinning-crowd-renderer-0.1",
                unity_version = Application.unityVersion,
                platform = Application.platform.ToString(),
                graphics_device = capabilities.GraphicsDevice.ToString(),
                supports_compute_shaders = capabilities.ComputeShadersSupported,
                renderer_binding_verified = false,
                completion_verified = false,
                gpu_execution_claim = false,
                agent_count = AgentCount,
                vertices_per_agent = VerticesPerAgent,
                draw_submission_count = 0,
                pixel_count = 0,
                minimum_pixel_count = 16,
                max_position_error = 0.0f,
                position_tolerance = PositionTolerance,
                failure_reason = string.Empty,
                claim_scope = "one procedural GPU crowd draw plus completed output readback; no FPS, frame-time, throughput or cross-device claim",
                report_path = reportPath
            };

            if (!capabilities.ComputeShadersSupported
                || capabilities.GraphicsDevice == GraphicsDeviceType.Null)
            {
                report.status = "skip-no-gpu";
                report.failure_reason = "compute_or_graphics_device_unavailable";
                WriteReport(reportPath, report);
                Debug.Log("JADREN GPU crowd renderer report=" + reportPath + " status=skip-no-gpu");
                EditorApplication.Exit(0);
                return;
            }

            GameObject hostObject = null;
            GameObject cameraObject = null;
            Mesh mesh = null;
            RenderTexture renderTexture = null;
            Texture2D readbackTexture = null;
            JadrenGpuSkinningCrowdRenderer host = null;
            try
            {
                var computeShader = Resources.Load<ComputeShader>("JadrenAnimationGpuSkinning");
                if (computeShader == null)
                {
                    throw new InvalidOperationException("GPU skinning ComputeShader resource is missing.");
                }
                var vertices = CreateVertices();
                var matrices = CreateMatrices();
                mesh = CreateSourceMesh();
                hostObject = new GameObject("JadrenGpuSkinningCrowdRendererSmoke");
                host = hostObject.AddComponent<JadrenGpuSkinningCrowdRenderer>();
                host.SetSkinningShader(computeShader);
                host.SetDrawBounds(new Bounds(Vector3.zero, Vector3.one * 20.0f));
                if (!host.TrySetCrowdData(mesh, null, vertices, matrices, out var configurationFailure))
                {
                    throw new InvalidOperationException("Crowd renderer configuration failed: " + configurationFailure);
                }
                host.SetGpuSkinningEnabled(true);
                if (!host.TryRenderFrame(out var frameFailure))
                {
                    throw new InvalidOperationException("Crowd renderer frame failed: " + frameFailure);
                }

                cameraObject = new GameObject("JadrenGpuSkinningCrowdRendererSmokeCamera");
                var camera = cameraObject.AddComponent<Camera>();
                camera.clearFlags = CameraClearFlags.SolidColor;
                camera.backgroundColor = Color.black;
                camera.orthographic = true;
                camera.orthographicSize = 4.2f;
                camera.aspect = 1.0f;
                camera.nearClipPlane = 0.1f;
                camera.farClipPlane = 100.0f;
                camera.transform.position = new Vector3(0.0f, 0.0f, -10.0f);
                camera.transform.rotation = Quaternion.identity;
                renderTexture = new RenderTexture(256, 256, 24, RenderTextureFormat.ARGB32)
                {
                    name = "JadrenGpuSkinningCrowdRendererSmokeTarget"
                };
                renderTexture.Create();
                camera.targetTexture = renderTexture;
                camera.Render();

                var previousActive = RenderTexture.active;
                try
                {
                    RenderTexture.active = renderTexture;
                    readbackTexture = new Texture2D(256, 256, TextureFormat.RGBA32, false);
                    readbackTexture.ReadPixels(new Rect(0.0f, 0.0f, 256.0f, 256.0f), 0, 0);
                    readbackTexture.Apply();
                }
                finally
                {
                    RenderTexture.active = previousActive;
                }
                report.pixel_count = CountLitPixels(readbackTexture);
                report.draw_submission_count = host.DrawSubmissionCount;
                if (report.pixel_count < report.minimum_pixel_count
                    || report.draw_submission_count != 1)
                {
                    throw new InvalidOperationException(
                        "Procedural crowd renderer did not produce the expected draw: pixels="
                        + report.pixel_count + ", draws=" + report.draw_submission_count);
                }
                report.renderer_binding_verified = true;

                if (!host.TryRequestReadback(out var request, out var readbackFailure))
                {
                    throw new InvalidOperationException("Crowd readback request failed: " + readbackFailure);
                }
                request.WaitForCompletion();
                if (request.hasError)
                {
                    throw new InvalidOperationException("Crowd renderer readback reported an error.");
                }
                var data = request.GetData<Vector3>();
                if (data.Length < vertices.Length)
                {
                    throw new InvalidOperationException("Crowd renderer readback count is invalid: " + data.Length);
                }
                for (var index = 0; index < vertices.Length; index++)
                {
                    var expected = vertices[index].Position
                        + AgentTranslation(index / VerticesPerAgent);
                    var error = Vector3.Distance(expected, data[index]);
                    report.max_position_error = Mathf.Max(report.max_position_error, error);
                }
                if (report.max_position_error > PositionTolerance)
                {
                    throw new InvalidOperationException(
                        "Crowd renderer readback parity failed: " + report.max_position_error);
                }
                report.completion_verified = true;
                report.gpu_execution_claim = true;
                report.status = "measured";
                WriteReport(reportPath, report);
                Debug.Log(
                    "JADREN GPU crowd renderer report=" + reportPath
                    + " status=measured pixels=" + report.pixel_count);
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
            finally
            {
                if (host != null) host.Dispose();
                if (readbackTexture != null) UnityEngine.Object.DestroyImmediate(readbackTexture);
                if (renderTexture != null)
                {
                    renderTexture.Release();
                    UnityEngine.Object.DestroyImmediate(renderTexture);
                }
                if (cameraObject != null) UnityEngine.Object.DestroyImmediate(cameraObject);
                if (hostObject != null) UnityEngine.Object.DestroyImmediate(hostObject);
                if (mesh != null) UnityEngine.Object.DestroyImmediate(mesh);
            }
        }

        private static JadrenGpuSkinningVertex[] CreateVertices()
        {
            var vertices = new JadrenGpuSkinningVertex[AgentCount * VerticesPerAgent];
            for (var agent = 0; agent < AgentCount; agent++)
            {
                var offset = agent * VerticesPerAgent;
                var index = new Vector4(agent, agent, agent, agent);
                var weights = new Vector4(1.0f, 0.0f, 0.0f, 0.0f);
                vertices[offset] = new JadrenGpuSkinningVertex(new Vector3(-0.25f, -0.25f, 0.0f), weights, index);
                vertices[offset + 1] = new JadrenGpuSkinningVertex(new Vector3(0.25f, -0.25f, 0.0f), weights, index);
                vertices[offset + 2] = new JadrenGpuSkinningVertex(new Vector3(0.0f, 0.25f, 0.0f), weights, index);
            }
            return vertices;
        }

        private static Matrix4x4[] CreateMatrices()
        {
            var matrices = new Matrix4x4[AgentCount];
            for (var agent = 0; agent < AgentCount; agent++)
            {
                matrices[agent] = Matrix4x4.Translate(AgentTranslation(agent));
            }
            return matrices;
        }

        private static Vector3 AgentTranslation(int agent)
        {
            return new Vector3(
                (agent % 10 - 4.5f) * 0.75f,
                (agent / 10 - 4.5f) * 0.75f,
                0.0f);
        }

        private static Mesh CreateSourceMesh()
        {
            var mesh = new Mesh { name = "JadrenGpuSkinningCrowdRendererSmokeMesh" };
            mesh.vertices = new[]
            {
                new Vector3(-0.25f, -0.25f, 0.0f),
                new Vector3(0.25f, -0.25f, 0.0f),
                new Vector3(0.0f, 0.25f, 0.0f)
            };
            mesh.normals = new[] { Vector3.forward, Vector3.forward, Vector3.forward };
            mesh.tangents = new[]
            {
                new Vector4(1.0f, 0.0f, 0.0f, 1.0f),
                new Vector4(1.0f, 0.0f, 0.0f, 1.0f),
                new Vector4(1.0f, 0.0f, 0.0f, 1.0f)
            };
            mesh.uv = new[] { Vector2.zero, Vector2.right, Vector2.up };
            mesh.triangles = new[] { 0, 1, 2 };
            mesh.bounds = new Bounds(Vector3.zero, Vector3.one * 20.0f);
            return mesh;
        }

        private static int CountLitPixels(Texture2D texture)
        {
            var pixels = texture.GetPixels32();
            var count = 0;
            for (var index = 0; index < pixels.Length; index++)
            {
                var pixel = pixels[index];
                if (pixel.r > 32 && pixel.g > 32 && pixel.b > 32 && pixel.a > 32)
                {
                    count++;
                }
            }
            return count;
        }

        private static void WriteReport(string reportPath, Report report)
        {
            File.WriteAllText(reportPath, JsonUtility.ToJson(report, true));
        }
    }
}
