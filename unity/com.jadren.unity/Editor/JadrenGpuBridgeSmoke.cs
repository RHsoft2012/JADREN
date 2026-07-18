using System;
using System.Runtime.InteropServices;
using Unity.Collections;
using UnityEngine;
using UnityEngine.Rendering;

namespace Jadren.Unity.Editor
{
    /// <summary>Editor-only contract smoke for the Unity GPU bridge.</summary>
    public static class JadrenGpuBridgeSmoke
    {
        public static void Run()
        {
            NativeArray<int> input = default(NativeArray<int>);
            NativeArray<int> output = default(NativeArray<int>);
            try
            {
                input = new NativeArray<int>(4, Allocator.Persistent);
                output = new NativeArray<int>(4, Allocator.Persistent);
                input[0] = 41;
                input[1] = -3;
                input[2] = 10;
                input[3] = 0;

                JadrenGpuDispatchReport report;
                var computeShaderStatus = "skipped";
                var asyncComputeShaderStatus = "skipped";
                var vulkanStatus = "skipped";
                var vulkanArrayStatus = "skipped";
                var vulkanArrayAsyncStatus = "skipped";
                var vulkanBinaryStatus = "skipped";
                var vulkanBinaryAsyncStatus = "skipped";
                var vulkanTensor3dStatus = "skipped";
                var vulkanTensor3dAsyncStatus = "skipped";
                var tensor3dCpuStatus = "skipped";
                var tensor3dStatus = "skipped";
                using (var inputView = new JadrenNativeArrayView<int>(input))
                using (var outputView = new JadrenNativeArrayView<int>(output))
                {
                    var gpuReport = JadrenGpuBridge.Dispatch(
                        "bridge_smoke_gpu_executor",
                        inputView,
                        outputView,
                        new JadrenGpuDispatchOptions(
                            JadrenGpuTarget.Gpu,
                            JadrenGpuFpPolicy.Strict,
                            64,
                            false),
                        new AddOneExecutor(),
                        new AddOneFallback());
                    if (gpuReport.Path != JadrenGpuDispatchPath.Gpu
                        || gpuReport.UsedCpuFallback
                        || output[0] != 42
                        || output[1] != -2)
                    {
                        throw new InvalidOperationException("Jadren GPU executor success contract failed.");
                    }

                    NativeArray<uint> tensor3dCpuOutput = default(NativeArray<uint>);
                    try
                    {
                        var tensor3dCpuLayout = new JadrenTensor3DLayout(4, 3, 2, 2, 11, 37, 72);
                        tensor3dCpuOutput = new NativeArray<uint>(tensor3dCpuLayout.Capacity, Allocator.Persistent);
                        using (var tensor3dCpuOutputView = new JadrenNativeArrayView<uint>(tensor3dCpuOutput))
                        {
                            var tensor3dCpuReport = JadrenTensor3DU32Bridge.Dispatch(
                                "bridge_smoke_tensor3d_affine_u32_cpu",
                                tensor3dCpuOutputView,
                                tensor3dCpuLayout,
                                42U,
                                new JadrenGpuDispatchOptions(
                                    JadrenGpuTarget.Cpu,
                                    JadrenGpuFpPolicy.Deterministic,
                                    32,
                                    false),
                                null,
                                new JadrenCpuTensor3DU32Fallback());
                            if (!tensor3dCpuReport.UsedCpuFallback
                                || tensor3dCpuOutput[tensor3dCpuLayout.LastPhysicalIndex] != 42U
                                || tensor3dCpuOutput[1] != 0U)
                            {
                                throw new InvalidOperationException("Jadren 3D affine CPU fallback contract failed.");
                            }
                            tensor3dCpuStatus = "passed";
                        }
                    }
                    finally
                    {
                        if (tensor3dCpuOutput.IsCreated)
                        {
                            tensor3dCpuOutput.Dispose();
                        }
                    }

                    if (SystemInfo.supportsComputeShaders
                        && SystemInfo.graphicsDeviceType != GraphicsDeviceType.Null)
                    {
                        var computeShader = Resources.Load<ComputeShader>("JadrenGpuAddOne");
                        if (computeShader == null)
                        {
                            throw new InvalidOperationException("Jadren compute shader resource is missing.");
                        }

                        var computeReport = JadrenGpuBridge.Dispatch(
                            "bridge_smoke_compute_shader",
                            inputView,
                            outputView,
                            new JadrenGpuDispatchOptions(
                                JadrenGpuTarget.Gpu,
                                JadrenGpuFpPolicy.Deterministic,
                                64,
                                false),
                            new JadrenComputeShaderExecutor<int>(computeShader, "AddOne"),
                            new AddOneFallback());
                        if (computeReport.Path != JadrenGpuDispatchPath.Gpu
                            || output[0] != 42
                            || output[1] != -2)
                        {
                            throw new InvalidOperationException("Jadren ComputeShader execution contract failed.");
                        }
                        computeShaderStatus = "passed";

                        var asyncDispatch = JadrenGpuBridge.DispatchAsync(
                            "bridge_smoke_compute_shader_async",
                            inputView,
                            outputView,
                            new JadrenGpuDispatchOptions(
                                JadrenGpuTarget.Gpu,
                                JadrenGpuFpPolicy.Deterministic,
                                64,
                                false),
                            new JadrenComputeShaderExecutor<int>(computeShader, "AddOne"),
                            new AddOneFallback());
                        var leaseBlocked = false;
                        try
                        {
                            inputView.Dispose();
                        }
                        catch (InvalidOperationException)
                        {
                            leaseBlocked = true;
                        }
                        if (!leaseBlocked)
                        {
                            asyncDispatch.Dispose();
                            throw new InvalidOperationException("Async dispatch did not retain the NativeArray lease.");
                        }
                        var asyncReport = asyncDispatch.Complete();
                        asyncDispatch.Dispose();
                        if (asyncReport.Path != JadrenGpuDispatchPath.Gpu
                            || output[0] != 42
                            || output[1] != -2)
                        {
                            throw new InvalidOperationException("Jadren async ComputeShader completion contract failed.");
                        }
                        asyncComputeShaderStatus = "passed";

                        NativeArray<uint> tensor3dOutput = default(NativeArray<uint>);
                        try
                        {
                            var tensor3dLayout = new JadrenTensor3DLayout(4, 3, 2, 2, 11, 37, 72);
                            tensor3dOutput = new NativeArray<uint>(tensor3dLayout.Capacity, Allocator.Persistent);
                            using (var tensor3dOutputView = new JadrenNativeArrayView<uint>(tensor3dOutput))
                            {
                                var tensor3dShader = Resources.Load<ComputeShader>("JadrenGpu3DAffineWrite");
                                if (tensor3dShader == null)
                                {
                                    throw new InvalidOperationException("Jadren 3D affine ComputeShader resource is missing.");
                                }
                                var tensor3dReport = JadrenTensor3DU32Bridge.Dispatch(
                                    "bridge_smoke_tensor3d_affine_u32",
                                    tensor3dOutputView,
                                    tensor3dLayout,
                                    42U,
                                    new JadrenGpuDispatchOptions(
                                        JadrenGpuTarget.Gpu,
                                        JadrenGpuFpPolicy.Deterministic,
                                        32,
                                        false),
                                    new JadrenComputeShaderTensor3DU32Executor(tensor3dShader),
                                    new JadrenCpuTensor3DU32Fallback());
                                if (tensor3dReport.UsedCpuFallback)
                                {
                                    throw new InvalidOperationException("Jadren 3D affine ComputeShader unexpectedly used CPU fallback.");
                                }

                                ulong checksum = 0;
                                var untouched = 0;
                                for (var index = 0; index < tensor3dOutput.Length; index++)
                                {
                                    checksum += tensor3dOutput[index];
                                    if (tensor3dOutput[index] == 0U)
                                    {
                                        untouched++;
                                    }
                                }
                                if (checksum != 1008UL
                                    || untouched != 48
                                    || tensor3dOutput[tensor3dLayout.LastPhysicalIndex] != 42U
                                    || tensor3dOutput[1] != 0U)
                                {
                                    throw new InvalidOperationException("Jadren 3D affine ComputeShader output mismatch.");
                                }
                                tensor3dStatus = "passed";
                            }
                        }
                        finally
                        {
                            if (tensor3dOutput.IsCreated)
                            {
                                tensor3dOutput.Dispose();
                            }
                        }

                        NativeArray<uint> vulkanTensor3dOutput = default(NativeArray<uint>);
                        try
                        {
                            var vulkanTensor3dLayout = new JadrenTensor3DLayout(4, 3, 2, 2, 11, 37, 72);
                            vulkanTensor3dOutput = new NativeArray<uint>(vulkanTensor3dLayout.Capacity, Allocator.Persistent);
                            using (var vulkanTensor3dOutputView = new JadrenNativeArrayView<uint>(vulkanTensor3dOutput))
                            {
                                var vulkanTensor3dReport = JadrenTensor3DU32Bridge.Dispatch(
                                    "bridge_smoke_vulkan_tensor3d_affine_u32",
                                    vulkanTensor3dOutputView,
                                    vulkanTensor3dLayout,
                                    42U,
                                    new JadrenGpuDispatchOptions(
                                        JadrenGpuTarget.Gpu,
                                        JadrenGpuFpPolicy.Deterministic,
                                        32,
                                        true),
                                    new JadrenVulkanTensor3DU32Executor(),
                                    new JadrenCpuTensor3DU32Fallback());
                                if (!vulkanTensor3dReport.UsedCpuFallback)
                                {
                                    ulong checksum = 0;
                                    var untouched = 0;
                                    for (var index = 0; index < vulkanTensor3dOutput.Length; index++)
                                    {
                                        checksum += vulkanTensor3dOutput[index];
                                        if (vulkanTensor3dOutput[index] == 0U)
                                        {
                                            untouched++;
                                        }
                                    }
                                    if (checksum != 1008UL
                                        || untouched != 48
                                        || vulkanTensor3dOutput[vulkanTensor3dLayout.LastPhysicalIndex] != 42U
                                        || vulkanTensor3dOutput[1] != 0U)
                                    {
                                        throw new InvalidOperationException("Jadren Vulkan 3D affine output mismatch.");
                                    }
                                    vulkanTensor3dStatus = "passed";
                                }
                                else
                                {
                                    vulkanTensor3dStatus = "skipped:" + vulkanTensor3dReport.FallbackReason;
                                }
                            }
                        }
                        finally
                        {
                            if (vulkanTensor3dOutput.IsCreated)
                            {
                                vulkanTensor3dOutput.Dispose();
                            }
                        }

                        NativeArray<uint> vulkanTensor3dAsyncOutput = default(NativeArray<uint>);
                        try
                        {
                            var vulkanTensor3dAsyncLayout = new JadrenTensor3DLayout(4, 3, 2, 2, 11, 37, 72);
                            vulkanTensor3dAsyncOutput = new NativeArray<uint>(vulkanTensor3dAsyncLayout.Capacity, Allocator.Persistent);
                            using (var vulkanTensor3dAsyncOutputView = new JadrenNativeArrayView<uint>(vulkanTensor3dAsyncOutput))
                            {
                                var vulkanTensor3dAsync = JadrenTensor3DU32Bridge.DispatchAsync(
                                    "bridge_smoke_vulkan_tensor3d_affine_u32_async",
                                    vulkanTensor3dAsyncOutputView,
                                    vulkanTensor3dAsyncLayout,
                                    42U,
                                    new JadrenGpuDispatchOptions(
                                        JadrenGpuTarget.Gpu,
                                        JadrenGpuFpPolicy.Deterministic,
                                        32,
                                        true),
                                    new JadrenVulkanTensor3DU32Executor(),
                                    new JadrenCpuTensor3DU32Fallback());
                                if (vulkanTensor3dAsync.UsedCpuFallback)
                                {
                                    vulkanTensor3dAsync.Dispose();
                                    vulkanTensor3dAsyncStatus = "skipped:" + vulkanTensor3dAsync.Report.FallbackReason;
                                }
                                else
                                {
                                    var tensor3dAsyncLeaseBlocked = false;
                                    try
                                    {
                                        vulkanTensor3dAsyncOutputView.Dispose();
                                    }
                                    catch (InvalidOperationException)
                                    {
                                        tensor3dAsyncLeaseBlocked = true;
                                    }
                                    if (!tensor3dAsyncLeaseBlocked)
                                    {
                                        vulkanTensor3dAsync.Dispose();
                                        throw new InvalidOperationException("Vulkan 3D async dispatch did not retain the NativeArray lease.");
                                    }
                                    var completed = vulkanTensor3dAsync.Complete();
                                    vulkanTensor3dAsync.Dispose();
                                    ulong checksum = 0;
                                    var untouched = 0;
                                    for (var index = 0; index < vulkanTensor3dAsyncOutput.Length; index++)
                                    {
                                        checksum += vulkanTensor3dAsyncOutput[index];
                                        if (vulkanTensor3dAsyncOutput[index] == 0U)
                                        {
                                            untouched++;
                                        }
                                    }
                                    if (completed.Path != JadrenTensor3DDispatchPath.Gpu
                                        || checksum != 1008UL
                                        || untouched != 48
                                        || vulkanTensor3dAsyncOutput[vulkanTensor3dAsyncLayout.LastPhysicalIndex] != 42U
                                        || vulkanTensor3dAsyncOutput[1] != 0U)
                                    {
                                        throw new InvalidOperationException("Jadren Vulkan 3D async affine output mismatch.");
                                    }
                                    vulkanTensor3dAsyncStatus = "passed";
                                }
                            }
                        }
                        finally
                        {
                            if (vulkanTensor3dAsyncOutput.IsCreated)
                            {
                                vulkanTensor3dAsyncOutput.Dispose();
                            }
                        }

                        NativeArray<float> vulkanInput = default(NativeArray<float>);
                        NativeArray<float> vulkanOutput = default(NativeArray<float>);
                        try
                        {
                            const int vulkanF32Count = 70;
                            vulkanInput = new NativeArray<float>(vulkanF32Count, Allocator.Persistent);
                            vulkanOutput = new NativeArray<float>(vulkanF32Count, Allocator.Persistent);
                            for (var index = 0; index < vulkanF32Count; index++)
                            {
                                vulkanInput[index] = 7.0f + index * 3.0f;
                            }
                            using (var vulkanInputView = new JadrenNativeArrayView<float>(vulkanInput))
                            using (var vulkanOutputView = new JadrenNativeArrayView<float>(vulkanOutput))
                            {
                                var vulkanReport = JadrenGpuBridge.Dispatch(
                                    "bridge_smoke_vulkan_f32",
                                    vulkanInputView,
                                    vulkanOutputView,
                                    new JadrenGpuDispatchOptions(
                                        JadrenGpuTarget.Gpu,
                                        JadrenGpuFpPolicy.Deterministic,
                                        1,
                                        true),
                                    new JadrenVulkanF32Executor(),
                                    new AddOneFloatFallback());
                                if (!vulkanReport.UsedCpuFallback)
                                {
                                    for (var index = 0; index < vulkanF32Count; index++)
                                    {
                                        if (vulkanOutput[index] != vulkanInput[index] + 1.0f)
                                        {
                                            throw new InvalidOperationException("Jadren Vulkan f32 runtime-length output mismatch.");
                                        }
                                    }
                                    vulkanStatus = "passed";
                                }
                                else
                                {
                                    vulkanStatus = "skipped:" + vulkanReport.FallbackReason;
                                }
                            }
                        }
                        finally
                        {
                            if (vulkanOutput.IsCreated)
                            {
                                vulkanOutput.Dispose();
                            }
                            if (vulkanInput.IsCreated)
                            {
                                vulkanInput.Dispose();
                            }
                        }

                        NativeArray<uint> vulkanArrayInput = default(NativeArray<uint>);
                        NativeArray<uint> vulkanArrayOutput = default(NativeArray<uint>);
                        try
                        {
                            vulkanArrayInput = new NativeArray<uint>(4, Allocator.Persistent);
                            vulkanArrayOutput = new NativeArray<uint>(4, Allocator.Persistent);
                            vulkanArrayInput[0] = 7;
                            vulkanArrayInput[1] = 10;
                            vulkanArrayInput[2] = 13;
                            vulkanArrayInput[3] = 16;
                            using (var vulkanArrayInputView = new JadrenNativeArrayView<uint>(vulkanArrayInput))
                            using (var vulkanArrayOutputView = new JadrenNativeArrayView<uint>(vulkanArrayOutput))
                            {
                                var vulkanArrayReport = JadrenGpuBridge.Dispatch(
                                    "bridge_smoke_vulkan_u32_array",
                                    vulkanArrayInputView,
                                    vulkanArrayOutputView,
                                    new JadrenGpuDispatchOptions(
                                        JadrenGpuTarget.Gpu,
                                        JadrenGpuFpPolicy.Deterministic,
                                        64,
                                        true),
                                    new JadrenVulkanU32ArrayExecutor(),
                                    new AddOneUIntFallback());
                                if (!vulkanArrayReport.UsedCpuFallback)
                                {
                                    if (vulkanArrayOutput[0] != 8
                                        || vulkanArrayOutput[1] != 11
                                        || vulkanArrayOutput[2] != 14
                                        || vulkanArrayOutput[3] != 17)
                                    {
                                        throw new InvalidOperationException("Jadren Vulkan u32 array output mismatch.");
                                    }
                                    vulkanArrayStatus = "passed";
                                }
                                else
                                {
                                    vulkanArrayStatus = "skipped:" + vulkanArrayReport.FallbackReason;
                                }

                                var vulkanArrayAsync = JadrenGpuBridge.DispatchAsync(
                                    "bridge_smoke_vulkan_u32_array_async",
                                    vulkanArrayInputView,
                                    vulkanArrayOutputView,
                                    new JadrenGpuDispatchOptions(
                                        JadrenGpuTarget.Gpu,
                                        JadrenGpuFpPolicy.Deterministic,
                                        64,
                                        true),
                                    new JadrenVulkanU32ArrayExecutor(),
                                    new AddOneUIntFallback());
                                var arrayLeaseBlocked = false;
                                try
                                {
                                    vulkanArrayInputView.Dispose();
                                }
                                catch (InvalidOperationException)
                                {
                                    arrayLeaseBlocked = true;
                                }
                                if (!arrayLeaseBlocked)
                                {
                                    vulkanArrayAsync.Dispose();
                                    throw new InvalidOperationException("Vulkan u32 async dispatch did not retain the NativeArray lease.");
                                }
                                var vulkanArrayAsyncReport = vulkanArrayAsync.Complete();
                                vulkanArrayAsync.Dispose();
                                if (vulkanArrayAsyncReport.Path != JadrenGpuDispatchPath.Gpu
                                    || vulkanArrayOutput[0] != 8
                                    || vulkanArrayOutput[1] != 11
                                    || vulkanArrayOutput[2] != 14
                                    || vulkanArrayOutput[3] != 17)
                                {
                                    throw new InvalidOperationException("Jadren Vulkan u32 async output mismatch.");
                                }
                                vulkanArrayAsyncStatus = "passed";

                                var vulkanBinaryReport = JadrenGpuBridge.Dispatch(
                                    "bridge_smoke_vulkan_u32_binary_multiply",
                                    vulkanArrayInputView,
                                    vulkanArrayOutputView,
                                    new JadrenGpuDispatchOptions(
                                        JadrenGpuTarget.Gpu,
                                        JadrenGpuFpPolicy.Deterministic,
                                        64,
                                        true),
                                    new JadrenVulkanU32BinaryExecutor(
                                        JadrenU32BinaryOperation.Multiply,
                                        3U),
                                    new AddOneUIntFallback());
                                if (!vulkanBinaryReport.UsedCpuFallback)
                                {
                                    if (vulkanArrayOutput[0] != 21U
                                        || vulkanArrayOutput[1] != 30U
                                        || vulkanArrayOutput[2] != 39U
                                        || vulkanArrayOutput[3] != 48U)
                                    {
                                        throw new InvalidOperationException("Jadren Vulkan u32 binary output mismatch.");
                                    }
                                    vulkanBinaryStatus = "passed";
                                }
                                else
                                {
                                    vulkanBinaryStatus = "skipped:" + vulkanBinaryReport.FallbackReason;
                                }

                                var vulkanBinaryAsync = JadrenGpuBridge.DispatchAsync(
                                    "bridge_smoke_vulkan_u32_binary_multiply_async",
                                    vulkanArrayInputView,
                                    vulkanArrayOutputView,
                                    new JadrenGpuDispatchOptions(
                                        JadrenGpuTarget.Gpu,
                                        JadrenGpuFpPolicy.Deterministic,
                                        64,
                                        true),
                                    new JadrenVulkanU32BinaryExecutor(
                                        JadrenU32BinaryOperation.Multiply,
                                        3U),
                                    new AddOneUIntFallback());
                                if (!vulkanBinaryAsync.UsedCpuFallback)
                                {
                                    var binaryLeaseBlocked = false;
                                    try
                                    {
                                        vulkanArrayInputView.Dispose();
                                    }
                                    catch (InvalidOperationException)
                                    {
                                        binaryLeaseBlocked = true;
                                    }
                                    if (!binaryLeaseBlocked)
                                    {
                                        vulkanBinaryAsync.Dispose();
                                        throw new InvalidOperationException("Vulkan binary async dispatch did not retain the NativeArray lease.");
                                    }
                                    vulkanBinaryAsync.Complete();
                                    vulkanBinaryAsync.Dispose();
                                    if (vulkanArrayOutput[0] != 21U
                                        || vulkanArrayOutput[1] != 30U
                                        || vulkanArrayOutput[2] != 39U
                                        || vulkanArrayOutput[3] != 48U)
                                    {
                                        throw new InvalidOperationException("Jadren Vulkan u32 binary async output mismatch.");
                                    }
                                    vulkanBinaryAsyncStatus = "passed";
                                }
                                else
                                {
                                    vulkanBinaryAsync.Dispose();
                                    vulkanBinaryAsyncStatus = "skipped:" + vulkanBinaryAsync.Report.FallbackReason;
                                }
                            }
                        }
                        finally
                        {
                            if (vulkanArrayOutput.IsCreated)
                            {
                                vulkanArrayOutput.Dispose();
                            }
                            if (vulkanArrayInput.IsCreated)
                            {
                                vulkanArrayInput.Dispose();
                            }
                        }
                    }

                    var options = new JadrenGpuDispatchOptions(
                        JadrenGpuTarget.Auto,
                        JadrenGpuFpPolicy.Deterministic,
                        64,
                        true);
                    report = JadrenGpuBridge.Dispatch(
                        "bridge_smoke_add_one",
                        inputView,
                        outputView,
                        options,
                        new UnavailableExecutor(),
                        new AddOneFallback());

                    if (!report.UsedCpuFallback
                        || report.ElementCount != 4
                        || report.ElementSize != sizeof(int)
                        || output[0] != 42
                        || output[1] != -2
                        || report.FallbackReason != "gpu_executor_unavailable")
                    {
                        throw new InvalidOperationException("Jadren GPU bridge fallback contract failed.");
                    }

                    var rejected = false;
                    try
                    {
                        JadrenGpuBridge.Dispatch(
                            "bridge_smoke_reject",
                            inputView,
                            outputView,
                            new JadrenGpuDispatchOptions(
                                JadrenGpuTarget.Gpu,
                                JadrenGpuFpPolicy.Strict,
                                64,
                                false),
                            new UnavailableExecutor(),
                            new AddOneFallback());
                    }
                    catch (InvalidOperationException)
                    {
                        rejected = true;
                    }

                    if (!rejected)
                    {
                        throw new InvalidOperationException("Explicit GPU request silently fell back.");
                    }

                    var autoRejected = false;
                    try
                    {
                        JadrenGpuBridge.Dispatch(
                            "bridge_smoke_auto_reject",
                            inputView,
                            outputView,
                            new JadrenGpuDispatchOptions(
                                JadrenGpuTarget.Auto,
                                JadrenGpuFpPolicy.Deterministic,
                                64,
                                false),
                            new UnavailableExecutor(),
                            new AddOneFallback());
                    }
                    catch (InvalidOperationException)
                    {
                        autoRejected = true;
                    }

                    if (!autoRejected)
                    {
                        throw new InvalidOperationException("Auto target silently ignored fallback policy.");
                    }
                }

                Debug.Log(
                    "JADREN_GPU_BRIDGE_SMOKE status=passed "
                    + "path=" + report.Path
                    + " reason=" + report.FallbackReason
                    + " compute_shader=" + computeShaderStatus
                    + " async_compute_shader=" + asyncComputeShaderStatus
                    + " vulkan=" + vulkanStatus
                    + " vulkan_array=" + vulkanArrayStatus
                    + " vulkan_array_async=" + vulkanArrayAsyncStatus
                    + " vulkan_binary=" + vulkanBinaryStatus
                    + " vulkan_binary_async=" + vulkanBinaryAsyncStatus
                    + " vulkan_tensor3d_affine=" + vulkanTensor3dStatus
                    + " vulkan_tensor3d_async=" + vulkanTensor3dAsyncStatus
                    + " tensor3d_cpu=" + tensor3dCpuStatus
                    + " tensor3d_affine=" + tensor3dStatus);
            }
            finally
            {
                if (output.IsCreated)
                {
                    output.Dispose();
                }
                if (input.IsCreated)
                {
                    input.Dispose();
                }
            }
        }

