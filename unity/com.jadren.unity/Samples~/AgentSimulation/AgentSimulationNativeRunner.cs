using System;
using System.Runtime.InteropServices;
using Jadren.Unity;
using Unity.Collections;

namespace Jadren.Unity.Samples.AgentSimulation
{
    /// <summary>
    /// One zero-copy batch boundary from a caller-owned NativeArray to the
    /// Jadren AgentState slice export. The runner never owns or resizes agents.
    /// </summary>
    public sealed class AgentSimulationNativeRunner : IDisposable
    {
        private const int AgentStateByteSize = 24;

        private readonly JadrenNativeArrayView<AgentState> view;
        private bool disposed;

        public AgentSimulationNativeRunner(NativeArray<AgentState> agents)
        {
            view = new JadrenNativeArrayView<AgentState>(agents);
            if (view.ElementSize != AgentStateByteSize)
            {
                view.Dispose();
                throw new InvalidOperationException(
                    "AgentState must remain six sequential Float32 fields for the Jadren batch ABI.");
            }
        }

        public int Count
        {
            get
            {
                ThrowIfDisposed();
                return view.Length;
            }
        }

        /// <summary>Updates every agent through one synchronous native call.</summary>
        public void Step(float deltaTime)
        {
            ThrowIfDisposed();
            if (float.IsNaN(deltaTime) || float.IsInfinity(deltaTime) || deltaTime < 0.0f)
            {
                throw new ArgumentOutOfRangeException(nameof(deltaTime));
            }

            using (var lease = view.Acquire(writable: true))
            {
                StepBatchNative(lease.Pointer, new UIntPtr((uint)lease.Length), deltaTime);
            }
        }

        public void Dispose()
        {
            if (disposed)
            {
                return;
            }

            view.Dispose();
            disposed = true;
            GC.SuppressFinalize(this);
        }

        private void ThrowIfDisposed()
        {
            if (disposed)
            {
                throw new ObjectDisposedException(nameof(AgentSimulationNativeRunner));
            }
        }

        [DllImport(
            "jadren_native",
            EntryPoint = "jadren_agent_step_batch",
            CallingConvention = CallingConvention.Cdecl,
            ExactSpelling = true)]
        private static extern void StepBatchNative(
            IntPtr agentsPointer,
            UIntPtr agentsLength,
            float deltaTime);
    }
}
