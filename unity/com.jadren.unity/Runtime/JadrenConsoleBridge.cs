using System;

namespace Jadren.Unity
{
    /// <summary>
    /// Console diagnostics facade sharing the profiler callback table.
    /// The caller owns the sink and may enqueue messages for Unity's main thread.
    /// </summary>
    public static class JadrenConsoleBridge
    {
        public static Action<uint, string> Sink
        {
            get => JadrenProfilerBridge.LogSink;
            set => JadrenProfilerBridge.LogSink = value;
        }

        public static bool TryEnable(out int status)
        {
            return JadrenProfilerBridge.TryEnable(out status);
        }

        public static bool TryDisable(out int status)
        {
            return JadrenProfilerBridge.TryDisable(out status);
        }
    }
}