        private sealed class UnavailableExecutor : IJadrenGpuExecutor
        {
            public bool IsAvailable => false;

            public bool TryDispatch(
                IntPtr input,
                IntPtr output,
                int elementCount,
                int elementSize,
                JadrenGpuDispatchOptions options,
                out string failureReason)
            {
                failureReason = "should_not_be_called";
                return false;
            }
        }

        private sealed class AddOneExecutor : IJadrenGpuExecutor
        {
            public bool IsAvailable => true;

            public bool TryDispatch(
                IntPtr input,
                IntPtr output,
                int elementCount,
                int elementSize,
                JadrenGpuDispatchOptions options,
                out string failureReason)
            {
                if (elementSize != sizeof(int))
                {
                    failureReason = "unexpected_element_size";
                    return false;
                }

                for (var index = 0; index < elementCount; index++)
                {
                    var offset = checked(index * sizeof(int));
                    Marshal.WriteInt32(output, offset, Marshal.ReadInt32(input, offset) + 1);
                }
                failureReason = string.Empty;
                return true;
            }
        }

        private sealed class AddOneFallback : IJadrenCpuFallback<int>
        {
            public void Execute(IntPtr input, IntPtr output, int elementCount)
            {
                for (var index = 0; index < elementCount; index++)
                {
                    var offset = checked(index * sizeof(int));
                    Marshal.WriteInt32(output, offset, Marshal.ReadInt32(input, offset) + 1);
                }
            }
        }

        private sealed class AddOneFloatFallback : IJadrenCpuFallback<float>
        {
            public void Execute(IntPtr input, IntPtr output, int elementCount)
            {
                var values = new float[elementCount];
                Marshal.Copy(input, values, 0, elementCount);
                for (var index = 0; index < elementCount; index++)
                {
                    values[index] += 1.0f;
                }
                Marshal.Copy(values, 0, output, elementCount);
            }
        }

        private sealed class AddOneUIntFallback : IJadrenCpuFallback<uint>
        {
            public void Execute(IntPtr input, IntPtr output, int elementCount)
            {
                for (var index = 0; index < elementCount; index++)
                {
                    var offset = checked(index * sizeof(uint));
                    var value = unchecked((uint)Marshal.ReadInt32(input, offset));
                    Marshal.WriteInt32(output, offset, unchecked((int)(value + 1U)));
                }
            }
        }
    }
}
