using System;
using UnityEngine;
using UnityEngine.Rendering;

namespace Jadren.Animation
{
    /// <summary>
    /// Opt-in renderer-side crowd path. A source mesh is drawn procedurally
    /// once per agent while one flat GPU skinning output buffer is addressed
    /// with SV_InstanceID. The caller owns the vertex and bone arrays and may
    /// update them in place between frames; Unity API calls remain on the main
    /// thread and no GameObject/Animator is created per agent.
    /// </summary>
    [DisallowMultipleComponent]
    [DefaultExecutionOrder(10000)]
    public sealed class JadrenGpuSkinningCrowdRenderer : MonoBehaviour, IDisposable
    {
        [SerializeField] private Mesh sourceMesh;
        [SerializeField] private Material sourceMaterial;
        [SerializeField] private ComputeShader skinningShader;
        [SerializeField] private Shader crowdShader;
        [SerializeField] private Bounds drawBounds = new Bounds(Vector3.zero, Vector3.one * 1000.0f);
        [SerializeField] private bool gpuSkinningEnabled;

        private JadrenAnimationGpuSkinningGraphicsStream graphicsStream;
        private JadrenGpuSkinningVertex[] vertices;
        private Matrix4x4[] boneMatrices;
        private MaterialPropertyBlock propertyBlock;
        private Material proxyMaterial;
        private int agentCount;
        private int verticesPerAgent;
        private bool initialized;
        private bool disposed;

        public Mesh SourceMesh { get { return sourceMesh; } }
        public MaterialPropertyBlock PropertyBlock { get { return propertyBlock; } }
        public bool IsReady { get { return initialized && !disposed; } }
        public bool GpuSkinningEnabled { get { return gpuSkinningEnabled; } }
        public int AgentCount { get { return agentCount; } }
        public int VerticesPerAgent { get { return verticesPerAgent; } }
        public int DrawSubmissionCount { get; private set; }
        public int GraphicsBufferAllocationCount
        {
            get { return graphicsStream == null ? 0 : graphicsStream.BufferAllocationCount; }
        }
        public string LastFailureReason { get; private set; } = string.Empty;

        private void Awake()
        {
            if (gpuSkinningEnabled)
            {
                TryInitialize(out _);
            }
        }

        private void LateUpdate()
        {
            if (gpuSkinningEnabled)
            {
                TryRenderFrame(out _);
            }
        }

        private void OnDisable()
        {
            ReleaseGpuResources();
        }

        private void OnDestroy()
        {
            Dispose();
        }

        /// <summary>
        /// Assigns caller-owned data for one source mesh repeated across the
        /// crowd. Vertex count must be an exact multiple of mesh.vertexCount.
        /// </summary>
        public bool TrySetCrowdData(
            Mesh mesh,
            Material material,
            JadrenGpuSkinningVertex[] crowdVertices,
            Matrix4x4[] crowdBoneMatrices,
            out string failureReason)
        {
            failureReason = string.Empty;
            if (disposed)
            {
                return Fail("crowd_renderer_disposed", out failureReason);
            }
            if (mesh == null || mesh.vertexCount < 1)
            {
                return Fail("crowd_source_mesh_missing", out failureReason);
            }
            if (crowdVertices == null || crowdVertices.Length < mesh.vertexCount
                || crowdVertices.Length % mesh.vertexCount != 0)
            {
                return Fail("crowd_vertex_count_not_mesh_multiple", out failureReason);
            }
            if (crowdBoneMatrices == null || crowdBoneMatrices.Length < 1)
            {
                return Fail("crowd_bone_matrices_missing", out failureReason);
            }

            ReleaseGpuResources();
            sourceMesh = mesh;
            sourceMaterial = material;
            vertices = crowdVertices;
            boneMatrices = crowdBoneMatrices;
            verticesPerAgent = mesh.vertexCount;
            agentCount = crowdVertices.Length / verticesPerAgent;
            DrawSubmissionCount = 0;
            LastFailureReason = string.Empty;
            return true;
        }

        public void SetDrawBounds(Bounds bounds)
        {
            drawBounds = bounds;
        }

        public void SetSkinningShader(ComputeShader shader)
        {
            if (skinningShader == shader)
            {
                return;
            }
            ReleaseGpuResources();
            skinningShader = shader;
        }

        public void SetCrowdShader(Shader shader)
        {
            if (crowdShader == shader)
            {
                return;
            }
            ReleaseGpuResources();
            crowdShader = shader;
        }

        public void SetGpuSkinningEnabled(bool enabled)
        {
            gpuSkinningEnabled = enabled;
            if (!enabled)
            {
                ReleaseGpuResources();
                return;
            }
            disposed = false;
            TryInitialize(out _);
        }

