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
            if (!JadrenAnimationGpuSkinningDispatcher.TryValidateInputs(
                    vertices,
                    boneMatrices,
                    out failureReason))
            {
                return false;
            }

            try
            {
                EnsureBuffers(vertices.Length, boneMatrices.Length);
                vertexBuffer.SetData(vertices, 0, 0, vertices.Length);
                matrixBuffer.SetData(boneMatrices, 0, 0, boneMatrices.Length);
                shader.SetBuffer(kernel, "Vertices", vertexBuffer);
                shader.SetBuffer(kernel, "BoneMatrices", matrixBuffer);
                shader.SetBuffer(kernel, "OutputPosition", outputBuffer);
                shader.SetInt("JadrenVertexCount", vertices.Length);
                shader.SetInt("JadrenBoneCount", boneMatrices.Length);
                var groups = (vertices.Length + (int)threadsX - 1) / (int)threadsX;
                shader.Dispatch(kernel, groups, 1, 1);
                propertyBlock.SetBuffer("_JadrenGpuPositions", outputBuffer);
                propertyBlock.SetInt("_JadrenGpuVertexCount", vertices.Length);
                // The proxy shader may consume the same reusable inputs to
                // reconstruct skinned normals/material lighting without
                // changing the versioned 44-byte vertex ABI or adding a
                // second CPU-side staging path.
                propertyBlock.SetBuffer("_JadrenGpuSkinningVertices", vertexBuffer);
                propertyBlock.SetBuffer("_JadrenGpuSkinningBoneMatrices", matrixBuffer);
                propertyBlock.SetInt("_JadrenGpuSkinningBoneCount", boneMatrices.Length);
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
            GC.SuppressFinalize(this);
        }

        private void EnsureBuffers(int requiredVertexCount, int requiredBoneCount)
        {
            if (vertexBuffer != null && vertexCapacity >= requiredVertexCount
                && matrixBuffer != null && boneCapacity >= requiredBoneCount
                && outputBuffer != null)
            {
                return;
            }

            Release(vertexBuffer);
            Release(matrixBuffer);
            Release(outputBuffer);
            vertexCapacity = Math.Max(requiredVertexCount, Math.Max(1, vertexCapacity * 2));
            boneCapacity = Math.Max(requiredBoneCount, Math.Max(1, boneCapacity * 2));
            vertexBuffer = new ComputeBuffer(
                vertexCapacity,
                JadrenGpuSkinningVertex.StrideBytes,
                ComputeBufferType.Structured);
            matrixBuffer = new ComputeBuffer(
                boneCapacity,
                sizeof(float) * 16,
                ComputeBufferType.Structured);
            outputBuffer = new ComputeBuffer(
                vertexCapacity,
                sizeof(float) * 3,
                ComputeBufferType.Structured);
            bufferAllocationCount++;
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
