using System;
using System.IO;
using Jadren.Animation;
using UnityEditor;
using UnityEngine;
using UnityEngine.Rendering;

namespace Jadren.Animation.Editor
{
    /// <summary>
    /// Graphics smoke for the opt-in SkinnedMeshRenderer proxy host. The
    /// fixture is a real SkinnedMeshRenderer with bone weights and bindposes;
    /// the source renderer is disabled and the visible triangle is produced
    /// by the GPU position buffer consumed by the proxy material.
    /// </summary>
    public static class JadrenAnimationGpuSkinnedMeshRendererBatchRunner
    {
        private const string ReportFileName = "jadren-animation-gpu-skinned-mesh-renderer-smoke.json";

        [Serializable]
        private sealed class Report
        {
            public string schema;
            public string status;
            public string unity_version;
            public string graphics_device;
            public string fixture_kind;
            public bool supports_compute_shaders;
            public bool source_skinned_renderer_verified;
            public bool proxy_renderer_verified;
            public bool renderer_binding_verified;
            public bool completion_verified;
            public bool buffer_reuse_verified;
            public bool normal_lighting_verified;
            public bool normal_map_verified;
            public bool material_slots_verified;
            public bool gpu_execution_claim;
            public int vertex_count;
            public int frames_submitted;
            public int buffer_allocation_count;
            public int material_slot_count;
            public int pixel_count;
            public int minimum_pixel_count;
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
                schema = "jadren-unity-animation-gpu-skinned-mesh-renderer-0.1",
                unity_version = Application.unityVersion,
                graphics_device = capabilities.GraphicsDevice.ToString(),
                fixture_kind = "real_skinned_mesh_renderer_proxy",
                supports_compute_shaders = capabilities.ComputeShadersSupported,
                source_skinned_renderer_verified = false,
                proxy_renderer_verified = false,
                renderer_binding_verified = false,
                completion_verified = false,
                buffer_reuse_verified = false,
                normal_lighting_verified = false,
                normal_map_verified = false,
                material_slots_verified = false,
                gpu_execution_claim = false,
                vertex_count = 0,
                frames_submitted = 0,
                buffer_allocation_count = 0,
                material_slot_count = 0,
                pixel_count = 0,
                minimum_pixel_count = 4,
                failure_reason = string.Empty,
                claim_scope = "real SkinnedMeshRenderer mesh/bindpose proxy binding smoke only; no FPS, frame-time, throughput, texture fidelity or production Entities claim",
                report_path = reportPath
            };

            if (!capabilities.ComputeShadersSupported
                || capabilities.GraphicsDevice == GraphicsDeviceType.Null)
            {
                report.status = "skip-no-gpu";
                report.failure_reason = "compute_or_graphics_device_unavailable";
                WriteReport(reportPath, report);
                Debug.Log("JADREN GPU skinned mesh renderer report=" + reportPath + " status=skip-no-gpu");
                EditorApplication.Exit(0);
                return;
            }

