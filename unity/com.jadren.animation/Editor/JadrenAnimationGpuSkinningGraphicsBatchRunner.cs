using System;
using System.IO;
using UnityEditor;
using UnityEngine;
using UnityEngine.Rendering;
using Jadren.Animation;

namespace Jadren.Animation.Editor
{
    /// <summary>
    /// Verifies that a live GPU skinning output buffer can be consumed by a
    /// renderer-side vertex shader. The source mesh is intentionally placed
    /// off-camera; only the bound compute output can produce visible pixels.
    /// </summary>
    public static class JadrenAnimationGpuSkinningGraphicsBatchRunner
    {
        private const string ReportFileName = "jadren-animation-gpu-skinning-graphics-smoke.json";

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
            public bool renderer_binding_verified;
            public bool gpu_execution_claim;
            public string claim_scope;
            public int pixel_count;
            public int minimum_pixel_count;
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
                schema = "jadren-unity-animation-gpu-skinning-graphics-0.1",
                unity_version = Application.unityVersion,
                platform = Application.platform.ToString(),
                graphics_device = capabilities.GraphicsDevice.ToString(),
                supports_compute_shaders = capabilities.ComputeShadersSupported,
                completion_verified = false,
                renderer_binding_verified = false,
                gpu_execution_claim = false,
                claim_scope = "completed compute-to-renderer buffer binding smoke only; no FPS, frame-time or throughput claim",
                pixel_count = 0,
                minimum_pixel_count = 4,
                failure_reason = string.Empty,
                report_path = reportPath
            };

            if (!capabilities.ComputeShadersSupported
                || capabilities.GraphicsDevice == GraphicsDeviceType.Null)
            {
                report.status = "skip-no-gpu";
                report.failure_reason = "compute_or_graphics_device_unavailable";
                WriteReport(reportPath, report);
                Debug.Log("JADREN animation GPU skinning graphics report=" + reportPath + " status=skip-no-gpu");
                EditorApplication.Exit(0);
                return;
            }

