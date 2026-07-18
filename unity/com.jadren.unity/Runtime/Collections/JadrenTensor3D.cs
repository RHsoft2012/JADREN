using System;
using System.Runtime.InteropServices;
using Unity.Collections;

namespace Jadren.Unity
{
    /// <summary>
    /// Validated affine layout for a three-dimensional logical tensor stored
    /// in one physical u32 buffer. All strides and indices are measured in
    /// elements, not bytes.
    /// </summary>
    public readonly struct JadrenTensor3DLayout
    {
        public JadrenTensor3DLayout(
            int width,
            int height,
            int depth,
            int strideX,
            int strideY,
            int strideZ,
            int capacity)
        {
            if (width < 1 || height < 1 || depth < 1)
            {
                throw new ArgumentOutOfRangeException(nameof(width), "Tensor dimensions must be positive.");
            }
            if (strideX < 1 || strideY < 1 || strideZ < 1)
            {
                throw new ArgumentOutOfRangeException(nameof(strideX), "Tensor strides must be positive.");
            }
            if (capacity < 1)
            {
                throw new ArgumentOutOfRangeException(nameof(capacity), "Tensor capacity must be positive.");
            }

            Width = width;
            Height = height;
            Depth = depth;
            StrideX = strideX;
            StrideY = strideY;
            StrideZ = strideZ;
            Capacity = capacity;

            if (!TryGetPhysicalIndex(width - 1, height - 1, depth - 1, out _))
            {
                throw new ArgumentException("Tensor layout exceeds physical capacity.", nameof(capacity));
            }
        }

        public int Width { get; }
        public int Height { get; }
        public int Depth { get; }
        public int StrideX { get; }
        public int StrideY { get; }
        public int StrideZ { get; }
        public int Capacity { get; }

        public int LogicalElementCount
        {
            get
            {
                return checked(Width * Height * Depth);
            }
        }

        public int LastPhysicalIndex
        {
            get
            {
                TryGetPhysicalIndex(Width - 1, Height - 1, Depth - 1, out var index);
                return index;
            }
        }

        public bool TryGetPhysicalIndex(int x, int y, int z, out int index)
        {
            index = 0;
            if (x < 0 || x >= Width || y < 0 || y >= Height || z < 0 || z >= Depth)
            {
                return false;
            }

            try
            {
                index = checked(x * StrideX + y * StrideY + z * StrideZ);
            }
            catch (OverflowException)
            {
                return false;
            }
            return index >= 0 && index < Capacity;
        }
    }

    /// <summary>Completion path for a 3D affine u32 dispatch.</summary>
    public enum JadrenTensor3DDispatchPath
    {
        Gpu,
        CpuFallback
    }

    /// <summary>Audit-friendly result for one completed 3D dispatch.</summary>
    public readonly struct JadrenTensor3DDispatchReport
    {
        internal JadrenTensor3DDispatchReport(
            string kernelName,
            JadrenTensor3DDispatchPath path,
            JadrenTensor3DLayout layout,
            uint value,
            string fallbackReason)
        {
            KernelName = kernelName;
            Path = path;
            Layout = layout;
            Value = value;
            FallbackReason = fallbackReason ?? string.Empty;
        }

        public string KernelName { get; }
        public JadrenTensor3DDispatchPath Path { get; }
        public JadrenTensor3DLayout Layout { get; }
        public uint Value { get; }
        public string FallbackReason { get; }
        public bool UsedCpuFallback => Path == JadrenTensor3DDispatchPath.CpuFallback;
    }

    /// <summary>
    /// Frame-spanning completion handle for one 3D affine u32 dispatch. The
    /// output lease remains active until Complete or Dispose is called.
    /// </summary>
    public sealed class JadrenTensor3DU32AsyncDispatch : IDisposable
    {
        private readonly JadrenTensor3DDispatchReport report;
        private readonly JadrenNativeArrayAsyncLease<uint> outputLease;
        private readonly Func<bool> isDone;
        private readonly Action completeCore;
        private bool completed;
        private Exception completionError;

        internal JadrenTensor3DU32AsyncDispatch(
            JadrenTensor3DDispatchReport report,
            JadrenNativeArrayAsyncLease<uint> outputLease,
            Func<bool> isDone,
            Action completeCore)
        {
            this.report = report;
            this.outputLease = outputLease;
            this.isDone = isDone;
            this.completeCore = completeCore;
        }

        public JadrenTensor3DDispatchPath Path => report.Path;
        public bool UsedCpuFallback => report.UsedCpuFallback;
        public JadrenTensor3DDispatchReport Report => report;
        public bool IsDone => completed || isDone == null || isDone();

        public JadrenTensor3DDispatchReport Complete()
        {
            if (completed)
            {
                if (completionError != null)
                {
                    throw completionError;
                }
                return report;
            }

            try
            {
                completeCore?.Invoke();
                return report;
            }
            catch (Exception error)
            {
                completionError = error;
                throw;
            }
            finally
            {
                completed = true;
                outputLease?.Dispose();
            }
        }