            GameObject sourceObject = null;
            GameObject rootBoneObject = null;
            GameObject boneObject = null;
            Mesh sourceMesh = null;
            Material sourceMaterial = null;
            RenderTexture renderTexture = null;
            Texture2D readbackTexture = null;
            Texture2D normalMapTexture = null;
            JadrenGpuSkinnedMeshRenderer host = null;
            try
            {
                var computeShader = Resources.Load<ComputeShader>("JadrenAnimationGpuSkinning");
                if (computeShader == null)
                {
                    throw new InvalidOperationException("GPU skinning ComputeShader resource is missing.");
                }
                var proxyShader = Shader.Find("Jadren/Animation/GpuSkinnedMeshPreview");
                if (proxyShader == null)
                {
                    throw new InvalidOperationException("GPU skinned mesh proxy shader is missing.");
                }

                sourceObject = new GameObject("JadrenGpuSkinnedMeshRendererSmokeSource");
                rootBoneObject = new GameObject("JadrenGpuSkinnedMeshRendererSmokeRoot");
                rootBoneObject.transform.SetParent(sourceObject.transform, false);
                boneObject = new GameObject("JadrenGpuSkinnedMeshRendererSmokeBone");
                boneObject.transform.SetParent(rootBoneObject.transform, false);

                sourceMesh = new Mesh
                {
                    name = "JadrenGpuSkinnedMeshRendererSmokeMesh",
                    vertices = new[]
                    {
                        new Vector3(-0.55f, -0.45f, 0.0f),
                        new Vector3(0.55f, -0.45f, 0.0f),
                        new Vector3(0.0f, 0.55f, 0.0f)
                    },
                    triangles = new[] { 0, 1, 2 },
                    uv = new[]
                    {
                        new Vector2(0.0f, 0.0f),
                        new Vector2(1.0f, 0.0f),
                        new Vector2(0.5f, 1.0f)
                    },
                    normals = new[]
                    {
                        Vector3.right,
                        Vector3.right,
                        Vector3.right
                    },
                    tangents = new[]
                    {
                        new Vector4(0.0f, 1.0f, 0.0f, 1.0f),
                        new Vector4(0.0f, 1.0f, 0.0f, 1.0f),
                        new Vector4(0.0f, 1.0f, 0.0f, 1.0f)
                    },
                    bindposes = new[] { boneObject.transform.worldToLocalMatrix },
                    boneWeights = new[]
                    {
                        new BoneWeight { boneIndex0 = 0, weight0 = 1.0f },
                        new BoneWeight { boneIndex0 = 0, weight0 = 1.0f },
                        new BoneWeight { boneIndex0 = 0, weight0 = 1.0f }
                    }
                };
                sourceMesh.RecalculateBounds();

                var sourceRenderer = sourceObject.AddComponent<SkinnedMeshRenderer>();
                sourceRenderer.sharedMesh = sourceMesh;
                sourceRenderer.bones = new[] { boneObject.transform };
                sourceRenderer.rootBone = boneObject.transform;
                sourceMaterial = new Material(Shader.Find("Unlit/Color"))
                {
                    name = "JadrenGpuSkinnedMeshRendererSmokeSourceMaterial"
                };
                sourceMaterial.color = Color.white;
                sourceRenderer.sharedMaterials = new[] { sourceMaterial, sourceMaterial };
                report.source_skinned_renderer_verified = sourceRenderer.sharedMesh == sourceMesh
                    && sourceRenderer.bones.Length == 1
                    && sourceRenderer.sharedMaterials.Length == 2
                    && sourceMesh.boneWeights.Length == sourceMesh.vertexCount;

                host = sourceObject.AddComponent<JadrenGpuSkinnedMeshRenderer>();
                host.SetSkinningShader(computeShader);
                host.SetProxyShader(proxyShader);
                host.SetGpuSkinningEnabled(true);
                if (!host.TryInitialize(out var initializationFailure))
                {
                    throw new InvalidOperationException("SkinnedMeshRenderer host initialization failed: " + initializationFailure);
                }
                report.vertex_count = host.VertexCount;
                report.material_slot_count = host.ProxyMaterialCount;
                report.material_slots_verified = report.material_slot_count == 2
                    && host.ProxyRenderer.sharedMaterials.Length == 2;
                if (!report.material_slots_verified)
                {
                    throw new InvalidOperationException(
                        "Proxy material slot count was " + report.material_slot_count + ".");
                }
                if (host.ProxyRenderer == null || host.ProxyRenderer.sharedMaterial == null)
                {
                    throw new InvalidOperationException("GPU proxy material was not created.");
                }
                // Rotate a deliberately non-camera-facing source normal. A
                // shader that does not consume the reusable skinning inputs
                // remains dark; the bound skinned normal is lit by +Y.
                host.ProxyRenderer.sharedMaterial.SetVector(
                    "_JadrenLightDirection",
                    new Vector4(0.0f, 1.0f, 0.0f, 0.0f));
                boneObject.transform.localRotation = Quaternion.Euler(0.0f, 0.0f, 90.0f);
                boneObject.transform.localPosition = new Vector3(0.35f, 0.15f, 0.0f);
                if (!host.TryRenderFrame(out var frameFailure))
                {
                    throw new InvalidOperationException("SkinnedMeshRenderer GPU frame failed: " + frameFailure);
                }
                report.proxy_renderer_verified = host.ProxyRenderer != null
                    && host.ProxyRenderer.enabled
                    && !sourceRenderer.enabled;
                report.frames_submitted = 1;
                report.buffer_allocation_count = host.GraphicsBufferAllocationCount;
                if (report.buffer_allocation_count != 1)
                {
                    throw new InvalidOperationException(
                        "Initial graphics stream allocation count was "
                        + report.buffer_allocation_count + ".");
                }

                var cameraObject = new GameObject("JadrenGpuSkinnedMeshRendererSmokeCamera");
                try
                {
                    var camera = cameraObject.AddComponent<Camera>();
                    camera.clearFlags = CameraClearFlags.SolidColor;
                    camera.backgroundColor = Color.black;
                    camera.orthographic = true;
                    camera.orthographicSize = 2.0f;
                    camera.aspect = 1.0f;
                    camera.nearClipPlane = 0.1f;
                    camera.farClipPlane = 100.0f;
                    camera.transform.position = new Vector3(0.0f, 0.0f, -3.0f);
                    camera.transform.rotation = Quaternion.identity;
                    renderTexture = new RenderTexture(64, 64, 24, RenderTextureFormat.ARGB32)
                    {
                        name = "JadrenGpuSkinnedMeshRendererSmokeTarget"
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
                    var firstPixelCount = CountLitPixels(readbackTexture);
                    if (firstPixelCount < report.minimum_pixel_count)
                    {
                        throw new InvalidOperationException(
                            "GPU skinned mesh proxy produced too few pixels: " + firstPixelCount);
                    }
                    report.normal_lighting_verified = true;

                    // Encode a tangent-space +Y normal and change the light
                    // to +Z. With the 90° rotated fixture, only the sampled
                    // normal map produces the bright result.
                    normalMapTexture = new Texture2D(
                        1,
                        1,
                        TextureFormat.RGBA32,
                        false,
                        true)
                    {
                        name = "JadrenGpuSkinnedMeshRendererSmokeNormalMap"
                    };
                    normalMapTexture.SetPixel(0, 0, new Color(0.5f, 1.0f, 0.5f, 1.0f));
                    normalMapTexture.Apply();
                    host.ProxyRenderer.sharedMaterial.SetTexture("_BumpMap", normalMapTexture);
                    host.ProxyRenderer.sharedMaterial.SetFloat("_BumpScale", 1.0f);
                    host.ProxyRenderer.sharedMaterial.SetVector(
                        "_JadrenLightDirection",
                        new Vector4(0.0f, 0.0f, 1.0f, 0.0f));
                    boneObject.transform.localPosition = new Vector3(-0.25f, 0.2f, 0.0f);
                    if (!host.TryRenderFrame(out frameFailure))
                    {
                        throw new InvalidOperationException(
                            "Second SkinnedMeshRenderer GPU frame failed: " + frameFailure);
                    }
                    report.frames_submitted = 2;
                    report.buffer_allocation_count = host.GraphicsBufferAllocationCount;
                    report.buffer_reuse_verified = report.buffer_allocation_count == 1;
                    if (!report.buffer_reuse_verified)
                    {
                        throw new InvalidOperationException(
                            "Graphics stream reallocated buffers between frames: "
                            + report.buffer_allocation_count);
                    }
                    camera.Render();
                    var secondPreviousActive = RenderTexture.active;
                    try
                    {
                        RenderTexture.active = renderTexture;
                        readbackTexture.ReadPixels(new Rect(0.0f, 0.0f, 64.0f, 64.0f), 0, 0);
                        readbackTexture.Apply();
                    }
                    finally
                    {
                        RenderTexture.active = secondPreviousActive;
                    }
                    report.pixel_count = CountLitPixels(readbackTexture);
                    if (report.pixel_count < report.minimum_pixel_count)
                    {
                        throw new InvalidOperationException(
                            "Second GPU skinned mesh proxy frame produced too few pixels: "
                            + report.pixel_count);
                    }
                    report.normal_map_verified = true;
                }
                finally
                {
                    UnityEngine.Object.DestroyImmediate(cameraObject);
                }

                report.renderer_binding_verified = true;
                report.completion_verified = true;
                report.status = "measured";
                report.gpu_execution_claim = true;
                WriteReport(reportPath, report);
                Debug.Log(
                    "JADREN GPU skinned mesh renderer report=" + reportPath
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
                if (host != null)
                {
                    host.Dispose();
                }
                if (readbackTexture != null) UnityEngine.Object.DestroyImmediate(readbackTexture);
                if (normalMapTexture != null) UnityEngine.Object.DestroyImmediate(normalMapTexture);
                if (renderTexture != null)
                {
                    renderTexture.Release();
                    UnityEngine.Object.DestroyImmediate(renderTexture);
                }
                if (sourceObject != null) UnityEngine.Object.DestroyImmediate(sourceObject);
                if (rootBoneObject != null) UnityEngine.Object.DestroyImmediate(rootBoneObject);
                if (boneObject != null) UnityEngine.Object.DestroyImmediate(boneObject);
                if (sourceMaterial != null) UnityEngine.Object.DestroyImmediate(sourceMaterial);
                if (sourceMesh != null) UnityEngine.Object.DestroyImmediate(sourceMesh);
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
