using System;
using UnityEngine;
using UnityEngine.Rendering;

namespace Jadren.Animation
{
    /// <summary>
    /// Reusable main-thread graphics path for GPU skinning. Vertex, matrix and
    /// output buffers grow only when the workload exceeds their current
    /// capacity; the caller must dispatch after the previous frame consumed
    /// the bound output (LateUpdate is the intended Unity lifecycle point).
    /// No CPU readback is requested by this class.
    /// </summary>
    public sealed class JadrenAnimationGpuSkinningGraphicsStream : IDisposable
    {
        private readonly ComputeShader shader;
        private readonly int kernel;
        private readonly uint threadsX;
        private ComputeBuffer vertexBuffer;
        private ComputeBuffer matrixBuffer;
        private ComputeBuffer outputBuffer;
        private int vertexCapacity;
        private int boneCapacity;
        private int outputCapacity;
        private int bufferAllocationCount;
        private bool disposed;

        public JadrenAnimationGpuSkinningGraphicsStream(ComputeShader shader)
        {
            if (shader == null)
            {
                throw new ArgumentNullException(nameof(shader));
            }
            this.shader = shader;
            kernel = shader.FindKernel("SkinVertices");
            shader.GetKernelThreadGroupSizes(kernel, out threadsX, out var threadsY, out var threadsZ);
            if (threadsX < 1 || threadsY != 1 || threadsZ != 1)
            {
                throw new ArgumentException(
                    "GPU skinning kernel must use a positive one-dimensional workgroup.",
                    nameof(shader));
            }
        }

        public bool IsAvailable
        {
            get
            {
                return !disposed
                    && shader != null
                    && SystemInfo.supportsComputeShaders
                    && SystemInfo.graphicsDeviceType != GraphicsDeviceType.Null;
            }
        }

        public int VertexCapacity { get { return vertexCapacity; } }
        public int BoneCapacity { get { return boneCapacity; } }
        public int BufferAllocationCount { get { return bufferAllocationCount; } }

        /// <summary>
        /// Requests an explicit completion readback for diagnostics and
        /// benchmarks. The caller must wait for this request before asking
        /// the stream to resize or reuse its output buffer.
        /// </summary>
        public bool TryRequestReadback(
            out AsyncGPUReadbackRequest request,
            out string failureReason)
        {
            request = default;
            failureReason = string.Empty;
            if (!IsAvailable || outputBuffer == null)
            {
                failureReason = "gpu_skinning_output_unavailable";
                return false;
            }
            try
            {
                request = AsyncGPUReadback.Request(outputBuffer);
                return true;
            }
            catch (Exception error)
            {
                failureReason = "unity_compute_readback_request_exception:"
                    + error.GetType().Name + ":" + error.Message;
                return false;
            }
        }

        /// <summary>
        /// Updates reusable staging buffers, dispatches the compute kernel and
        /// binds the output buffer to the supplied property block.
        /// </summary>
        public bool TryDispatchAndBind(
            JadrenGpuSkinningVertex[] vertices,
            Matrix4x4[] boneMatrices,
            MaterialPropertyBlock propertyBlock,
            out string failureReason)
        {
            return TryDispatchAndBind(
                vertices,
                boneMatrices,
                propertyBlock,
                0,
                0,
                true,
                out failureReason);
        }

        /// <summary>
        /// Dispatches a crowd layout where vertices are repeated per agent and
        /// bone matrices are packed as <c>[agent][bone]</c>. A zero layout
        /// keeps the original single-character contract. The per-agent
        /// offsets are consumed by the compute and proxy shaders, so each
        /// instance can publish an independent pose in one dispatch.
        /// </summary>
        public bool TryDispatchAndBind(
            JadrenGpuSkinningVertex[] vertices,
            Matrix4x4[] boneMatrices,
            MaterialPropertyBlock propertyBlock,
            int verticesPerAgent,
            int bonesPerAgent,
            out string failureReason)
        {
            return TryDispatchAndBind(
                vertices,
                boneMatrices,
                propertyBlock,
                verticesPerAgent,
                bonesPerAgent,
                true,
                out failureReason);
        }

        /// <summary>
        /// Dispatches the crowd layout with an explicit input-validation
        /// switch. Renderer hosts validate immutable arrays during setup and
        /// pass <c>false</c> here to keep per-frame work bounded.
        /// </summary>
        public bool TryDispatchAndBind(
            JadrenGpuSkinningVertex[] vertices,
            Matrix4x4[] boneMatrices,
            MaterialPropertyBlock propertyBlock,
            int verticesPerAgent,
            int bonesPerAgent,
            bool validateInputs,
            out string failureReason)
        {
            var agentCount = verticesPerAgent > 0
                && vertices != null
                ? vertices.Length / verticesPerAgent
                : 1;
            return TryDispatchAndBindCore(
                vertices,
                boneMatrices,
                propertyBlock,
                verticesPerAgent,
                bonesPerAgent,
                agentCount,
                false,
                validateInputs,
                out failureReason);
        }

