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
        [SerializeField] private Material[] sourceMaterials;
        [SerializeField] private ComputeShader skinningShader;
        [SerializeField] private Shader crowdShader;
        [SerializeField] private Bounds drawBounds = new Bounds(Vector3.zero, Vector3.one * 1000.0f);
        [SerializeField] private bool gpuSkinningEnabled;

        private JadrenAnimationGpuSkinningGraphicsStream graphicsStream;
        private JadrenGpuSkinningVertex[] vertices;
        private Matrix4x4[] boneMatrices;
        private MaterialPropertyBlock propertyBlock;
        private Material proxyMaterial;
        private Material[] proxyMaterials = Array.Empty<Material>();
        private ComputeBuffer externalBoneMatrixBuffer;
        private int externalBoneMatrixCount;
        private int agentCount;
        private int verticesPerAgent;
        private int bonesPerAgent;
        private bool sharedVertexLayout;
        private int drawSubmeshCount;
        private bool initialized;
        private bool disposed;

        public Mesh SourceMesh { get { return sourceMesh; } }
        public MaterialPropertyBlock PropertyBlock { get { return propertyBlock; } }
        public bool IsReady { get { return initialized && !disposed; } }
        public bool GpuSkinningEnabled { get { return gpuSkinningEnabled; } }
        public int AgentCount { get { return agentCount; } }
        public int VerticesPerAgent { get { return verticesPerAgent; } }
        public int BonesPerAgent { get { return bonesPerAgent; } }
        public bool UsesSharedVertexLayout { get { return sharedVertexLayout; } }
        public int DrawSubmissionCount { get; private set; }
        public int DrawSubmeshCount { get { return drawSubmeshCount; } }
        public int MaterialCount { get { return sourceMaterials == null ? 0 : sourceMaterials.Length; } }
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
            return TrySetCrowdData(
                mesh,
                material,
                crowdVertices,
                crowdBoneMatrices,
                0,
                out failureReason);
        }

        /// <summary>
        /// Configures one mesh with one material per submesh. The material
        /// array must either contain one entry (submesh zero only, preserving
        /// the original Phase-1 contract) or exactly mesh.subMeshCount entries.
        /// </summary>
        public bool TrySetCrowdMaterials(
            Mesh mesh,
            Material[] materials,
            JadrenGpuSkinningVertex[] crowdVertices,
            Matrix4x4[] crowdBoneMatrices,
            out string failureReason)
        {
            return TryConfigureCrowdData(
                mesh,
                materials,
                crowdVertices,
                crowdBoneMatrices,
                0,
                0,
                false,
                out failureReason);
        }
        public bool UsesExternalBoneMatrices { get { return externalBoneMatrixBuffer != null; } }

        /// <summary>
        /// Configures an explicit per-agent layout. Vertices contain one
        /// complete mesh copy per agent and matrices are packed as
        /// <c>[agent][bone]</c>. Passing zero preserves the legacy flat
        /// matrix-index contract used by the original synthetic smoke.
        /// </summary>
        public bool TrySetCrowdData(
            Mesh mesh,
            Material material,
            JadrenGpuSkinningVertex[] crowdVertices,
            Matrix4x4[] crowdBoneMatrices,
            int bonesPerAgent,
            out string failureReason)
        {
            return TryConfigureCrowdData(
                mesh,
                material,
                crowdVertices,
                crowdBoneMatrices,
                0,
                bonesPerAgent,
                false,
                out failureReason);
        }

        /// <summary>
        /// Configures a crowd with one shared source-mesh vertex array. The
        /// matrix array is still packed as <c>[agent][bone]</c>, while the
        /// procedural draw expands the compute output by instance ID.
        /// </summary>
        public bool TrySetSharedCrowdData(
            Mesh mesh,
            Material material,
            JadrenGpuSkinningVertex[] sourceVertices,
            Matrix4x4[] crowdBoneMatrices,
            int agentCount,
            int bonesPerAgent,
            out string failureReason)
        {
            return TryConfigureCrowdData(
                mesh,
                material,
                sourceVertices,
                crowdBoneMatrices,
                agentCount,
                bonesPerAgent,
                true,
                out failureReason);
        }

        /// <summary>
        /// Configures shared vertices with one material per submesh. The
        /// compute pose buffer remains shared by every submesh draw; only the
        /// material binding changes, so multi-material characters add draw
        /// submissions without duplicating animation work.
        /// </summary>
        public bool TrySetSharedCrowdMaterials(
            Mesh mesh,
            Material[] materials,
            JadrenGpuSkinningVertex[] sourceVertices,
            Matrix4x4[] crowdBoneMatrices,
            int agentCount,
            int bonesPerAgent,
            out string failureReason)
        {
            return TryConfigureCrowdData(
                mesh,
                materials,
                sourceVertices,
                crowdBoneMatrices,
                agentCount,
                bonesPerAgent,
                true,
                out failureReason);
        }

        private bool TryConfigureCrowdData(
            Mesh mesh,
            Material material,
            JadrenGpuSkinningVertex[] crowdVertices,
            Matrix4x4[] crowdBoneMatrices,
            int requestedAgentCount,
            int bonesPerAgent,
            bool sharedVertices,
            out string failureReason)
        {
            return TryConfigureCrowdData(
                mesh,
                new[] { material },
                crowdVertices,
                crowdBoneMatrices,
                requestedAgentCount,
                bonesPerAgent,
                sharedVertices,
                out failureReason);
        }

        private bool TryConfigureCrowdData(
            Mesh mesh,
            Material[] materials,
            JadrenGpuSkinningVertex[] crowdVertices,
            Matrix4x4[] crowdBoneMatrices,
            int requestedAgentCount,
            int bonesPerAgent,
            bool sharedVertices,
            out string failureReason)
        {
            var effectiveMaterials = materials == null || materials.Length == 0
                ? new[] { (Material)null }
                : materials;
            return TryConfigureCrowdDataCore(
                mesh,
                effectiveMaterials,
                crowdVertices,
                crowdBoneMatrices,
                requestedAgentCount,
                bonesPerAgent,
                sharedVertices,
                out failureReason);
        }

        private bool TryConfigureCrowdDataCore(
            Mesh mesh,
            Material[] materials,
            JadrenGpuSkinningVertex[] crowdVertices,
            Matrix4x4[] crowdBoneMatrices,
            int requestedAgentCount,
            int bonesPerAgent,
            bool sharedVertices,
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
                || (sharedVertices
                    ? crowdVertices.Length != mesh.vertexCount
                    : crowdVertices.Length % mesh.vertexCount != 0))
            {
                return Fail("crowd_vertex_count_not_mesh_multiple", out failureReason);
            }
            if (crowdBoneMatrices == null || crowdBoneMatrices.Length < 1)
            {
                return Fail("crowd_bone_matrices_missing", out failureReason);
            }
            if (bonesPerAgent < 0)
            {
                return Fail("crowd_bones_per_agent_invalid", out failureReason);
            }
            if (sharedVertices && (requestedAgentCount < 1 || bonesPerAgent < 1))
            {
                return Fail("crowd_shared_layout_invalid", out failureReason);
            }
            var submeshCount = Mathf.Max(1, mesh.subMeshCount);
            if (materials == null || materials.Length < 1
                || (materials.Length != 1 && materials.Length != submeshCount))
            {
                return Fail("crowd_material_count_not_submesh_count", out failureReason);
            }
            if (bonesPerAgent > 0)
            {
                for (var vertexIndex = 0; vertexIndex < crowdVertices.Length; vertexIndex++)
                {
                    if (!crowdVertices[vertexIndex].TryValidate(bonesPerAgent, out var vertexReason))
                    {
                        return Fail(
                            "crowd_vertex_input_invalid:" + vertexIndex + ":" + vertexReason,
                            out failureReason);
                    }
                }
            }

            if (!JadrenAnimationGpuSkinningDispatcher.TryValidateInputs(
                    crowdVertices,
                    crowdBoneMatrices,
                    out var inputFailure))
            {
                return Fail(inputFailure, out failureReason);
            }

            var agentCount = sharedVertices
                ? requestedAgentCount
                : crowdVertices.Length / mesh.vertexCount;
            if (agentCount < 1)
            {
                return Fail("crowd_agent_count_invalid", out failureReason);
            }
            if (bonesPerAgent > 0
                && (long)crowdBoneMatrices.Length != (long)agentCount * bonesPerAgent)
            {
                return Fail("crowd_bone_matrices_not_agent_multiple", out failureReason);
            }

            ReleaseGpuResources();
            sourceMesh = mesh;
            sourceMaterials = new Material[materials.Length == submeshCount ? submeshCount : 1];
            Array.Copy(materials, sourceMaterials, sourceMaterials.Length);
            sourceMaterial = sourceMaterials[0];
            vertices = crowdVertices;
            boneMatrices = crowdBoneMatrices;
            verticesPerAgent = mesh.vertexCount;
            this.agentCount = agentCount;
            this.bonesPerAgent = bonesPerAgent;
            sharedVertexLayout = sharedVertices;
            drawSubmeshCount = sourceMaterials.Length;
            DrawSubmissionCount = 0;
            LastFailureReason = string.Empty;
            return true;
        }

        public void SetDrawBounds(Bounds bounds)
        {
            drawBounds = bounds;
        }

        /// <summary>
        /// Binds a caller-owned GPU animation output buffer. The renderer does
        /// not release this buffer; the animation stream owns its lifetime.
        /// </summary>
        public void SetExternalBoneMatrices(ComputeBuffer buffer, int matrixCount)
        {
            externalBoneMatrixBuffer = buffer;
            externalBoneMatrixCount = Mathf.Max(0, matrixCount);
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
                // Reuse the parity-tested skinned preview shader. It handles
                // shared crowd vertices, SRP unlit passes and the same
                // instance/vertex addressing as the single-character path.
                crowdShader = Shader.Find("Jadren/Animation/GpuSkinnedMeshPreview");
            }
            if (crowdShader == null)
            {
                return Fail("gpu_crowd_shader_missing", out failureReason);
            }
            if (sourceMaterials == null || sourceMaterials.Length < 1)
            {
                sourceMaterials = new[] { sourceMaterial };
            }
            drawSubmeshCount = sourceMaterials.Length;
            try
            {
                graphicsStream = new JadrenAnimationGpuSkinningGraphicsStream(skinningShader);
                propertyBlock = new MaterialPropertyBlock();
                proxyMaterials = new Material[drawSubmeshCount];
                for (var submesh = 0; submesh < drawSubmeshCount; submesh++)
                {
                    var proxy = new Material(crowdShader)
                    {
                        name = sourceMesh.name + ".JadrenGpuCrowdMaterial" + submesh
                    };
                    CopyMaterialSurface(sourceMaterials[submesh], proxy);
                    proxy.enableInstancing = true;
                    proxyMaterials[submesh] = proxy;
                }
                proxyMaterial = proxyMaterials[0];
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

        /// <summary>
        /// Dispatches the flat batch once and submits one procedural draw per
        /// configured submesh/material. Multi-material meshes therefore share
        /// animation work while preserving their material assignments.
        /// </summary>
        public bool TryRenderFrame(out string failureReason)
        {
            failureReason = string.Empty;
            if (!TryInitialize(out failureReason))
            {
                return false;
            }
            var dispatchSucceeded = sharedVertexLayout && externalBoneMatrixBuffer != null
                ? graphicsStream.TryDispatchAndBindSharedVertices(
                    vertices,
                    externalBoneMatrixBuffer,
                    externalBoneMatrixCount,
                    propertyBlock,
                    agentCount,
                    bonesPerAgent,
                    false,
                    out failureReason)
                : sharedVertexLayout
                    ? graphicsStream.TryDispatchAndBindSharedVertices(
                        vertices,
                        boneMatrices,
                        propertyBlock,
                        agentCount,
                        bonesPerAgent,
                        false,
                        out failureReason)
                    : graphicsStream.TryDispatchAndBind(
                        vertices,
                        boneMatrices,
                        propertyBlock,
                        bonesPerAgent > 0 ? verticesPerAgent : 0,
                        bonesPerAgent,
                        false,
                        out failureReason);
            if (!dispatchSucceeded)
            {
                return Fail(failureReason, out failureReason);
            }

            propertyBlock.SetInt("_JadrenGpuCrowdVerticesPerInstance", verticesPerAgent);
            propertyBlock.SetInt("_JadrenGpuCrowdBonesPerInstance", bonesPerAgent);
            propertyBlock.SetInt("_JadrenGpuCrowdInstanceCount", agentCount);
            propertyBlock.SetInt("_JadrenGpuCrowdSharedVertices", sharedVertexLayout ? 1 : 0);
            try
            {
                for (var submesh = 0; submesh < drawSubmeshCount; submesh++)
                {
                    Graphics.DrawMeshInstancedProcedural(
                        sourceMesh,
                        submesh,
                        proxyMaterials[submesh],
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
                }
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

        /// <summary>
        /// Diagnostic-only position bounds for the last dispatched crowd.
        /// This intentionally waits for the GPU and is not part of the
        /// runtime render path; it is used by the Phase-1 parity harness to
        /// catch a deformed/collapsed output before comparing FPS.
        /// </summary>
        public bool TryReadbackPositionBounds(
            out Vector3 minimum,
            out Vector3 maximum,
            out string failureReason)
        {
            minimum = new Vector3(float.PositiveInfinity, float.PositiveInfinity, float.PositiveInfinity);
            maximum = new Vector3(float.NegativeInfinity, float.NegativeInfinity, float.NegativeInfinity);
            if (!TryRequestReadback(out var request, out failureReason))
            {
                return false;
            }
            request.WaitForCompletion();
            if (request.hasError)
            {
                failureReason = "gpu_position_readback_error";
                return false;
            }
            var positions = request.GetData<Vector3>();
            for (var index = 0; index < positions.Length; index++)
            {
                minimum = Vector3.Min(minimum, positions[index]);
                maximum = Vector3.Max(maximum, positions[index]);
            }
            failureReason = string.Empty;
            return positions.Length > 0;
        }

        /// <summary>
        /// Diagnostic-only copy of the last dispatched positions. The method
        /// waits for GPU completion and is intentionally excluded from the
        /// benchmark hot path; it allows backend parity tools to compare the
        /// GPU pose stream with the managed matrix fallback.
        /// </summary>
        public bool TryReadbackPositions(
            out Vector3[] positions,
            out string failureReason)
        {
            positions = Array.Empty<Vector3>();
            if (!TryRequestReadback(out var request, out failureReason))
            {
                return false;
            }
            request.WaitForCompletion();
            if (request.hasError)
            {
                failureReason = "gpu_position_readback_error";
                return false;
            }
            var data = request.GetData<Vector3>();
            positions = new Vector3[data.Length];
            data.CopyTo(positions);
            failureReason = string.Empty;
            return positions.Length > 0;
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
            externalBoneMatrixBuffer = null;
            externalBoneMatrixCount = 0;
            if (proxyMaterials != null)
            {
                for (var index = 0; index < proxyMaterials.Length; index++)
                {
                    var proxy = proxyMaterials[index];
                    if (proxy == null)
                    {
                        continue;
                    }
                    if (Application.isPlaying)
                    {
                        Destroy(proxy);
                    }
                    else
                    {
                        DestroyImmediate(proxy);
                    }
                }
                proxyMaterials = Array.Empty<Material>();
            }
            proxyMaterial = null;
            propertyBlock = null;
            agentCount = 0;
            verticesPerAgent = 0;
            bonesPerAgent = 0;
            sharedVertexLayout = false;
            drawSubmeshCount = 0;
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
            if (source.HasProperty("_BaseMap") && destination.HasProperty("_MainTex"))
            {
                destination.SetTexture("_MainTex", source.GetTexture("_BaseMap"));
                destination.SetTextureScale("_MainTex", source.GetTextureScale("_BaseMap"));
                destination.SetTextureOffset("_MainTex", source.GetTextureOffset("_BaseMap"));
            }
            else if (source.HasProperty("_MainTex") && destination.HasProperty("_MainTex"))
            {
                destination.SetTexture("_MainTex", source.GetTexture("_MainTex"));
                destination.SetTextureScale("_MainTex", source.GetTextureScale("_MainTex"));
                destination.SetTextureOffset("_MainTex", source.GetTextureOffset("_MainTex"));
            }
            else if (source.HasProperty("_BaseColorMap") && destination.HasProperty("_MainTex"))
            {
                destination.SetTexture("_MainTex", source.GetTexture("_BaseColorMap"));
                destination.SetTextureScale("_MainTex", source.GetTextureScale("_BaseColorMap"));
                destination.SetTextureOffset("_MainTex", source.GetTextureOffset("_BaseColorMap"));
            }
            if (source.HasProperty("_BumpMap") && destination.HasProperty("_BumpMap"))
            {
                destination.SetTexture("_BumpMap", source.GetTexture("_BumpMap"));
            }
            else if (source.HasProperty("_NormalMap") && destination.HasProperty("_BumpMap"))
            {
                destination.SetTexture("_BumpMap", source.GetTexture("_NormalMap"));
            }
            if (source.HasProperty("_BumpScale") && destination.HasProperty("_BumpScale"))
            {
                destination.SetFloat("_BumpScale", source.GetFloat("_BumpScale"));
            }
            if (destination.HasProperty("_JadrenCull"))
            {
                var cull = source.HasProperty("_CullMode")
                    ? source.GetFloat("_CullMode")
                    : source.HasProperty("_Cull")
                        ? source.GetFloat("_Cull")
                        : 0.0f;
                destination.SetFloat("_JadrenCull", Mathf.Clamp(cull, 0.0f, 2.0f));
            }
            if (destination.HasProperty("_JadrenZWrite"))
            {
                var zWrite = source.HasProperty("_ZWrite")
                    ? source.GetFloat("_ZWrite")
                    : 0.0f;
                destination.SetFloat("_JadrenZWrite", zWrite > 0.5f ? 1.0f : 0.0f);
            }
        }
    }
}
