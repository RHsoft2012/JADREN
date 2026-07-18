using System;
using System.Runtime.InteropServices;

namespace Jadren.Unity
{
    /// <summary>Stable operation codes shared with the native Vulkan ABI.</summary>
    public enum JadrenU32BinaryOperation
    {
        Add = 0,
        Subtract = 1,
        Multiply = 2,
        Divide = 3,
        Remainder = 4,
        BitAnd = 5,
        BitOr = 6,
        BitXor = 7,
        ShiftLeft = 8,
        ShiftRight = 9
    }

    /// <summary>
    /// Native Vulkan executor for the parametrized runtime-length u32 binary
    /// kernel. The operation and operand are immutable for one executor.
    /// </summary>
    public sealed class JadrenVulkanU32BinaryExecutor : IJadrenGpuExecutor,
        IJadrenGpuAsyncNativeArrayExecutor<uint>
    {
        private const int ElementSize = sizeof(uint);
        private const int MaxElements = 128;
        private const int WorkgroupSize = 64;
        private readonly JadrenU32BinaryOperation operation;
        private readonly uint operand;

        public JadrenVulkanU32BinaryExecutor(
            JadrenU32BinaryOperation operation,
            uint operand)
        {
            if ((operation == JadrenU32BinaryOperation.Divide
                    || operation == JadrenU32BinaryOperation.Remainder)
                && operand == 0U)
            {
                throw new ArgumentOutOfRangeException(nameof(operand), "Division operands must be non-zero.");
            }
            if ((operation == JadrenU32BinaryOperation.ShiftLeft
                    || operation == JadrenU32BinaryOperation.ShiftRight)
                && operand >= 32U)
            {
                throw new ArgumentOutOfRangeException(nameof(operand), "Shift operands must be smaller than 32.");
            }

            this.operation = operation;
            this.operand = operand;
        }

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
                failureReason = "vulkan_u32_binary_contract_invalid";
                return false;
            }
            if (options.WorkgroupSize != WorkgroupSize)
            {
                failureReason = "vulkan_workgroup_size_mismatch";
                return false;
            }

            try
            {
                var result = Native.jadren_vk_u32_binary_array(
                    input,
                    output,
                    checked((uint)elementCount),
                    (uint)operation,
                    operand);
                if (result.Status != 0)
                {
                    failureReason = "vulkan_status_" + result.Status;
                    return false;
                }
                if (result.ProcessedLength != (uint)elementCount
                    || result.Operation != (uint)operation
                    || result.Operand != operand)
                {
                    failureReason = "vulkan_u32_binary_metadata_mismatch";
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
                failureReason = "vulkan_binary_entry_missing";
                return false;
            }
            catch (BadImageFormatException)
            {
                failureReason = "vulkan_plugin_architecture_mismatch";
                return false;
            }
        }

        /// <summary>
        /// The current binary ABI is synchronous; this completed wrapper
        /// preserves the bridge's lease and async ownership contract.
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
            dispatch = new JadrenGpuAsyncDispatch<uint>(report, input, output, null, null);
            return true;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct NativeResult
        {
            public int Status;
            public ulong OutputChecksum;
            public uint PhysicalDeviceCount;
            public uint ProcessedLength;
            public uint Operation;
            public uint Operand;
        }

        private static class Native
        {
            [DllImport("jadren_vulkan_runtime", CallingConvention = CallingConvention.Cdecl,
                EntryPoint = "jadren_vk_u32_binary_array")]
            internal static extern NativeResult jadren_vk_u32_binary_array(
                IntPtr input,
                IntPtr output,
                uint length,
                uint operation,
                uint operand);
        }
    }
}
