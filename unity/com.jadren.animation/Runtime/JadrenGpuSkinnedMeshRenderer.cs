using System;
using UnityEngine;

namespace Jadren.Animation
{
    /// <summary>
    /// Opt-in host for the GPU skinning MVP. It snapshots a real
    /// SkinnedMeshRenderer's readable mesh and bone matrices, dispatches the
    /// caller-owned data to the compute shader, and renders the completed
    /// position buffer through a child MeshRenderer proxy. The source
    /// SkinnedMeshRenderer remains the safe fallback until a frame has been
    /// bound successfully.
    /// </summary>
    [DisallowMultipleComponent]
    [RequireComponent(typeof(SkinnedMeshRenderer))]
    [DefaultExecutionOrder(10000)]
    public sealed class JadrenGpuSkinnedMeshRenderer : MonoBehaviour, IDisposable
    {
        [SerializeField] private SkinnedMeshRenderer sourceRenderer;
        [SerializeField] private ComputeShader skinningShader;
        [SerializeField] private Shader proxyShader;
        [SerializeField] private bool gpuSkinningEnabled;
        [SerializeField] private bool disableSourceRenderer = true;
        [SerializeField] private float boundsPadding = 1.0f;
        [SerializeField] private bool presentationEnabled = true;

        private JadrenAnimationGpuSkinningGraphicsStream graphicsStream;
        private JadrenGpuSkinningVertex[] vertices;
        private Matrix4x4[] boneMatrices;
        private Mesh sourceMesh;
        private Mesh proxyMesh;
        private GameObject proxyObject;
        private MeshRenderer proxyRenderer;
        private Material[] proxyMaterials;
        private MaterialPropertyBlock propertyBlock;
        private bool sourceRendererWasEnabled;
        private bool hasRenderedFrame;
        private bool initialized;
        private bool disposed;

        public SkinnedMeshRenderer SourceRenderer { get { return sourceRenderer; } }
        public MeshRenderer ProxyRenderer { get { return proxyRenderer; } }
        public bool IsReady { get { return initialized && !disposed; } }
        public bool GpuSkinningEnabled { get { return gpuSkinningEnabled; } }
        public bool PresentationEnabled { get { return presentationEnabled; } }
        /// <summary>
        /// True only after a complete GPU dispatch has been bound to the proxy.
        /// Until then the source SkinnedMeshRenderer remains the safe visible
        /// fallback, including during the first Play-mode frame.
        /// </summary>
        public bool HasRenderedFrame { get { return hasRenderedFrame; } }
        public int VertexCount { get { return vertices == null ? 0 : vertices.Length; } }
        public int ProxyMaterialCount
        {
            get { return proxyMaterials == null ? 0 : proxyMaterials.Length; }
        }
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

        private void OnEnable()
        {
            disposed = false;
            if (gpuSkinningEnabled)
            {
                TryInitialize(out _);
            }
        }