        public bool TryInitialize(out string failureReason)
        {
            failureReason = string.Empty;
            if (disposed)
            {
                return Fail("crowd_renderer_disposed", out failureReason);
            }
            if (initialized)
            {
                return true;
            }
            if (sourceMesh == null || vertices == null || boneMatrices == null)
            {
                return Fail("crowd_data_not_configured", out failureReason);
            }
            if (skinningShader == null)
            {
                skinningShader = Resources.Load<ComputeShader>("JadrenAnimationGpuSkinning");
            }
            if (skinningShader == null)
            {
                return Fail("gpu_skinning_shader_missing", out failureReason);
            }
            if (crowdShader == null)
            {
                crowdShader = Shader.Find("Jadren/Animation/GpuCrowdSkinnedMeshPreview");
            }
            if (crowdShader == null)
            {
                return Fail("gpu_crowd_shader_missing", out failureReason);
            }
            try
            {
                graphicsStream = new JadrenAnimationGpuSkinningGraphicsStream(skinningShader);
                propertyBlock = new MaterialPropertyBlock();
                proxyMaterial = new Material(crowdShader)
                {
                    name = sourceMesh.name + ".JadrenGpuCrowdMaterial"
                };
                CopyMaterialSurface(sourceMaterial, proxyMaterial);
                proxyMaterial.enableInstancing = true;
                initialized = true;
                LastFailureReason = string.Empty;
                return true;
            }
            catch (Exception error)
            {
                ReleaseGpuResources();
                return Fail("crowd_renderer_init_exception:" + error.GetType().Name, out failureReason);
            }
        }

        /// <summary>Dispatches the flat batch and submits one procedural draw.</summary>
        public bool TryRenderFrame(out string failureReason)
        {
            failureReason = string.Empty;
            if (!TryInitialize(out failureReason))
            {
                return false;
            }
            if (!graphicsStream.TryDispatchAndBind(
                    vertices,
                    boneMatrices,
                    propertyBlock,
                    out failureReason))
            {
                return Fail(failureReason, out failureReason);
            }

            propertyBlock.SetInt("_JadrenGpuCrowdVerticesPerInstance", verticesPerAgent);
            propertyBlock.SetInt("_JadrenGpuCrowdInstanceCount", agentCount);
            try
            {
                Graphics.DrawMeshInstancedProcedural(
                    sourceMesh,
                    0,
                    proxyMaterial,
                    drawBounds,
                    agentCount,
                    propertyBlock,
                    ShadowCastingMode.Off,
                    false,
                    gameObject.layer,
                    null,
                    LightProbeUsage.Off,
                    null);
                DrawSubmissionCount++;
                LastFailureReason = string.Empty;
                return true;
            }
            catch (Exception error)
            {
                return Fail("crowd_draw_exception:" + error.GetType().Name, out failureReason);
            }
        }

        /// <summary>Requests a diagnostic readback after a submitted frame.</summary>
        public bool TryRequestReadback(
            out AsyncGPUReadbackRequest request,
            out string failureReason)
        {
            if (graphicsStream == null)
            {
                request = default;
                failureReason = "crowd_stream_unavailable";
                return false;
            }
            return graphicsStream.TryRequestReadback(out request, out failureReason);
        }

        public void Dispose()
        {
            if (disposed)
            {
                return;
            }
            disposed = true;
            ReleaseGpuResources();
            GC.SuppressFinalize(this);
        }

        private bool Fail(string reason, out string failureReason)
        {
            failureReason = reason;
            LastFailureReason = reason;
            return false;
        }

        private void ReleaseGpuResources()
        {
            if (graphicsStream != null)
            {
                graphicsStream.Dispose();
                graphicsStream = null;
            }
            if (proxyMaterial != null)
            {
                if (Application.isPlaying)
                {
                    Destroy(proxyMaterial);
                }
                else
                {
                    DestroyImmediate(proxyMaterial);
                }
                proxyMaterial = null;
            }
            propertyBlock = null;
            initialized = false;
        }

        private static void CopyMaterialSurface(Material source, Material destination)
        {
            if (source == null || destination == null)
            {
                return;
            }
            if (source.HasProperty("_BaseColor") && destination.HasProperty("_BaseColor"))
            {
                destination.SetColor("_BaseColor", source.GetColor("_BaseColor"));
            }
            else if (source.HasProperty("_Color") && destination.HasProperty("_BaseColor"))
            {
                destination.SetColor("_BaseColor", source.GetColor("_Color"));
            }
            if (source.HasProperty("_MainTex") && destination.HasProperty("_MainTex"))
            {
                destination.SetTexture("_MainTex", source.GetTexture("_MainTex"));
            }
            if (source.HasProperty("_BumpMap") && destination.HasProperty("_BumpMap"))
            {
                destination.SetTexture("_BumpMap", source.GetTexture("_BumpMap"));
            }
            if (source.HasProperty("_BumpScale") && destination.HasProperty("_BumpScale"))
            {
                destination.SetFloat("_BumpScale", source.GetFloat("_BumpScale"));
            }
        }
    }
}
