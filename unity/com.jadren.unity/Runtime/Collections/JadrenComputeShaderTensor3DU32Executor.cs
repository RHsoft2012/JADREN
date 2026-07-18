using System;
using Unity.Collections;
using UnityEngine;
using UnityEngine.Rendering;

namespace Jadren.Unity
{
    /// <summary>
    /// Unity ComputeShader implementation of the Jadren 3D affine-stride
    /// u32 write contract. The adapter owns a temporary GPU buffer and uses
    /// explicit readback; the caller's NativeArray remains borrowed only.
    /// </summary>
    public sealed class JadrenComputeShaderTensor3DU32Executor : IJadrenGpuTensor3DU32Executor
    {
        private readonly ComputeShader shader;
        private readonly int kernel;
        private readonly string outputBufferName;
        private readonly string widthPropertyName;
        private readonly string heightPropertyName;
        private readonly string depthPropertyName;
        private readonly string strideXPropertyName;
        private readonly string strideYPropertyName;
        private readonly string strideZPropertyName;
        private readonly string capacityPropertyName;
        private readonly string valuePropertyName;
        private readonly uint threadsX;
        private readonly uint threadsY;
        private readonly uint threadsZ;

        public JadrenComputeShaderTensor3DU32Executor(
            ComputeShader shader,
            string kernelName = "AffineWrite",
            string outputBufferName = "Output",
            string widthPropertyName = "JadrenWidth",
            string heightPropertyName = "JadrenHeight",
            string depthPropertyName = "JadrenDepth",
            string strideXPropertyName = "JadrenStrideX",
            string strideYPropertyName = "JadrenStrideY",
            string strideZPropertyName = "JadrenStrideZ",
            string capacityPropertyName = "JadrenCapacity",
            string valuePropertyName = "JadrenValue")
        {
            if (shader == null)
            {
                throw new ArgumentNullException(nameof(shader));
            }
            if (string.IsNullOrEmpty(kernelName))
            {
                throw new ArgumentException("Kernel name must not be empty.", nameof(kernelName));
            }
            if (string.IsNullOrEmpty(outputBufferName))
            {
                throw new ArgumentException("Output buffer name must not be empty.", nameof(outputBufferName));
            }

            this.shader = shader;
            kernel = shader.FindKernel(kernelName);
            shader.GetKernelThreadGroupSizes(kernel, out threadsX, out threadsY, out threadsZ);
            if (threadsX < 1 || threadsY < 1 || threadsZ < 1)
            {
                throw new ArgumentException("The ComputeShader kernel must use positive 3D workgroup dimensions.", nameof(shader));
            }
            this.outputBufferName = outputBufferName;
            this.widthPropertyName = widthPropertyName;
            this.heightPropertyName = heightPropertyName;
            this.depthPropertyName = depthPropertyName;
            this.strideXPropertyName = strideXPropertyName;
            this.strideYPropertyName = strideYPropertyName;
            this.strideZPropertyName = strideZPropertyName;
            this.capacityPropertyName = capacityPropertyName;
            this.valuePropertyName = valuePropertyName;
        }

        public bool IsAvailable => shader != null && SystemInfo.supportsComputeShaders;

        public bool TryDispatch(
            NativeArray<uint> output,
            JadrenTensor3DLayout layout,
            uint value,
            JadrenGpuDispatchOptions options,
            out string failureReason)
        {
            failureReason = string.Empty;
            if (!IsAvailable)
            {
                failureReason = "unity_compute_unavailable";
                return false;
            }
            if (!output.IsCreated || output.Length != layout.Capacity)
            {
                failureReason = "native_array_capacity_invalid";
                return false;
            }
            var expectedWorkgroupSize = checked((int)(threadsX * threadsY * threadsZ));
            if (options.WorkgroupSize != expectedWorkgroupSize)
            {
                failureReason = "workgroup_size_mismatch";
                return false;
            }

            ComputeBuffer outputBuffer = null;
            try
            {
                outputBuffer = new ComputeBuffer(layout.Capacity, sizeof(uint), ComputeBufferType.Structured);
                outputBuffer.SetData(output);
                shader.SetBuffer(kernel, outputBufferName, outputBuffer);
                shader.SetInt(widthPropertyName, layout.Width);
                shader.SetInt(heightPropertyName, layout.Height);
                shader.SetInt(depthPropertyName, layout.Depth);
                shader.SetInt(strideXPropertyName, layout.StrideX);
                shader.SetInt(strideYPropertyName, layout.StrideY);
                shader.SetInt(strideZPropertyName, layout.StrideZ);
                shader.SetInt(capacityPropertyName, layout.Capacity);
                shader.SetInt(valuePropertyName, unchecked((int)value));
                var groupsX = (layout.Width + (int)threadsX - 1) / (int)threadsX;
                var groupsY = (layout.Height + (int)threadsY - 1) / (int)threadsY;
                var groupsZ = (layout.Depth + (int)threadsZ - 1) / (int)threadsZ;
                shader.Dispatch(kernel, groupsX, groupsY, groupsZ);
                var readback = AsyncGPUReadback.RequestIntoNativeArray(
                    ref output,
                    outputBuffer,
                    (Action<AsyncGPUReadbackRequest>)null);
                readback.WaitForCompletion();
                if (readback.hasError)
                {
                    failureReason = "unity_compute_readback_error";
                    return false;
                }
                return true;
            }
            catch (Exception error)
            {
                failureReason = "unity_compute_3d_exception:" + error.GetType().Name + ":" + error.Message;
                return false;
            }
            finally
            {
                if (outputBuffer != null)
                {
                    outputBuffer.Release();
                }
            }
        }
    }
}