            GameObject cameraObject = null;
            GameObject rendererObject = null;
            Mesh mesh = null;
            Material material = null;
            RenderTexture renderTexture = null;
            Texture2D readbackTexture = null;
            try
            {
                var computeShader = Resources.Load<ComputeShader>("JadrenAnimationGpuSkinning");
                if (computeShader == null)
                {
                    throw new InvalidOperationException("GPU skinning ComputeShader resource is missing.");
                }
                var previewShader = Shader.Find("Jadren/Animation/GpuPositionPreview");
                if (previewShader == null)
                {
                    throw new InvalidOperationException("GPU position preview shader is missing.");
                }

                var vertices = new[]
                {
                    new JadrenGpuSkinningVertex(
                        new Vector3(10.0f, 10.0f, 0.0f),
                        new Vector4(1.0f, 0.0f, 0.0f, 0.0f),
                        Vector4.zero),
                    new JadrenGpuSkinningVertex(
                        new Vector3(11.0f, 10.0f, 0.0f),
                        new Vector4(1.0f, 0.0f, 0.0f, 0.0f),
                        Vector4.zero),
                    new JadrenGpuSkinningVertex(
                        new Vector3(10.0f, 11.0f, 0.0f),
                        new Vector4(1.0f, 0.0f, 0.0f, 0.0f),
                        Vector4.zero)
                };
                var matrices = new[]
                {
                    Matrix4x4.Translate(new Vector3(-10.0f, -10.0f, 0.0f))
                };

                using (var dispatcher = new JadrenAnimationGpuSkinningDispatcher(computeShader))
                {
                    if (!dispatcher.TryDispatchToGraphics(
                            vertices,
                            matrices,
                            out var dispatch,
                            out var failureReason))
                    {
                        throw new InvalidOperationException("GPU graphics dispatch was rejected: " + failureReason);
                    }
                    try
                    {
                        mesh = new Mesh
                        {
                            name = "JadrenGpuSkinningGraphicsSmokeMesh",
                            vertices = new[]
                            {
                                vertices[0].Position,
                                vertices[1].Position,
                                vertices[2].Position
                            },
                            triangles = new[] { 0, 1, 2 }
                        };
                        mesh.RecalculateBounds();
                        // The source vertices are deliberately off-camera;
                        // renderer culling must explicitly cover the GPU
                        // output positions as required by the capability gate.
                        mesh.bounds = new Bounds(Vector3.zero, Vector3.one * 100.0f);
                        material = new Material(previewShader)
                        {
                            name = "JadrenGpuSkinningGraphicsSmokeMaterial"
                        };
                        material.SetColor("_BaseColor", Color.white);
                        rendererObject = new GameObject("JadrenGpuSkinningGraphicsSmokeRenderer");
                        var filter = rendererObject.AddComponent<MeshFilter>();
                        filter.sharedMesh = mesh;
                        var meshRenderer = rendererObject.AddComponent<MeshRenderer>();
                        meshRenderer.sharedMaterial = material;
                        var propertyBlock = new MaterialPropertyBlock();
                        if (!dispatch.BindOutput(propertyBlock))
                        {
                            throw new InvalidOperationException("GPU output buffer binding was rejected.");
                        }
                        meshRenderer.SetPropertyBlock(propertyBlock);

                        cameraObject = new GameObject("JadrenGpuSkinningGraphicsSmokeCamera");
                        var camera = cameraObject.AddComponent<Camera>();
                        camera.clearFlags = CameraClearFlags.SolidColor;
                        camera.backgroundColor = Color.black;
                        camera.orthographic = true;
                        camera.orthographicSize = 3.0f;
                        camera.aspect = 1.0f;
                        camera.nearClipPlane = 0.1f;
                        camera.farClipPlane = 100.0f;
                        camera.transform.position = new Vector3(0.0f, 0.0f, -5.0f);
                        camera.transform.rotation = Quaternion.identity;
                        renderTexture = new RenderTexture(64, 64, 24, RenderTextureFormat.ARGB32)
                        {
                            name = "JadrenGpuSkinningGraphicsSmokeTarget"
                        };
                        renderTexture.Create();
                        camera.targetTexture = renderTexture;
                        camera.Render();

                        var previousActive = RenderTexture.active;
                        try
                        {
                            RenderTexture.active = renderTexture;
                            readbackTexture = new Texture2D(64, 64, TextureFormat.RGBA32, false);
                            readbackTexture.ReadPixels(new Rect(0.0f, 0.0f, 64.0f, 64.0f), 0, 0);
                            readbackTexture.Apply();
                        }
                        finally
                        {
                            RenderTexture.active = previousActive;
                        }

                        report.pixel_count = CountLitPixels(readbackTexture);
                        if (report.pixel_count < report.minimum_pixel_count)
                        {
                            throw new InvalidOperationException(
                                "Renderer did not consume the GPU output buffer; lit pixels="
                                + report.pixel_count + ".");
                        }
                        report.renderer_binding_verified = true;
                    }
                    finally
                    {
                        // A graphics dispatch has no readback; Dispose releases
                        // its input/output buffers after Camera.Render returns.
                        dispatch.Dispose();
                    }
                }

                report.status = "measured";
                report.completion_verified = true;
                report.gpu_execution_claim = true;
                WriteReport(reportPath, report);
                Debug.Log(
                    "JADREN animation GPU skinning graphics report=" + reportPath
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
                if (readbackTexture != null) UnityEngine.Object.DestroyImmediate(readbackTexture);
                if (renderTexture != null)
                {
                    renderTexture.Release();
                    UnityEngine.Object.DestroyImmediate(renderTexture);
                }
                if (cameraObject != null) UnityEngine.Object.DestroyImmediate(cameraObject);
                if (rendererObject != null) UnityEngine.Object.DestroyImmediate(rendererObject);
                if (material != null) UnityEngine.Object.DestroyImmediate(material);
                if (mesh != null) UnityEngine.Object.DestroyImmediate(mesh);
            }
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