        /// <summary>
        /// Dispatches a crowd using one shared mesh vertex array and matrices
        /// packed as <c>[agent][bone]</c>. The compute output remains expanded
        /// per agent for the procedural instanced draw, but the input geometry
        /// is uploaded only once.
        /// </summary>
        public bool TryDispatchAndBindSharedVertices(
            JadrenGpuSkinningVertex[] vertices,
            Matrix4x4[] boneMatrices,
            MaterialPropertyBlock propertyBlock,
            int agentCount,
            int bonesPerAgent,
            bool validateInputs,
            out string failureReason)
        {
            return TryDispatchAndBindCore(
                vertices,
                boneMatrices,
                propertyBlock,
                vertices == null ? 0 : vertices.Length,
                bonesPerAgent,
                agentCount,
                true,
                validateInputs,
                out failureReason);
        }

        /// <summary>
        /// Dispatches the shared-mesh crowd while reusing a caller-owned GPU
        /// bone matrix buffer. This is the GPU animation route: animation and
        /// hierarchy evaluation already wrote the matrices, so no per-frame
        /// Matrix4x4[] upload is performed here.
        /// </summary>
        public bool TryDispatchAndBindSharedVertices(
            JadrenGpuSkinningVertex[] vertices,
            ComputeBuffer externalBoneMatrices,
            int externalBoneCount,
            MaterialPropertyBlock propertyBlock,
            int agentCount,
            int bonesPerAgent,
            bool validateInputs,
            out string failureReason)
        {
            failureReason = string.Empty;
            if (!IsAvailable)
            {
                failureReason = "unity_compute_unavailable";
                return false;
            }
            if (propertyBlock == null)
            {
                failureReason = "property_block_missing";
                return false;
            }
            if (vertices == null || vertices.Length < 1)
            {
                failureReason = "vertex_input_missing";
                return false;
            }
            if (externalBoneMatrices == null || externalBoneCount < 1
                || bonesPerAgent < 1 || agentCount < 1
                || (long)externalBoneCount != (long)agentCount * bonesPerAgent)
            {
                failureReason = "external_bone_layout_invalid";
                return false;
            }
            if (validateInputs && !TryValidateExternalVertices(
                vertices,
                bonesPerAgent,
                out failureReason))
            {
                return false;
            }

            var outputVertexCountLong = (long)vertices.Length * agentCount;
            if (outputVertexCountLong < 1 || outputVertexCountLong > int.MaxValue)
            {
                failureReason = "crowd_output_count_overflow";
                return false;
            }

            try
            {
                EnsureVertexAndOutputBuffers(vertices.Length, (int)outputVertexCountLong);
                vertexBuffer.SetData(vertices, 0, 0, vertices.Length);
                shader.SetBuffer(kernel, "Vertices", vertexBuffer);
                shader.SetBuffer(kernel, "BoneMatrices", externalBoneMatrices);
                shader.SetBuffer(kernel, "OutputPosition", outputBuffer);
                shader.SetInt("JadrenVertexCount", (int)outputVertexCountLong);
                shader.SetInt("JadrenInputVertexCount", vertices.Length);
                shader.SetInt("JadrenBoneCount", externalBoneCount);
                shader.SetInt("JadrenVerticesPerAgent", vertices.Length);
                shader.SetInt("JadrenBonesPerAgent", bonesPerAgent);
                shader.SetInt("JadrenAgentCount", agentCount);
                DispatchLinear((int)outputVertexCountLong);
                propertyBlock.SetBuffer("_JadrenGpuPositions", outputBuffer);
                propertyBlock.SetInt("_JadrenGpuVertexCount", (int)outputVertexCountLong);
                propertyBlock.SetBuffer("_JadrenGpuSkinningVertices", vertexBuffer);
                propertyBlock.SetBuffer("_JadrenGpuSkinningBoneMatrices", externalBoneMatrices);
                propertyBlock.SetInt("_JadrenGpuSkinningInputVertexCount", vertices.Length);
                propertyBlock.SetInt("_JadrenGpuSkinningBoneCount", externalBoneCount);
                propertyBlock.SetInt("_JadrenGpuCrowdVerticesPerInstance", vertices.Length);
                propertyBlock.SetInt("_JadrenGpuCrowdBonesPerInstance", bonesPerAgent);
                propertyBlock.SetInt("_JadrenGpuCrowdInstanceCount", agentCount);
                return true;
            }
            catch (Exception error)
            {
                failureReason = "unity_compute_external_stream_exception:"
                    + error.GetType().Name + ":" + error.Message;
                return false;
            }
        }

