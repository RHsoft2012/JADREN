using System;
using Unity.Collections;

namespace Jadren.Unity
{
    /// <summary>Explicit route requested by a Unity host.</summary>
    public enum JadrenGpuTarget
    {
        Cpu,
        Gpu,
        Auto
    }

    /// <summary>Floating-point contract passed to a GPU executor.</summary>
    public enum JadrenGpuFpPolicy
    {
        Strict,
        Fast,
        Deterministic
    }

    /// <summary>
    /// Immutable dispatch policy. GPU selection is never implicit in this
    /// bridge: the host chooses the target, precision contract and fallback.
    /// </summary>
    public readonly struct JadrenGpuDispatchOptions
    {
        public JadrenGpuDispatchOptions(
            JadrenGpuTarget target,
            JadrenGpuFpPolicy fp,
            int workgroupSize,
            bool allowCpuFallback)
        {
            if (workgroupSize < 1)
            {
                throw new ArgumentOutOfRangeException(nameof(workgroupSize), "Workgroup size must be positive.");
            }

            Target = target;
            Fp = fp;
            WorkgroupSize = workgroupSize;
            AllowCpuFallback = allowCpuFallback;
        }

        public JadrenGpuTarget Target { get; }
        public JadrenGpuFpPolicy Fp { get; }
        public int WorkgroupSize { get; }
        public bool AllowCpuFallback { get; }

        public static JadrenGpuDispatchOptions Default
        {
            get
            {
                return new JadrenGpuDispatchOptions(
                    JadrenGpuTarget.Auto,
                    JadrenGpuFpPolicy.Deterministic,
                    64,
                    true);
            }
        }
    }

    /// <summary>Observed completion path for one bridge call.</summary>
    public enum JadrenGpuDispatchPath
    {
        Gpu,
        CpuFallback
    }

    /// <summary>Audit-friendly result of a completed dispatch.</summary>
    public readonly struct JadrenGpuDispatchReport
    {
        internal JadrenGpuDispatchReport(
            string kernelName,
            JadrenGpuDispatchPath path,
            int elementCount,
            int elementSize,
            string fallbackReason)
        {
            KernelName = kernelName;
            Path = path;
            ElementCount = elementCount;
            ElementSize = elementSize;
            FallbackReason = fallbackReason ?? string.Empty;
        }

        public string KernelName { get; }
        public JadrenGpuDispatchPath Path { get; }
        public int ElementCount { get; }
        public int ElementSize { get; }
        public string FallbackReason { get; }
        public bool UsedCpuFallback => Path == JadrenGpuDispatchPath.CpuFallback;
    }

    /// <summary>
    /// Frame-safe completion handle for one GPU dispatch. NativeArray leases
    /// and temporary GPU resources remain owned until Complete/Dispose.
    /// </summary>
    public sealed class JadrenGpuAsyncDispatch<T> : IDisposable where T : unmanaged
    {
        private readonly JadrenGpuDispatchReport report;
        private readonly JadrenNativeArrayAsyncLease<T> inputLease;
        private readonly JadrenNativeArrayAsyncLease<T> outputLease;
        private readonly Func<bool> isDone;
        private readonly Action completeCore;
        private bool completed;
        private Exception completionError;

        internal JadrenGpuAsyncDispatch(
            JadrenGpuDispatchReport report,
            JadrenNativeArrayAsyncLease<T> inputLease,
            JadrenNativeArrayAsyncLease<T> outputLease,
            Func<bool> isDone,
            Action completeCore)
        {
            this.report = report;
            this.inputLease = inputLease;
            this.outputLease = outputLease;
            this.isDone = isDone;
            this.completeCore = completeCore;
        }

        public JadrenGpuDispatchPath Path => report.Path;
        public bool UsedCpuFallback => report.UsedCpuFallback;
        public bool IsDone => completed || isDone == null || isDone();
        public JadrenGpuDispatchReport Report => report;