        public bool TryComplete(out JadrenTensor3DDispatchReport completedReport)
        {
            if (!IsDone)
            {
                completedReport = default(JadrenTensor3DDispatchReport);
                return false;
            }

            completedReport = Complete();
            return true;
        }

        public void Dispose()
        {
            Complete();
            GC.SuppressFinalize(this);
        }

        internal static JadrenTensor3DU32AsyncDispatch Completed(JadrenTensor3DDispatchReport report)
        {
            return new JadrenTensor3DU32AsyncDispatch(report, null, null, null);
        }
    }

    /// <summary>Unity GPU contract for a 3D affine u32 write kernel.</summary>
    public interface IJadrenGpuTensor3DU32Executor
    {
        bool IsAvailable { get; }

        bool TryDispatch(
            NativeArray<uint> output,
            JadrenTensor3DLayout layout,
            uint value,
            JadrenGpuDispatchOptions options,
            out string failureReason);
    }

    /// <summary>
    /// Async GPU contract for a 3D affine u32 write kernel. A successful
    /// implementation takes ownership of the output lease until completion.
    /// </summary>
    public interface IJadrenGpuAsyncTensor3DU32Executor
    {
        bool IsAvailable { get; }

        bool TryDispatchAsync(
            string kernelName,
            JadrenNativeArrayAsyncLease<uint> output,
            JadrenTensor3DLayout layout,
            uint value,
            JadrenGpuDispatchOptions options,
            out JadrenTensor3DU32AsyncDispatch dispatch,
            out string failureReason);
    }

    /// <summary>CPU fallback contract for the same affine write semantics.</summary>
    public interface IJadrenCpuTensor3DU32Fallback
    {
        void Execute(IntPtr output, JadrenTensor3DLayout layout, uint value);
    }

    /// <summary>
    /// Lifetime-safe Unity bridge for the 3D affine-stride contract. The
    /// caller-owned NativeArray remains leased for the whole GPU or fallback
    /// operation and is never resized or disposed by this bridge.
    /// </summary>
    public static class JadrenTensor3DU32Bridge
    {
        public static JadrenTensor3DDispatchReport Dispatch(
            string kernelName,
            JadrenNativeArrayView<uint> output,
            JadrenTensor3DLayout layout,
            uint value,
            JadrenGpuDispatchOptions options,
            IJadrenGpuTensor3DU32Executor gpuExecutor,
            IJadrenCpuTensor3DU32Fallback cpuFallback)
        {
            if (string.IsNullOrEmpty(kernelName))
            {
                throw new ArgumentException("Kernel name must not be empty.", nameof(kernelName));
            }
            if (output == null)
            {
                throw new ArgumentNullException(nameof(output));
            }
            if (layout.Width < 1
                || layout.Height < 1
                || layout.Depth < 1
                || layout.StrideX < 1
                || layout.StrideY < 1
                || layout.StrideZ < 1
                || layout.Capacity < 1
                || !layout.TryGetPhysicalIndex(
                    layout.Width - 1,
                    layout.Height - 1,
                    layout.Depth - 1,
                    out _))
            {
                throw new ArgumentException("Tensor 3D layout is invalid or exceeds capacity.", nameof(layout));
            }
            if (output.Length != layout.Capacity)
            {
                throw new ArgumentException("Output NativeArray length must equal the physical tensor capacity.", nameof(output));
            }

            using (var outputLease = output.Acquire(true))
            {
                var failureReason = string.Empty;
                var executorAvailable = gpuExecutor != null && gpuExecutor.IsAvailable;
                if (options.Target != JadrenGpuTarget.Cpu && executorAvailable)
                {
                    try
                    {
                        if (gpuExecutor.TryDispatch(
                                output.BorrowedArray,
                                layout,
                                value,
                                options,
                                out failureReason))
                        {
                            return new JadrenTensor3DDispatchReport(
                                kernelName,
                                JadrenTensor3DDispatchPath.Gpu,
                                layout,
                                value,
                                string.Empty);
                        }
                    }
                    catch (Exception error)
                    {
                        failureReason = "gpu_executor_exception:" + error.GetType().Name;
                    }
                }

                if (string.IsNullOrEmpty(failureReason))
                {
                    failureReason = options.Target == JadrenGpuTarget.Cpu
                        ? "explicit_cpu"
                        : gpuExecutor == null
                            ? "gpu_executor_missing"
                            : !executorAvailable
                                ? "gpu_executor_unavailable"
                                : "gpu_dispatch_rejected";
                }
                if (options.Target != JadrenGpuTarget.Cpu && !options.AllowCpuFallback)
                {
                    throw new InvalidOperationException(
                        "GPU 3D dispatch was requested but no GPU execution completed: " + failureReason);
                }
                if (cpuFallback == null)
                {
                    throw new InvalidOperationException(
                        "CPU 3D fallback is required for this dispatch: " + failureReason);
                }

                cpuFallback.Execute(outputLease.Pointer, layout, value);
                return new JadrenTensor3DDispatchReport(
                    kernelName,
                    JadrenTensor3DDispatchPath.CpuFallback,
                    layout,
                    value,
                    failureReason);
            }
        }

