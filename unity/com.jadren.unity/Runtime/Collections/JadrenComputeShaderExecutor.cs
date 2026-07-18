using System;
using Unity.Collections;
using Unity.Collections.LowLevel.Unsafe;
using UnityEngine;
using UnityEngine.Rendering;

namespace Jadren.Unity
{
    /// <summary>
    /// Synchronous Unity ComputeShader adapter for one structured input/output
    /// pair. Buffer upload/readback is explicit; it is not a zero-copy claim.
    /// The adapter owns temporary ComputeBuffers, never the NativeArrays.
    /// </summary>
    public sealed class JadrenComputeShaderExecutor<T> : IJadrenGpuNativeArrayExecutor<T>, IJadrenGpuAsyncNativeArrayExecutor<T>
        where T : unmanaged
    {
        private readonly ComputeShader shader;
        private readonly int kernel;
        private readonly string inputBufferName;
        private readonly string outputBufferName;
        private readonly string countPropertyName;
        private readonly uint threadsX;

        public JadrenComputeShaderExecutor(
            ComputeShader shader,
            string kernelName,
            string inputBufferName = "Input",
            string outputBufferName = "Output",
            string countPropertyName = "JadrenElementCount")
        {
            if (shader == null)
            {
                throw new ArgumentNullException(nameof(shader));
            }
            if (string.IsNullOrEmpty(kernelName))
            {
                throw new ArgumentException("Kernel name must not be empty.", nameof(kernelName));
            }
            if (string.IsNullOrEmpty(inputBufferName))
            {
                throw new ArgumentException("Input buffer name must not be empty.", nameof(inputBufferName));
            }
            if (string.IsNullOrEmpty(outputBufferName))
            {
                throw new ArgumentException("Output buffer name must not be empty.", nameof(outputBufferName));
            }
            if (string.IsNullOrEmpty(countPropertyName))
            {
                throw new ArgumentException("Count property name must not be empty.", nameof(countPropertyName));
            }

            this.shader = shader;
            kernel = shader.FindKernel(kernelName);
            shader.GetKernelThreadGroupSizes(kernel, out threadsX, out var threadsY, out var threadsZ);
            if (threadsX < 1 || threadsY != 1 || threadsZ != 1)
            {
                throw new ArgumentException("The ComputeShader kernel must use a one-dimensional workgroup.", nameof(shader));
            }
            this.inputBufferName = inputBufferName;
            this.outputBufferName = outputBufferName;
            this.countPropertyName = countPropertyName;
        }

        public bool IsAvailable => shader != null && SystemInfo.supportsComputeShaders;

        public bool TryDispatch(
            NativeArray<T> input,
            NativeArray<T> output,
            int elementCount,
            JadrenGpuDispatchOptions options,
            out string failureReason)
        {
            failureReason = string.Empty;
            if (!IsAvailable)
            {
                failureReason = "unity_compute_unavailable";
                return false;
            }
            if (!input.IsCreated || !output.IsCreated || input.Length < elementCount || output.Length < elementCount)
            {
                failureReason = "native_array_length_invalid";
                return false;
            }
            if (elementCount < 1)
            {
                failureReason = "empty_dispatch";
                return false;
            }
            if (options.WorkgroupSize != threadsX)
            {
                failureReason = "workgroup_size_mismatch";
                return false;
            }

            var elementSize = UnsafeUtility.SizeOf<T>();
            if (elementSize < 1)
            {
                failureReason = "element_layout_invalid";
                return false;
            }

            ComputeBuffer inputBuffer = null;
            ComputeBuffer outputBuffer = null;
            try
            {
                inputBuffer = new ComputeBuffer(elementCount, elementSize, ComputeBufferType.Structured);
                outputBuffer = new ComputeBuffer(elementCount, elementSize, ComputeBufferType.Structured);
                inputBuffer.SetData(input, 0, 0, elementCount);
                outputBuffer.SetData(output, 0, 0, elementCount);
                shader.SetBuffer(kernel, inputBufferName, inputBuffer);
                shader.SetBuffer(kernel, outputBufferName, outputBuffer);
                shader.SetInt(countPropertyName, elementCount);
                var groupCount = (elementCount + (int)threadsX - 1) / (int)threadsX;
                shader.Dispatch(kernel, groupCount, 1, 1);
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
                failureReason = "unity_compute_exception:" + error.GetType().Name + ":" + error.Message;
                return false;
            }
            finally
            {
                if (inputBuffer != null)
                {
                    inputBuffer.Release();
                }
                if (outputBuffer != null)
                {
                    outputBuffer.Release();
                }
            }
        }

        public bool TryDispatchAsync(
            string kernelName,
            JadrenNativeArrayAsyncLease<T> input,
            JadrenNativeArrayAsyncLease<T> output,
            int elementCount,
            JadrenGpuDispatchOptions options,
            out JadrenGpuAsyncDispatch<T> dispatch,
            out string failureReason)
        {
            dispatch = null;
            failureReason = string.Empty;
            if (!IsAvailable)
            {
                failureReason = "unity_compute_unavailable";
                return false;
            }
            if (input == null || output == null || input.Length < elementCount || output.Length < elementCount)
            {
                failureReason = "async_lease_invalid";
                return false;
            }
            if (elementCount < 1)
            {
                failureReason = "empty_dispatch";
                return false;
            }
            if (options.WorkgroupSize != threadsX)
            {
                failureReason = "workgroup_size_mismatch";
                return false;
            }

            var elementSize = UnsafeUtility.SizeOf<T>();
            if (elementSize < 1)
            {
                failureReason = "element_layout_invalid";
                return false;
            }

            ComputeBuffer inputBuffer = null;
            ComputeBuffer outputBuffer = null;
            try
            {
                inputBuffer = new ComputeBuffer(elementCount, elementSize, ComputeBufferType.Structured);
                outputBuffer = new ComputeBuffer(elementCount, elementSize, ComputeBufferType.Structured);
                inputBuffer.SetData(input.BorrowedArray, 0, 0, elementCount);
                outputBuffer.SetData(output.BorrowedArray, 0, 0, elementCount);
                shader.SetBuffer(kernel, inputBufferName, inputBuffer);
                shader.SetBuffer(kernel, outputBufferName, outputBuffer);
                shader.SetInt(countPropertyName, elementCount);
                var groupCount = (elementCount + (int)threadsX - 1) / (int)threadsX;
                shader.Dispatch(kernel, groupCount, 1, 1);
                var outputArray = output.BorrowedArray;
                var readback = AsyncGPUReadback.RequestIntoNativeArray(
                    ref outputArray,
                    outputBuffer,
                    (Action<AsyncGPUReadbackRequest>)null);
                var report = new JadrenGpuDispatchReport(
                    kernelName,
                    JadrenGpuDispatchPath.Gpu,
                    elementCount,
                    elementSize,
                    string.Empty);
                dispatch = new JadrenGpuAsyncDispatch<T>(
                    report,
                    input,
                    output,
                    () => readback.done,
                    () =>
                    {
                        try
                        {
                            readback.WaitForCompletion();
                            if (readback.hasError)
                            {
                                throw new InvalidOperationException("Unity GPU readback failed.");
                            }
                        }
                        finally
                        {
                            if (inputBuffer != null)
                            {
                                inputBuffer.Release();
                                inputBuffer = null;
                            }
                            if (outputBuffer != null)
                            {
                                outputBuffer.Release();
                                outputBuffer = null;
                            }
                        }
                    });
                return true;
            }
            catch (Exception error)
            {
                failureReason = "unity_compute_async_exception:" + error.GetType().Name + ":" + error.Message;
                if (inputBuffer != null)
                {
                    inputBuffer.Release();
                }
                if (outputBuffer != null)
                {
                    outputBuffer.Release();
                }
                return false;
            }
        }
    }
}
