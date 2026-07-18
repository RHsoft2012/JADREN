using System;
using UnityEngine;
using UnityEngine.Rendering;

namespace Jadren.Animation
{
    /// <summary>
    /// Main-thread dispatcher for the first GPU skinning contract. It stages
    /// caller-owned vertices and bone matrices, then returns a frame-spanning
    /// readback handle. It does not inspect Mesh, Renderer or Animator state.
    /// </summary>
    public sealed class JadrenAnimationGpuSkinningDispatcher : IDisposable
    {
        private readonly ComputeShader shader;
        private readonly int kernel;
        private readonly uint threadsX;

        public JadrenAnimationGpuSkinningDispatcher(ComputeShader shader)
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
                return shader != null
                    && SystemInfo.supportsComputeShaders
                    && SystemInfo.graphicsDeviceType != GraphicsDeviceType.Null;
            }
        }

        public bool TryDispatch(
            JadrenGpuSkinningVertex[] vertices,
            Matrix4x4[] boneMatrices,
            out JadrenAnimationGpuSkinningDispatch dispatch,
            out string failureReason)
        {
            return TryDispatchInternal(
                vertices,
                boneMatrices,
                true,
                out dispatch,
                out failureReason);
        }

        /// <summary>
        /// Dispatches the same validated skinning workload but retains the
        /// output buffer for a renderer material binding. No CPU readback is
        /// requested; the returned handle must stay alive until rendering has
        /// consumed the buffer and is then disposed by the main-thread host.
        /// </summary>
        public bool TryDispatchToGraphics(
            JadrenGpuSkinningVertex[] vertices,
            Matrix4x4[] boneMatrices,
            out JadrenAnimationGpuSkinningDispatch dispatch,
            out string failureReason)
        {
            return TryDispatchInternal(
                vertices,
                boneMatrices,
                false,
                out dispatch,
                out failureReason);
        }

        private bool TryDispatchInternal(
            JadrenGpuSkinningVertex[] vertices,
            Matrix4x4[] boneMatrices,
            bool requestReadback,
            out JadrenAnimationGpuSkinningDispatch dispatch,
            out string failureReason)
        {
            dispatch = null;
            failureReason = string.Empty;
            if (!IsAvailable)
            {
                failureReason = "unity_compute_unavailable";
                return false;
            }
            if (!TryValidateInputs(vertices, boneMatrices, out failureReason))
            {
                return false;
            }

            ComputeBuffer vertexBuffer = null;
            ComputeBuffer matrixBuffer = null;
            ComputeBuffer outputBuffer = null;
            try
            {
                vertexBuffer = new ComputeBuffer(
                    vertices.Length,
                    JadrenGpuSkinningVertex.StrideBytes,
                    ComputeBufferType.Structured);
                matrixBuffer = new ComputeBuffer(
                    boneMatrices.Length,
                    sizeof(float) * 16,
                    ComputeBufferType.Structured);
                outputBuffer = new ComputeBuffer(
                    vertices.Length,
                    sizeof(float) * 3,
                    ComputeBufferType.Structured);
                vertexBuffer.SetData(vertices);
                matrixBuffer.SetData(boneMatrices);
                shader.SetBuffer(kernel, "Vertices", vertexBuffer);
                shader.SetBuffer(kernel, "BoneMatrices", matrixBuffer);
                shader.SetBuffer(kernel, "OutputPosition", outputBuffer);
                shader.SetInt("JadrenVertexCount", vertices.Length);
                shader.SetInt("JadrenBoneCount", boneMatrices.Length);
                var groups = (vertices.Length + (int)threadsX - 1) / (int)threadsX;
                shader.Dispatch(kernel, groups, 1, 1);

                var readback = requestReadback
                    ? AsyncGPUReadback.Request(outputBuffer)
                    : default;
                dispatch = new JadrenAnimationGpuSkinningDispatch(
                    new JadrenAnimationGpuSkinningResult(vertices.Length),
                    readback,
                    requestReadback,
                    vertexBuffer,
                    matrixBuffer,
                    outputBuffer,
                    vertices.Length);
                vertexBuffer = null;
                matrixBuffer = null;
                outputBuffer = null;
                return true;
            }
            catch (Exception error)
            {
                failureReason = "unity_compute_exception:" + error.GetType().Name + ":" + error.Message;
                return false;
            }
            finally
            {
                Release(vertexBuffer);
                Release(matrixBuffer);
                Release(outputBuffer);
            }
        }

        public void Dispose()
        {
            GC.SuppressFinalize(this);
        }

        internal static bool TryValidateInputs(
            JadrenGpuSkinningVertex[] vertices,
            Matrix4x4[] boneMatrices,
            out string failureReason)
        {
            failureReason = string.Empty;
            if (vertices == null || vertices.Length < 1)
            {
                failureReason = "vertex_input_missing";
                return false;
            }
            if (boneMatrices == null || boneMatrices.Length < 1)
            {
                failureReason = "bone_matrix_input_missing";
                return false;
            }
            for (var matrixIndex = 0; matrixIndex < boneMatrices.Length; matrixIndex++)
            {
                for (var row = 0; row < 4; row++)
                {
                    for (var column = 0; column < 4; column++)
                    {
                        var value = boneMatrices[matrixIndex][row, column];
                        if (float.IsNaN(value) || float.IsInfinity(value))
                        {
                            failureReason = "bone_matrix_non_finite:" + matrixIndex;
                            return false;
                        }
                    }
                }
            }
            for (var vertexIndex = 0; vertexIndex < vertices.Length; vertexIndex++)
            {
                if (!vertices[vertexIndex].TryValidate(boneMatrices.Length, out var vertexReason))
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

    /// <summary>Frame-spanning GPU skinning readback and buffer lifetime.</summary>
    public sealed class JadrenAnimationGpuSkinningDispatch : IDisposable
    {
        private readonly JadrenAnimationGpuSkinningResult result;
        private readonly AsyncGPUReadbackRequest readback;
        private readonly bool hasReadback;
        private readonly ComputeBuffer vertexBuffer;
        private readonly ComputeBuffer matrixBuffer;
        private readonly ComputeBuffer outputBuffer;
        private readonly int vertexCount;
        private bool completed;

        internal JadrenAnimationGpuSkinningDispatch(
            JadrenAnimationGpuSkinningResult result,
            AsyncGPUReadbackRequest readback,
            bool hasReadback,
            ComputeBuffer vertexBuffer,
            ComputeBuffer matrixBuffer,
            ComputeBuffer outputBuffer,
            int vertexCount)
        {
            this.result = result;
            this.readback = readback;
            this.hasReadback = hasReadback;
            this.vertexBuffer = vertexBuffer;
            this.matrixBuffer = matrixBuffer;
            this.outputBuffer = outputBuffer;
            this.vertexCount = vertexCount;
        }

        public JadrenAnimationGpuSkinningResult Result { get { return result; } }
        public bool HasReadback { get { return hasReadback; } }
        public int VertexCount { get { return vertexCount; } }
        public bool IsDone { get { return completed || (hasReadback && readback.done); } }

        /// <summary>Bind the live GPU output to a material property block.</summary>
        public bool BindOutput(MaterialPropertyBlock propertyBlock)
        {
            if (completed || propertyBlock == null || hasReadback)
            {
                return false;
            }
            propertyBlock.SetBuffer("_JadrenGpuPositions", outputBuffer);
            propertyBlock.SetInt("_JadrenGpuVertexCount", vertexCount);
            return true;
        }

        public bool TryComplete(out JadrenAnimationGpuSkinningResult completedResult)
        {
            if (!IsDone)
            {
                completedResult = null;
                return false;
            }
            completedResult = Complete();
            return true;
        }

        public JadrenAnimationGpuSkinningResult Complete()
        {
            if (completed)
            {
                return result;
            }
            if (!hasReadback)
            {
                result.TryPublishFailure("gpu_skinning_graphics_dispatch_has_no_readback");
                Release(vertexBuffer);
                Release(matrixBuffer);
                Release(outputBuffer);
                completed = true;
                return result;
            }
            try
            {
                readback.WaitForCompletion();
                if (readback.hasError)
                {
                    result.TryPublishFailure("unity_compute_readback_error");
                }
                else
                {
                    var data = readback.GetData<Vector3>();
                    var positions = new Vector3[vertexCount];
                    for (var vertexIndex = 0; vertexIndex < vertexCount; vertexIndex++)
                    {
                        positions[vertexIndex] = data[vertexIndex];
                    }
                    if (!result.TryPublishCompleted(positions))
                    {
                        result.TryPublishFailure("gpu_skinning_result_rejected");
                    }
                }
            }
            catch (Exception error)
            {
                result.TryPublishFailure(
                    "unity_compute_completion_exception:" + error.GetType().Name);
            }
            finally
            {
                Release(vertexBuffer);
                Release(matrixBuffer);
                Release(outputBuffer);
                completed = true;
            }
            return result;
        }

        public void Dispose()
        {
            Complete();
            GC.SuppressFinalize(this);
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