        public static JadrenTensor3DU32AsyncDispatch DispatchAsync(
            string kernelName,
            JadrenNativeArrayView<uint> output,
            JadrenTensor3DLayout layout,
            uint value,
            JadrenGpuDispatchOptions options,
            IJadrenGpuAsyncTensor3DU32Executor gpuExecutor,
            IJadrenCpuTensor3DU32Fallback cpuFallback)
        {
            if (string.IsNullOrEmpty(kernelName))
            {
                throw new ArgumentException("Kernel name must not be empty.", nameof(kernelName));
            }
            if (output == null)
            {
                throw new ArgumentNullException(nameof(output));
            }
            if (!layout.TryGetPhysicalIndex(
                    layout.Width - 1,
                    layout.Height - 1,
                    layout.Depth - 1,
                    out _)
                || layout.Width < 1
                || layout.Height < 1
                || layout.Depth < 1
                || layout.StrideX < 1
                || layout.StrideY < 1
                || layout.StrideZ < 1
                || layout.Capacity < 1)
            {
                throw new ArgumentException("Tensor 3D layout is invalid or exceeds capacity.", nameof(layout));
            }
            if (output.Length != layout.Capacity)
            {
                throw new ArgumentException("Output NativeArray length must equal the physical tensor capacity.", nameof(output));
            }

            var outputLease = output.AcquireAsync(true);
            var transferred = false;
            try
            {
                var failureReason = string.Empty;
                var executorAvailable = gpuExecutor != null && gpuExecutor.IsAvailable;
                if (options.Target != JadrenGpuTarget.Cpu && executorAvailable)
                {
                    try
                    {
                        if (gpuExecutor.TryDispatchAsync(
                                kernelName,
                                outputLease,
                                layout,
                                value,
                                options,
                                out var dispatch,
                                out failureReason)
                            && dispatch != null)
                        {
                            transferred = true;
                            return dispatch;
                        }
                    }
                    catch (Exception error)
                    {
                        failureReason = "gpu_executor_exception:" + error.GetType().Name;
                    }
                }

                if (string.IsNullOrEmpty(failureReason))
                {
                    failureReason = options.Target == JadrenGpuTarget.Cpu
                        ? "explicit_cpu"
                        : gpuExecutor == null
                            ? "gpu_executor_missing"
                            : !executorAvailable
                                ? "gpu_executor_unavailable"
                                : "gpu_dispatch_rejected";
                }
                if (options.Target != JadrenGpuTarget.Cpu && !options.AllowCpuFallback)
                {
                    throw new InvalidOperationException(
                        "GPU 3D dispatch was requested but no GPU execution started: " + failureReason);
                }
                if (cpuFallback == null)
                {
                    throw new InvalidOperationException(
                        "CPU 3D fallback is required for this dispatch: " + failureReason);
                }

                cpuFallback.Execute(outputLease.Pointer, layout, value);
                var report = new JadrenTensor3DDispatchReport(
                    kernelName,
                    JadrenTensor3DDispatchPath.CpuFallback,
                    layout,
                    value,
                    failureReason);
                outputLease.Dispose();
                transferred = true;
                return JadrenTensor3DU32AsyncDispatch.Completed(report);
            }
            finally
            {
                if (!transferred)
                {
                    outputLease.Dispose();
                }
            }
        }
    }

    /// <summary>Reference fallback used by samples and deterministic tests.</summary>
    public sealed class JadrenCpuTensor3DU32Fallback : IJadrenCpuTensor3DU32Fallback
    {
        public void Execute(IntPtr output, JadrenTensor3DLayout layout, uint value)
        {
            if (output == IntPtr.Zero)
            {
                throw new ArgumentException("Output pointer must be non-null.", nameof(output));
            }

            for (var index = 0; index < layout.Capacity; index++)
            {
                Marshal.WriteInt32(output, checked(index * sizeof(uint)), 0);
            }
            for (var z = 0; z < layout.Depth; z++)
            {
                for (var y = 0; y < layout.Height; y++)
                {
                    for (var x = 0; x < layout.Width; x++)
                    {
                        if (layout.TryGetPhysicalIndex(x, y, z, out var physicalIndex))
                        {
                            Marshal.WriteInt32(
                                output,
                                checked(physicalIndex * sizeof(uint)),
                                unchecked((int)value));
                        }
                    }
                }
            }
        }
    }
}
