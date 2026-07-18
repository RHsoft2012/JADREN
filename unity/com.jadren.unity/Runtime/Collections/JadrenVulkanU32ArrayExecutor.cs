using System;
using System.Runtime.InteropServices;

namespace Jadren.Unity
{
    /// <summary>
    /// Native Vulkan/SPIR-V executor for the bounded dynamic u32 array
    /// contract. The current ABI accepts one to 128 elements and dispatches
    /// one-dimensional workgroups of 64 lanes.
    /// </summary>
    public sealed class JadrenVulkanU32ArrayExecutor : IJadrenGpuExecutor,
        IJadrenGpuAsyncNativeArrayExecutor<uint>
    {
        private const int ElementSize = sizeof(uint);
        private const int MaxElements = 128;
        private const int WorkgroupSize = 64;

        public bool IsAvailable => true;

        public bool TryDispatch(
            IntPtr input,
            IntPtr output,
            int elementCount,
            int elementSize,
            JadrenGpuDispatchOptions options,
            out string failureReason)
        {
            failureReason = string.Empty;
            if (input == IntPtr.Zero || output == IntPtr.Zero)
            {
                failureReason = "vulkan_pointer_invalid";
                return false;
            }
            if (elementCount < 1 || elementCount > MaxElements || elementSize != ElementSize)
            {
                failureReason = "vulkan_u32_array_contract_invalid";
                return false;
            }
            if (options.WorkgroupSize != WorkgroupSize)
            {
                failureReason = "vulkan_workgroup_size_mismatch";
                return false;
            }

            try
            {
                var result = Native.jadren_vk_u32_add_one_array(
                    input,
                    output,
                    checked((uint)elementCount));
                if (result.Status != 0)
                {
                    failureReason = "vulkan_status_" + result.Status;
                    return false;
                }
                if (result.ProcessedLength != (uint)elementCount)
                {
                    failureReason = "vulkan_processed_length_mismatch";
                    return false;
                }
                return true;
            }
            catch (DllNotFoundException)
            {
                failureReason = "vulkan_plugin_missing";
                return false;
            }
            catch (EntryPointNotFoundException)
            {
                failureReason = "vulkan_entry_missing";
                return false;
            }
            catch (BadImageFormatException)
            {
                failureReason = "vulkan_plugin_architecture_mismatch";
                return false;
            }
        }

        /// <summary>
        /// The native ABI call is synchronous. It is exposed through the
        /// bridge's async lease API as an already-completed handle so callers
        /// still get deterministic lease ownership and completion semantics.
        /// </summary>
        public bool TryDispatchAsync(
            string kernelName,
            JadrenNativeArrayAsyncLease<uint> input,
            JadrenNativeArrayAsyncLease<uint> output,
            int elementCount,
            JadrenGpuDispatchOptions options,
            out JadrenGpuAsyncDispatch<uint> dispatch,
            out string failureReason)
        {
            dispatch = null;
            failureReason = string.Empty;
            if (input == null || output == null || input.Length < elementCount || output.Length < elementCount)
            {
                failureReason = "async_lease_invalid";
                return false;
            }
            if (!TryDispatch(
                    input.Pointer,
                    output.Pointer,
                    elementCount,
                    input.ElementSize,
                    options,
                    out failureReason))
            {
                return false;
            }

            var report = new JadrenGpuDispatchReport(
                kernelName,
                JadrenGpuDispatchPath.Gpu,
                elementCount,
                input.ElementSize,
                string.Empty);
            dispatch = new JadrenGpuAsyncDispatch<uint>(
                report,
                input,
                output,
                null,
                null);
            return true;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct NativeResult
        {
            public int Status;
            public ulong OutputChecksum;
            public uint PhysicalDeviceCount;
            public uint ProcessedLength;
        }

        private static class Native
        {
            [DllImport("jadren_vulkan_runtime", CallingConvention = CallingConvention.Cdecl,
                EntryPoint = "jadren_vk_u32_add_one_array")]
            internal static extern NativeResult jadren_vk_u32_add_one_array(
                IntPtr input,
                IntPtr output,
                uint length);
        }
    }
}