        private void LateUpdate()
        {
            if (gpuSkinningEnabled && presentationEnabled)
            {
                TryRenderFrame(out _);
            }
            else if (proxyRenderer != null)
            {
                proxyRenderer.enabled = false;
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

        /// <summary>Sets the compute shader used by the opt-in host.</summary>
        public void SetSkinningShader(ComputeShader shader)
        {
            if (skinningShader == shader)
            {
                return;
            }
            ReleaseGpuResources();
            skinningShader = shader;
        }

        /// <summary>Sets the shader used by the child proxy renderer.</summary>
        public void SetProxyShader(Shader shader)
        {
            if (proxyShader == shader)
            {
                return;
            }
            ReleaseGpuResources();
            proxyShader = shader;
        }

        /// <summary>
        /// Enables or disables the GPU host. Disabling always restores the
        /// source renderer's original enabled state.
        /// </summary>
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

        /// <summary>
        /// Gates only presentation/dispatch for visibility LOD. It keeps the
        /// reusable GPU resources alive, unlike SetGpuSkinningEnabled(false),
        /// so a camera crossing a culling boundary does not reallocate buffers.
        /// </summary>
        public void SetPresentationEnabled(bool enabled)
        {
            presentationEnabled = enabled;
            if (!enabled)
            {
                if (proxyRenderer != null)
                {
                    proxyRenderer.enabled = false;
                }
                if (sourceRenderer != null && initialized && disableSourceRenderer)
                {
                    sourceRenderer.enabled = hasRenderedFrame ? false : sourceRendererWasEnabled;
                }
                return;
            }

            if (!gpuSkinningEnabled || !initialized)
            {
                if (sourceRenderer != null)
                {
                    sourceRenderer.enabled = sourceRendererWasEnabled;
                }
                return;
            }
            if (sourceRenderer != null && disableSourceRenderer && hasRenderedFrame)
            {
                sourceRenderer.enabled = false;
            }
        }

        /// <summary>
        /// Builds the proxy from the current readable SkinnedMeshRenderer.
        /// This method is main-thread only because it reads Unity mesh and
        /// scene objects.
        /// </summary>
        public bool TryInitialize(out string failureReason)
        {
            failureReason = string.Empty;
            if (disposed)
            {
                failureReason = LastFailureReason = "renderer_host_disposed";
                return false;
            }
            if (initialized)
            {
                return true;
            }
            if (sourceRenderer == null)
            {
                sourceRenderer = GetComponent<SkinnedMeshRenderer>();
            }
            if (sourceRenderer == null)
            {
                return Fail("source_skinned_renderer_missing", out failureReason);
            }
            if (skinningShader == null)
            {
                skinningShader = Resources.Load<ComputeShader>("JadrenAnimationGpuSkinning");
            }
            if (skinningShader == null)
            {
                return Fail("gpu_skinning_shader_missing", out failureReason);
            }
            if (proxyShader == null)
            {
                proxyShader = Shader.Find("Jadren/Animation/GpuSkinnedMeshPreview");
            }
            if (proxyShader == null)
            {
                return Fail("gpu_skinning_proxy_shader_missing", out failureReason);
            }
            sourceMesh = sourceRenderer.sharedMesh;
            if (sourceMesh == null)
            {
                return Fail("source_mesh_missing", out failureReason);
            }
            try
            {
                var positions = sourceMesh.vertices;
                var weights = sourceMesh.boneWeights;
                var bindposes = sourceMesh.bindposes;
                var bones = sourceRenderer.bones;
                if (positions == null || positions.Length < 1)
                {
                    return Fail("source_mesh_vertices_missing", out failureReason);
                }
                if (weights == null || weights.Length != positions.Length)
                {
                    return Fail("source_mesh_bone_weights_invalid", out failureReason);
                }
                if (bindposes == null || bindposes.Length < 1 || bones == null
                    || bones.Length < bindposes.Length)
                {
                    return Fail("source_mesh_bone_bindings_invalid", out failureReason);
                }

                vertices = new JadrenGpuSkinningVertex[positions.Length];
                for (var index = 0; index < positions.Length; index++)
                {
                    var weight = weights[index];
                    vertices[index] = new JadrenGpuSkinningVertex(
                        positions[index],
                        new Vector4(weight.weight0, weight.weight1, weight.weight2, weight.weight3),
                        new Vector4(weight.boneIndex0, weight.boneIndex1, weight.boneIndex2, weight.boneIndex3));
                }

                boneMatrices = new Matrix4x4[bindposes.Length];
                graphicsStream = new JadrenAnimationGpuSkinningGraphicsStream(skinningShader);
                BuildProxyMesh(sourceMesh);
                propertyBlock = new MaterialPropertyBlock();
                sourceRendererWasEnabled = sourceRenderer.enabled;
                // Keep the source renderer enabled until the first successful
                // dispatch. A shader/compute failure must not make Play mode
                // render an empty character while the proxy is still warming.
                hasRenderedFrame = false;
                initialized = true;
                LastFailureReason = string.Empty;
                return true;
            }
            catch (Exception error)
            {
                ReleaseGpuResources();
                return Fail(
                    "source_mesh_read_exception:" + error.GetType().Name,
                    out failureReason);
            }
        }

        /// <summary>
        /// Dispatches one frame of GPU skinning and binds its output to the
        /// proxy renderer. The stream retains its buffers across frames and
        /// releases them only when this host is disabled or destroyed.
        /// </summary>
        public bool TryRenderFrame(out string failureReason)
        {
            failureReason = string.Empty;
            if (!TryInitialize(out failureReason))
            {
                return false;
            }
            var bones = sourceRenderer.bones;
            var bindposes = sourceMesh.bindposes;
            if (bones == null || bones.Length < bindposes.Length)
            {
                return FailFrame("source_bones_changed", out failureReason);
            }
            var worldToLocal = sourceRenderer.transform.worldToLocalMatrix;
            for (var index = 0; index < bindposes.Length; index++)
            {
                var bone = bones[index];
                if (bone == null)
                {
                    return FailFrame("source_bone_missing:" + index, out failureReason);
                }
                boneMatrices[index] = worldToLocal * bone.localToWorldMatrix * bindposes[index];
            }

            propertyBlock.Clear();
            if (!graphicsStream.TryDispatchAndBind(
                    vertices,
                    boneMatrices,
                    propertyBlock,
                    out failureReason))
            {
                LastFailureReason = failureReason;
                if (proxyRenderer != null)
                {
                    proxyRenderer.enabled = false;
                }
                sourceRenderer.enabled = sourceRendererWasEnabled;
                return false;
            }
            proxyRenderer.SetPropertyBlock(propertyBlock);
            if (disableSourceRenderer)
            {
                sourceRenderer.enabled = false;
            }
            proxyRenderer.enabled = true;
            hasRenderedFrame = true;
            LastFailureReason = string.Empty;
            return true;
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

        private void BuildProxyMesh(Mesh source)
        {
            proxyMesh = Instantiate(source);
            proxyMesh.name = source.name + ".JadrenGpuProxy";
            var bounds = sourceRenderer.localBounds;
            var padding = Mathf.Max(0.0f, boundsPadding);
            bounds.Expand(Vector3.one * (padding * 2.0f));
            proxyMesh.bounds = bounds;

            proxyObject = new GameObject(source.name + ".JadrenGpuProxyRenderer");
            proxyObject.transform.SetParent(sourceRenderer.transform, false);
            var filter = proxyObject.AddComponent<MeshFilter>();
            filter.sharedMesh = proxyMesh;
            proxyRenderer = proxyObject.AddComponent<MeshRenderer>();
            var sourceMaterials = sourceRenderer.sharedMaterials;
            if (sourceMaterials == null || sourceMaterials.Length < 1)
            {
                sourceMaterials = new Material[1];
            }
            proxyMaterials = new Material[sourceMaterials.Length];
            for (var materialIndex = 0; materialIndex < sourceMaterials.Length; materialIndex++)
            {
                var proxyMaterial = new Material(proxyShader)
                {
                    name = source.name + ".JadrenGpuProxyMaterial." + materialIndex
                };
                CopyMaterialSurface(sourceMaterials[materialIndex], proxyMaterial);
                proxyMaterials[materialIndex] = proxyMaterial;
            }
            proxyRenderer.sharedMaterials = proxyMaterials;
            proxyRenderer.enabled = false;
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
            if (!CopyTextureSurface(source, destination, "_BaseMap", "_MainTex"))
            {
                CopyTextureSurface(source, destination, "_MainTex", "_MainTex");
            }
            CopyTextureSurface(source, destination, "_BumpMap", "_BumpMap");
            if (source.HasProperty("_BumpScale") && destination.HasProperty("_BumpScale"))
            {
                destination.SetFloat("_BumpScale", source.GetFloat("_BumpScale"));
            }
        }

        private static bool CopyTextureSurface(
            Material source,
            Material destination,
            string sourceProperty,
            string destinationProperty)
        {
            if (!source.HasProperty(sourceProperty) || !destination.HasProperty(destinationProperty))
            {
                return false;
            }
            var texture = source.GetTexture(sourceProperty);
            if (texture == null)
            {
                return false;
            }
            destination.SetTexture(destinationProperty, texture);
            destination.SetTextureScale(destinationProperty, source.GetTextureScale(sourceProperty));
            destination.SetTextureOffset(destinationProperty, source.GetTextureOffset(sourceProperty));
            return true;
        }

        private bool Fail(string reason, out string failureReason)
        {
            failureReason = reason;
            LastFailureReason = reason;
            return false;
        }

        private bool FailFrame(string reason, out string failureReason)
        {
            if (proxyRenderer != null)
            {
                proxyRenderer.enabled = false;
            }
            if (sourceRenderer != null)
            {
                sourceRenderer.enabled = sourceRendererWasEnabled;
            }
            return Fail(reason, out failureReason);
        }

        private void ReleaseGpuResources()
        {
            if (sourceRenderer != null && initialized)
            {
                sourceRenderer.enabled = sourceRendererWasEnabled;
            }
            if (graphicsStream != null)
            {
                graphicsStream.Dispose();
                graphicsStream = null;
            }
            if (proxyMaterials != null)
            {
                for (var materialIndex = 0; materialIndex < proxyMaterials.Length; materialIndex++)
                {
                    DestroyUnityObject(proxyMaterials[materialIndex]);
                }
            }
            DestroyUnityObject(proxyObject);
            DestroyUnityObject(proxyMesh);
            proxyMaterials = null;
            proxyObject = null;
            proxyMesh = null;
            proxyRenderer = null;
            propertyBlock = null;
            vertices = null;
            boneMatrices = null;
            sourceMesh = null;
            hasRenderedFrame = false;
            initialized = false;
        }

        private static void DestroyUnityObject(UnityEngine.Object target)
        {
            if (target == null)
            {
                return;
            }
            if (Application.isPlaying)
            {
                UnityEngine.Object.Destroy(target);
            }
            else
            {
                UnityEngine.Object.DestroyImmediate(target);
            }
        }
    }
}