        private bool TryDispatchAndBindCore(
            JadrenGpuSkinningVertex[] vertices,
            Matrix4x4[] boneMatrices,
            MaterialPropertyBlock propertyBlock,
            int verticesPerAgent,
            int bonesPerAgent,
            int agentCount,
            bool sharedVertices,
            bool validateInputs,
            out string failureReason)
        {
            failureReason = string.Empty;
            if (!IsAvailable)
            {
                failureReason = "unity_compute_unavailable";
                return false;
            }
            if (propertyBlock == null)
            {
                failureReason = "property_block_missing";
                return false;
            }
            if (verticesPerAgent < 0 || bonesPerAgent < 0)
            {
                failureReason = "crowd_layout_negative";
                return false;
            }
            if (agentCount < 1)
            {
                failureReason = "crowd_agent_count_invalid";
                return false;
            }
            if ((verticesPerAgent == 0) != (bonesPerAgent == 0))
            {
                failureReason = "crowd_layout_incomplete";
                return false;
            }
            if (vertices == null || boneMatrices == null)
            {
                failureReason = vertices == null
                    ? "vertex_input_missing"
                    : "bone_matrix_input_missing";
                return false;
            }
            if (verticesPerAgent > 0)
            {
                if (vertices.Length < 1
                    || (sharedVertices
                        ? vertices.Length != verticesPerAgent
                        : vertices.Length % verticesPerAgent != 0))
                {
                    failureReason = "crowd_vertices_not_agent_multiple";
                    return false;
                }
                var expectedBoneCount = (long)agentCount * bonesPerAgent;
                if (bonesPerAgent < 1 || expectedBoneCount != boneMatrices.Length)
                {
                    failureReason = "crowd_bones_not_agent_multiple";
                    return false;
                }
            }

            var outputVertexCountLong = (long)vertices.Length * (sharedVertices ? agentCount : 1);
            if (outputVertexCountLong < 1 || outputVertexCountLong > int.MaxValue)
            {
                failureReason = "crowd_output_count_overflow";
                return false;
            }
            var outputVertexCount = (int)outputVertexCountLong;
            if (validateInputs
                && !JadrenAnimationGpuSkinningDispatcher.TryValidateInputs(
                    vertices,
                    boneMatrices,
                    out failureReason))
            {
                return false;
            }

            try
            {
                EnsureBuffers(vertices.Length, boneMatrices.Length, outputVertexCount);
                vertexBuffer.SetData(vertices, 0, 0, vertices.Length);
                matrixBuffer.SetData(boneMatrices, 0, 0, boneMatrices.Length);
                shader.SetBuffer(kernel, "Vertices", vertexBuffer);
                shader.SetBuffer(kernel, "BoneMatrices", matrixBuffer);
                shader.SetBuffer(kernel, "OutputPosition", outputBuffer);
                shader.SetInt("JadrenVertexCount", outputVertexCount);
                shader.SetInt("JadrenInputVertexCount", vertices.Length);
                shader.SetInt("JadrenBoneCount", boneMatrices.Length);
                shader.SetInt("JadrenVerticesPerAgent", verticesPerAgent);
                shader.SetInt("JadrenBonesPerAgent", bonesPerAgent);
                shader.SetInt("JadrenAgentCount", agentCount);
                DispatchLinear(outputVertexCount);
                propertyBlock.SetBuffer("_JadrenGpuPositions", outputBuffer);
                propertyBlock.SetInt("_JadrenGpuVertexCount", outputVertexCount);
                // The proxy shader may consume the same reusable inputs to
                // reconstruct skinned normals/material lighting without
                // changing the versioned 44-byte vertex ABI or adding a
                // second CPU-side staging path.
                propertyBlock.SetBuffer("_JadrenGpuSkinningVertices", vertexBuffer);
                propertyBlock.SetBuffer("_JadrenGpuSkinningBoneMatrices", matrixBuffer);
                propertyBlock.SetInt("_JadrenGpuSkinningInputVertexCount", vertices.Length);
                propertyBlock.SetInt("_JadrenGpuSkinningBoneCount", boneMatrices.Length);
                propertyBlock.SetInt("_JadrenGpuCrowdVerticesPerInstance", verticesPerAgent);
                propertyBlock.SetInt("_JadrenGpuCrowdBonesPerInstance", bonesPerAgent);
                propertyBlock.SetInt("_JadrenGpuCrowdInstanceCount", agentCount);
                return true;
            }
            catch (Exception error)
            {
                failureReason = "unity_compute_stream_exception:"
                    + error.GetType().Name + ":" + error.Message;
                return false;
            }
        }

