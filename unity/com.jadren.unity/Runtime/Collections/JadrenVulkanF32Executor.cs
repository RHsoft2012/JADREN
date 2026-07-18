using System;
using System.Runtime.InteropServices;

namespace Jadren.Unity
{
    /// <summary>
    /// Native Vulkan/SPIR-V executor for the bounded runtime-length f32 add
    /// kernel. The native ABI owns no Unity memory and processes the borrowed
    /// NativeArray range exactly once after the GPU fence completes.
    /// </summary>
    public sealed class JadrenVulkanF32Executor : IJadrenGpuExecutor
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
            failureReason = string.Empty;
            if (input == IntPtr.Zero || output == IntPtr.Zero)
            {
                failureReason = "vulkan_pointer_invalid";
                return false;
            }
            if (elementCount <= 0 || elementSize != sizeof(float))
            {
                failureReason = "vulkan_f32_requires_nonempty_f32_elements";
                return false;
            }
            try
            {
                var result = Native.jadren_vk_f32_add_one_array(input, output, checked((uint)elementCount));
                if (result.Status != 0)
                {
                    failureReason = "vulkan_status_" + result.Status;
                    return false;
                }
                if (result.ProcessedLength != checked((uint)elementCount))
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

        [StructLayout(LayoutKind.Sequential)]
        private struct NativeResult
        {
            public int Status;
            public double OutputChecksum;
            public uint PhysicalDeviceCount;
            public uint ProcessedLength;
        }

        private static class Native
        {
            [DllImport("jadren_vulkan_runtime", CallingConvention = CallingConvention.Cdecl,
                EntryPoint = "jadren_vk_f32_add_one_array")]
            internal static extern NativeResult jadren_vk_f32_add_one_array(
                IntPtr input,
                IntPtr output,
                uint length);
        }
    }
}
