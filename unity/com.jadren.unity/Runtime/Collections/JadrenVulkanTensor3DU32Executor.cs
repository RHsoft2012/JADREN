using System;
using System.Runtime.InteropServices;
using Unity.Collections;
using Unity.Collections.LowLevel.Unsafe;

namespace Jadren.Unity
{
    /// <summary>
    /// Native Vulkan executor for the parametrized 3D affine-stride u32 C ABI.
    /// The NativeArray pointer is borrowed until synchronous return or async
    /// completion/release.
    /// </summary>
    public sealed class JadrenVulkanTensor3DU32Executor : IJadrenGpuTensor3DU32Executor, IJadrenGpuAsyncTensor3DU32Executor
    {
        private const int WorkgroupSize = 32;

        public bool IsAvailable => true;

        public bool TryDispatch(
            NativeArray<uint> output,
            JadrenTensor3DLayout layout,
            uint value,
            JadrenGpuDispatchOptions options,
            out string failureReason)
        {
            failureReason = string.Empty;
            if (!output.IsCreated || output.Length != layout.Capacity)
            {
                failureReason = "vulkan_tensor3d_capacity_invalid";
                return false;
            }
            if (options.WorkgroupSize != WorkgroupSize)
            {
                failureReason = "vulkan_tensor3d_workgroup_size_mismatch";
                return false;
            }

            try
            {
                NativeResult result;
                unsafe
                {
                    result = Native.jadren_vk_u32_3d_strided_write(
                        (IntPtr)NativeArrayUnsafeUtility.GetUnsafePtr(output),
                        checked((uint)layout.Width),
                        checked((uint)layout.Height),
                        checked((uint)layout.Depth),
                        checked((uint)layout.StrideX),
                        checked((uint)layout.StrideY),
                        checked((uint)layout.StrideZ),
                        checked((uint)layout.Capacity),
                        value);
                }
                if (result.Status != 0)
                {
                    failureReason = "vulkan_status_" + result.Status;
                    return false;
                }
                if (result.TimelineCompleted != 1U || result.TimelineValue != 1UL)
                {
                    failureReason = "vulkan_tensor3d_timeline_incomplete";
                    return false;
                }
                if (result.Width != (uint)layout.Width
                    || result.Height != (uint)layout.Height
                    || result.Depth != (uint)layout.Depth
                    || result.StrideX != (uint)layout.StrideX
                    || result.StrideY != (uint)layout.StrideY
                    || result.StrideZ != (uint)layout.StrideZ
                    || result.Capacity != (uint)layout.Capacity
                    || result.LastPhysicalIndex != (uint)layout.LastPhysicalIndex
                    || result.WrittenElements != (uint)layout.LogicalElementCount)
                {
                    failureReason = "vulkan_tensor3d_metadata_mismatch";
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
                failureReason = "vulkan_tensor3d_entry_missing";
                return false;
            }
            catch (BadImageFormatException)
            {
                failureReason = "vulkan_plugin_architecture_mismatch";
                return false;
            }
            catch (OverflowException)
            {
                failureReason = "vulkan_tensor3d_metadata_overflow";
                return false;
            }
        }

        public bool TryDispatchAsync(
            string kernelName,
            JadrenNativeArrayAsyncLease<uint> output,
            JadrenTensor3DLayout layout,
            uint value,
            JadrenGpuDispatchOptions options,
            out JadrenTensor3DU32AsyncDispatch dispatch,
            out string failureReason)
        {
            dispatch = null;
            failureReason = string.Empty;
            if (output == null || output.Length != layout.Capacity)
            {
                failureReason = "vulkan_tensor3d_capacity_invalid";
                return false;
            }
            if (options.WorkgroupSize != WorkgroupSize)
            {
                failureReason = "vulkan_tensor3d_workgroup_size_mismatch";
                return false;
            }

            NativeAsyncBeginResult begin;
            try
            {
                begin = Native.jadren_vk_u32_3d_strided_write_async(
                    output.Pointer,
                    checked((uint)layout.Width),
                    checked((uint)layout.Height),
                    checked((uint)layout.Depth),
                    checked((uint)layout.StrideX),
                    checked((uint)layout.StrideY),
                    checked((uint)layout.StrideZ),
                    checked((uint)layout.Capacity),
                    value);
            }
            catch (DllNotFoundException)
            {
                failureReason = "vulkan_plugin_missing";
                return false;
            }
            catch (EntryPointNotFoundException)
            {
                failureReason = "vulkan_tensor3d_async_entry_missing";
                return false;
            }
            catch (BadImageFormatException)
            {
                failureReason = "vulkan_plugin_architecture_mismatch";
                return false;
            }
            catch (OverflowException)
            {
                failureReason = "vulkan_tensor3d_metadata_overflow";
                return false;
            }

            if (begin.Status != 0 || begin.Handle == IntPtr.Zero)
            {
                failureReason = "vulkan_async_status_" + begin.Status;
                return false;
            }

            var handle = begin.Handle;
            var handleOwned = true;
            try
            {
                var report = new JadrenTensor3DDispatchReport(
                    kernelName,
                    JadrenTensor3DDispatchPath.Gpu,
                    layout,
                    value,
                    string.Empty);
                dispatch = new JadrenTensor3DU32AsyncDispatch(
                    report,
                    output,
                    () =>
                    {
                        var poll = Native.jadren_vk_u32_3d_strided_write_async_poll(handle);
                        if (poll < 0)
                        {
                            throw new InvalidOperationException("Vulkan async poll failed with status " + poll + ".");
                        }
                        return poll == 1;
                    },
                    () =>
                    {
                        var result = Native.jadren_vk_u32_3d_strided_write_async_complete(handle);
                        handle = IntPtr.Zero;
                        if (result.Status != 0)
                        {
                            throw new InvalidOperationException("Vulkan async completion failed with status " + result.Status + ".");
                        }
                        if (!MatchesLayout(result, layout))
                        {
                            throw new InvalidOperationException("Vulkan async completion metadata mismatch.");
                        }
                        if (result.TimelineCompleted != 1U || result.TimelineValue != 1UL)
                        {
                            throw new InvalidOperationException("Vulkan async timeline completion was incomplete.");
                        }
                    });
                handleOwned = false;
                return true;
            }
            catch (DllNotFoundException)
            {
                failureReason = "vulkan_plugin_missing";
                return false;
            }
            catch (EntryPointNotFoundException)
            {
                failureReason = "vulkan_tensor3d_async_entry_missing";
                return false;
            }
            finally
            {
                if (handleOwned)
                {
                    try
                    {
                        Native.jadren_vk_u32_3d_strided_write_async_release(handle);
                    }
                    catch (Exception)
                    {
                        // The original begin failure is the actionable result;
                        // the native release is best-effort cleanup here.
                    }
                }
            }
        }

        private static bool MatchesLayout(NativeResult result, JadrenTensor3DLayout layout)
        {
            return result.Width == (uint)layout.Width
                && result.Height == (uint)layout.Height
                && result.Depth == (uint)layout.Depth
                && result.StrideX == (uint)layout.StrideX
                && result.StrideY == (uint)layout.StrideY
                && result.StrideZ == (uint)layout.StrideZ
                && result.Capacity == (uint)layout.Capacity
                && result.LastPhysicalIndex == (uint)layout.LastPhysicalIndex
                && result.WrittenElements == (uint)layout.LogicalElementCount;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct NativeResult
        {
            public int Status;
            public ulong OutputChecksum;
            public ulong TimelineValue;
            public uint TimelineCompleted;
            public uint PhysicalDeviceCount;
            public uint Width;
            public uint Height;
            public uint Depth;
            public uint StrideX;
            public uint StrideY;
            public uint StrideZ;
            public uint Capacity;
            public uint LastPhysicalIndex;
            public uint WrittenElements;
            public uint UntouchedElements;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct NativeAsyncBeginResult
        {
            public int Status;
            public IntPtr Handle;
        }

        private static class Native
        {
            [DllImport("jadren_vulkan_runtime", CallingConvention = CallingConvention.Cdecl,
                EntryPoint = "jadren_vk_u32_3d_strided_write")]
            internal static extern NativeResult jadren_vk_u32_3d_strided_write(
                IntPtr output,
                uint width,
                uint height,
                uint depth,
                uint strideX,
                uint strideY,
                uint strideZ,
                uint capacity,
                uint value);

            [DllImport("jadren_vulkan_runtime", CallingConvention = CallingConvention.Cdecl,
                EntryPoint = "jadren_vk_u32_3d_strided_write_async")]
            internal static extern NativeAsyncBeginResult jadren_vk_u32_3d_strided_write_async(
                IntPtr output,
                uint width,
                uint height,
                uint depth,
                uint strideX,
                uint strideY,
                uint strideZ,
                uint capacity,
                uint value);

            [DllImport("jadren_vulkan_runtime", CallingConvention = CallingConvention.Cdecl,
                EntryPoint = "jadren_vk_u32_3d_strided_write_async_poll")]
            internal static extern int jadren_vk_u32_3d_strided_write_async_poll(IntPtr handle);

            [DllImport("jadren_vulkan_runtime", CallingConvention = CallingConvention.Cdecl,
                EntryPoint = "jadren_vk_u32_3d_strided_write_async_complete")]
            internal static extern NativeResult jadren_vk_u32_3d_strided_write_async_complete(IntPtr handle);

            [DllImport("jadren_vulkan_runtime", CallingConvention = CallingConvention.Cdecl,
                EntryPoint = "jadren_vk_u32_3d_strided_write_async_release")]
            internal static extern int jadren_vk_u32_3d_strided_write_async_release(IntPtr handle);
        }
    }
}