        public void Dispose()
        {
            if (disposed)
            {
                return;
            }
            disposed = true;
            Release(vertexBuffer);
            Release(matrixBuffer);
            Release(outputBuffer);
            vertexBuffer = null;
            matrixBuffer = null;
            outputBuffer = null;
            vertexCapacity = 0;
            boneCapacity = 0;
            outputCapacity = 0;
            GC.SuppressFinalize(this);
        }

        private void DispatchLinear(int elementCount)
        {
            var groupsTotal = ((long)elementCount + (long)threadsX - 1L) / (long)threadsX;
            const int maxGroupsPerDimension = 65535;
            var groupsX = (int)Math.Min(groupsTotal, maxGroupsPerDimension);
            var groupsY = (int)((groupsTotal + groupsX - 1L) / groupsX);
            shader.SetInt("JadrenSkinningGroupStride", checked(groupsX * (int)threadsX));
            shader.Dispatch(kernel, groupsX, groupsY, 1);
            // D3D11/URP can otherwise let the procedural vertex pass observe
            // the previous UAV contents. Publish the compute result before
            // binding it as a vertex-stage SRV; this is the correctness gate
            // for Play-mode visibility, not a diagnostic readback.
            var fence = Graphics.CreateGraphicsFence(
                GraphicsFenceType.AsyncQueueSynchronisation,
                SynchronisationStageFlags.ComputeProcessing);
            Graphics.WaitOnAsyncGraphicsFence(fence);
        }

        private void EnsureBuffers(
            int requiredVertexCount,
            int requiredBoneCount,
            int requiredOutputCount)
        {
            if (vertexBuffer != null && vertexCapacity >= requiredVertexCount
                && matrixBuffer != null && boneCapacity >= requiredBoneCount
                && outputBuffer != null && outputCapacity >= requiredOutputCount)
            {
                return;
            }

            Release(vertexBuffer);
            Release(matrixBuffer);
            Release(outputBuffer);
            vertexCapacity = Math.Max(requiredVertexCount, Math.Max(1, vertexCapacity * 2));
            boneCapacity = Math.Max(requiredBoneCount, Math.Max(1, boneCapacity * 2));
            outputCapacity = Math.Max(requiredOutputCount, Math.Max(1, outputCapacity * 2));
            vertexBuffer = new ComputeBuffer(
                vertexCapacity,
                JadrenGpuSkinningVertex.StrideBytes,
                ComputeBufferType.Structured);
            matrixBuffer = new ComputeBuffer(
                boneCapacity,
                sizeof(float) * 16,
                ComputeBufferType.Structured);
            outputBuffer = new ComputeBuffer(
                outputCapacity,
                sizeof(float) * 3,
                ComputeBufferType.Structured);
            bufferAllocationCount++;
        }

        private void EnsureVertexAndOutputBuffers(
            int requiredVertexCount,
            int requiredOutputCount)
        {
            if (vertexBuffer != null && vertexCapacity >= requiredVertexCount
                && outputBuffer != null && outputCapacity >= requiredOutputCount)
            {
                return;
            }

            Release(vertexBuffer);
            Release(matrixBuffer);
            Release(outputBuffer);
            vertexCapacity = Math.Max(requiredVertexCount, Math.Max(1, vertexCapacity * 2));
            boneCapacity = 0;
            outputCapacity = Math.Max(requiredOutputCount, Math.Max(1, outputCapacity * 2));
            vertexBuffer = new ComputeBuffer(
                vertexCapacity,
                JadrenGpuSkinningVertex.StrideBytes,
                ComputeBufferType.Structured);
            outputBuffer = new ComputeBuffer(
                outputCapacity,
                sizeof(float) * 3,
                ComputeBufferType.Structured);
            matrixBuffer = null;
            bufferAllocationCount++;
        }

        private static bool TryValidateExternalVertices(
            JadrenGpuSkinningVertex[] vertices,
            int bonesPerAgent,
            out string failureReason)
        {
            failureReason = string.Empty;
            for (var vertexIndex = 0; vertexIndex < vertices.Length; vertexIndex++)
            {
                if (!vertices[vertexIndex].TryValidate(bonesPerAgent, out var vertexReason))
                {
                    failureReason = "vertex_input_invalid:" + vertexIndex + ":" + vertexReason;
                    return false;
                }
            }
            return true;
        }

        private static void Release(ComputeBuffer buffer)
        {
            if (buffer != null)
            {
                buffer.Release();
            }
        }
    }
}
