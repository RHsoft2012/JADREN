using System;
using System.Collections.Concurrent;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using UnityEngine.Profiling;

namespace Jadren.Unity
{
    /// <summary>
    /// Optional runtime profiler bridge. Registration happens before worker start;
    /// callback delegates are static and held for the lifetime of the process.
    /// </summary>
    public static class JadrenProfilerBridge
    {
        private static readonly ConcurrentDictionary<ulong, string> Samples =
            new ConcurrentDictionary<ulong, string>();
        private static readonly LogCallback Log = OnLog;
        private static readonly BeginCallback Begin = OnBegin;
        private static readonly EndCallback End = OnEnd;
        private static readonly CounterCallback Counter = OnCounter;
        private static Action<ulong, long> counterSink;
        private static Action<uint, string> logSink;
        private static int enabled;

        public static bool IsEnabled => Volatile.Read(ref enabled) != 0;

        /// <summary>Optional preinstalled sink for counter values; no sink means no-op.</summary>
        public static Action<ulong, long> CounterSink
        {
            get => Volatile.Read(ref counterSink);
            set => Volatile.Write(ref counterSink, value);
        }

        /// <summary>Optional console sink. It is invoked only when explicitly installed.</summary>
        public static Action<uint, string> LogSink
        {
            get => Volatile.Read(ref logSink);
            set => Volatile.Write(ref logSink, value);
        }

        public static bool RegisterSample(ulong nameId, string name)
        {
            if (nameId == 0 || string.IsNullOrEmpty(name))
            {
                return false;
            }
            Samples[nameId] = name;
            return true;
        }

        public static bool TryEnable(out int status)
        {
            if (IsEnabled)
            {
                status = 0;
                return true;
            }
            JadrenAbiVersion abi;
            if (!JadrenRuntime.TryInitialize(out abi, out status))
            {
                return false;
            }

            try
            {
                status = Native.jadren_rt_set_callbacks(Log, Begin, End, Counter, IntPtr.Zero);
                if (status == 0)
                {
                    Volatile.Write(ref enabled, 1);
                    return true;
                }
                return false;
            }
            catch (DllNotFoundException)
            {
                status = JadrenRuntime.NativeUnavailable;
                return false;
            }
            catch (EntryPointNotFoundException)
            {
                status = JadrenRuntime.NativeUnavailable;
                return false;
            }
            catch (BadImageFormatException)
            {
                status = JadrenRuntime.NativeUnavailable;
                return false;
            }
        }

        public static bool TryDisable(out int status)
        {
            if (!IsEnabled)
            {
                status = 1;
                return true;
            }
            try
            {
                status = Native.jadren_rt_set_callbacks(null, null, null, null, IntPtr.Zero);
                Volatile.Write(ref enabled, 0);
                return status == 0 || status == 1;
            }
            catch (DllNotFoundException)
            {
                status = JadrenRuntime.NativeUnavailable;
                return false;
            }
            catch (EntryPointNotFoundException)
            {
                status = JadrenRuntime.NativeUnavailable;
                return false;
            }
            catch (BadImageFormatException)
            {
                status = JadrenRuntime.NativeUnavailable;
                return false;
            }
        }

        [AOT.MonoPInvokeCallback(typeof(BeginCallback))]
        private static void OnBegin(ulong nameId, IntPtr context)
        {
            if (Samples.TryGetValue(nameId, out var name))
            {
                Profiler.BeginSample(name);
            }
        }

        [AOT.MonoPInvokeCallback(typeof(LogCallback))]
        private static void OnLog(uint level, IntPtr message, ulong length, IntPtr context)
        {
            var sink = Volatile.Read(ref logSink);
            if (sink == null || length > int.MaxValue || (length != 0 && message == IntPtr.Zero))
            {
                return;
            }
            try
            {
                var bytes = new byte[(int)length];
                if (bytes.Length != 0)
                {
                    Marshal.Copy(message, bytes, 0, bytes.Length);
                }
                sink(level, new UTF8Encoding(false, true).GetString(bytes));
            }
            catch (Exception)
            {
                // A host callback must never unwind through the C ABI.
            }
        }

        [AOT.MonoPInvokeCallback(typeof(EndCallback))]
        private static void OnEnd(IntPtr context)
        {
            Profiler.EndSample();
        }

        [AOT.MonoPInvokeCallback(typeof(CounterCallback))]
        private static void OnCounter(ulong nameId, long value, IntPtr context)
        {
            Volatile.Read(ref counterSink)?.Invoke(nameId, value);
        }

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate void LogCallback(uint level, IntPtr message, ulong length, IntPtr context);

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate void BeginCallback(ulong nameId, IntPtr context);

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate void EndCallback(IntPtr context);

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate void CounterCallback(ulong nameId, long value, IntPtr context);

        private static class Native
        {
            [DllImport("jadren_runtime", CallingConvention = CallingConvention.Cdecl,
                EntryPoint = "jadren_rt_set_callbacks")]
            internal static extern int jadren_rt_set_callbacks(
                LogCallback log,
                BeginCallback profilerBegin,
                EndCallback profilerEnd,
                CounterCallback profilerCounter,
                IntPtr context);
        }
    }
}