        public JadrenGpuDispatchReport Complete()
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
                inputLease?.Dispose();
                outputLease?.Dispose();
            }
        }

        public bool TryComplete(out JadrenGpuDispatchReport completedReport)
        {
            if (!IsDone)
            {
                completedReport = default(JadrenGpuDispatchReport);
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

        internal static JadrenGpuAsyncDispatch<T> Completed(JadrenGpuDispatchReport report)
        {
            return new JadrenGpuAsyncDispatch<T>(report, null, null, null, null);
        }
    }

    /// <summary>
    /// Unity-side hook for a real Vulkan/graphics executor. Implementations
    /// must keep the supplied pointers borrowed until TryDispatch returns and
    /// must report a controlled failure instead of retaining them.
    /// </summary>
    public interface IJadrenGpuExecutor
    {
        bool IsAvailable { get; }

        bool TryDispatch(
            IntPtr input,
            IntPtr output,
            int elementCount,
            int elementSize,
            JadrenGpuDispatchOptions options,
            out string failureReason);
    }

    /// <summary>
    /// GPU executor contract for Unity APIs that require the NativeArray
    /// safety handle rather than a raw pointer (for example ComputeBuffer).
    /// The array values are borrowed and must not be retained.
    /// </summary>
    public interface IJadrenGpuNativeArrayExecutor<T> where T : unmanaged
    {
        bool IsAvailable { get; }

        bool TryDispatch(
            NativeArray<T> input,
            NativeArray<T> output,
            int elementCount,
            JadrenGpuDispatchOptions options,
            out string failureReason);
    }

    /// <summary>
    /// Async executor contract. A successful implementation takes ownership
    /// of both async leases and must return them through its completion handle.
    /// </summary>
    public interface IJadrenGpuAsyncNativeArrayExecutor<T> where T : unmanaged
    {
        bool IsAvailable { get; }

        bool TryDispatchAsync(
            string kernelName,
            JadrenNativeArrayAsyncLease<T> input,
            JadrenNativeArrayAsyncLease<T> output,
            int elementCount,
            JadrenGpuDispatchOptions options,
            out JadrenGpuAsyncDispatch<T> dispatch,
            out string failureReason);
    }

    /// <summary>
    /// CPU fallback ABI for one unmanaged element type. The pointers are valid
    /// only for the duration of Execute and must never be stored.
    /// </summary>
    public interface IJadrenCpuFallback<T> where T : unmanaged
    {
        void Execute(IntPtr input, IntPtr output, int elementCount);
    }

    /// <summary>
    /// Lifetime-safe Unity bridge over borrowed NativeArray views. It owns no
    /// arrays, never copies managed data and keeps both leases alive through
    /// either GPU completion or CPU fallback execution.
    /// </summary>
    public static class JadrenGpuBridge
    {
        private static void Validate<T>(
            string kernelName,
            JadrenNativeArrayView<T> input,
            JadrenNativeArrayView<T> output)
            where T : unmanaged
        {
            if (string.IsNullOrEmpty(kernelName))
            {
                throw new ArgumentException("Kernel name must not be empty.", nameof(kernelName));
            }
            if (input == null)
            {
                throw new ArgumentNullException(nameof(input));
            }
            if (output == null)
            {
                throw new ArgumentNullException(nameof(output));
            }
            if (input.Length != output.Length)
            {
                throw new ArgumentException("Input and output NativeArrays must have equal lengths.");
            }
            if (input.Length < 1)
            {
                throw new ArgumentException("GPU bridge does not dispatch an empty buffer.");
            }
            if (input.ElementSize != output.ElementSize)
            {
                throw new ArgumentException("Input and output element layouts must match.");
            }
        }

        public static JadrenGpuDispatchReport Dispatch<T>(
            string kernelName,
            JadrenNativeArrayView<T> input,
            JadrenNativeArrayView<T> output,
            JadrenGpuDispatchOptions options,
            IJadrenGpuExecutor gpuExecutor,
            IJadrenCpuFallback<T> cpuFallback)
            where T : unmanaged
        {
            Validate(kernelName, input, output);

            using (var inputLease = input.Acquire(false))
            using (var outputLease = output.Acquire(true))
            {
                var failureReason = string.Empty;
                var executorAvailable = gpuExecutor != null && gpuExecutor.IsAvailable;
                if (options.Target != JadrenGpuTarget.Cpu
                    && executorAvailable)
                {
                    try
                    {
                        if (gpuExecutor.TryDispatch(
                                inputLease.Pointer,
                                outputLease.Pointer,
                                inputLease.Length,
                                inputLease.ElementSize,
                                options,
                                out failureReason))
                        {
                            return new JadrenGpuDispatchReport(
                                kernelName,
                                JadrenGpuDispatchPath.Gpu,
                                inputLease.Length,
                                inputLease.ElementSize,
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
                        "GPU dispatch was requested but no GPU execution completed: " + failureReason);
                }
                if (cpuFallback == null)
                {
                    throw new InvalidOperationException(
                        "CPU fallback is required for this dispatch: " + failureReason);
                }

                cpuFallback.Execute(inputLease.Pointer, outputLease.Pointer, inputLease.Length);
                return new JadrenGpuDispatchReport(
                    kernelName,
                    JadrenGpuDispatchPath.CpuFallback,
                    inputLease.Length,
                    inputLease.ElementSize,
                    failureReason);
            }
        }

        public static JadrenGpuAsyncDispatch<T> DispatchAsync<T>(
            string kernelName,
            JadrenNativeArrayView<T> input,
            JadrenNativeArrayView<T> output,
            JadrenGpuDispatchOptions options,
            IJadrenGpuAsyncNativeArrayExecutor<T> gpuExecutor,
            IJadrenCpuFallback<T> cpuFallback)
            where T : unmanaged
        {
            Validate(kernelName, input, output);
            var inputLease = input.AcquireAsync(false);
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
                                inputLease,
                                outputLease,
                                inputLease.Length,
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
                        "GPU dispatch was requested but no GPU execution started: " + failureReason);
                }
                if (cpuFallback == null)
                {
                    throw new InvalidOperationException(
                        "CPU fallback is required for this dispatch: " + failureReason);
                }

                cpuFallback.Execute(inputLease.Pointer, outputLease.Pointer, inputLease.Length);
                var report = new JadrenGpuDispatchReport(
                    kernelName,
                    JadrenGpuDispatchPath.CpuFallback,
                    inputLease.Length,
                    inputLease.ElementSize,
                    failureReason);
                inputLease.Dispose();
                outputLease.Dispose();
                transferred = true;
                return JadrenGpuAsyncDispatch<T>.Completed(report);
            }
            finally
            {
                if (!transferred)
                {
                    inputLease.Dispose();
                    outputLease.Dispose();
                }
            }
        }

        public static JadrenGpuDispatchReport Dispatch<T>(
            string kernelName,
            JadrenNativeArrayView<T> input,
            JadrenNativeArrayView<T> output,
            JadrenGpuDispatchOptions options,
            IJadrenGpuNativeArrayExecutor<T> gpuExecutor,
            IJadrenCpuFallback<T> cpuFallback)
            where T : unmanaged
        {
            Validate(kernelName, input, output);

            using (var inputLease = input.Acquire(false))
            using (var outputLease = output.Acquire(true))
            {
                var failureReason = string.Empty;
                var executorAvailable = gpuExecutor != null && gpuExecutor.IsAvailable;
                if (options.Target != JadrenGpuTarget.Cpu && executorAvailable)
                {
                    try
                    {
                        if (gpuExecutor.TryDispatch(
                                input.BorrowedArray,
                                output.BorrowedArray,
                                inputLease.Length,
                                options,
                                out failureReason))
                        {
                            return new JadrenGpuDispatchReport(
                                kernelName,
                                JadrenGpuDispatchPath.Gpu,
                                inputLease.Length,
                                inputLease.ElementSize,
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
                        "GPU dispatch was requested but no GPU execution completed: " + failureReason);
                }
                if (cpuFallback == null)
                {
                    throw new InvalidOperationException(
                        "CPU fallback is required for this dispatch: " + failureReason);
                }

                cpuFallback.Execute(inputLease.Pointer, outputLease.Pointer, inputLease.Length);
                return new JadrenGpuDispatchReport(
                    kernelName,
                    JadrenGpuDispatchPath.CpuFallback,
                    inputLease.Length,
                    inputLease.ElementSize,
                    failureReason);
            }
        }
    }
}
