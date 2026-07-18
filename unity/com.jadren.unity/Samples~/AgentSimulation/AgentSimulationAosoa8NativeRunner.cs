using System;
using System.Runtime.InteropServices;
using Jadren.Unity;
using Unity.Collections;

namespace Jadren.Unity.Samples.AgentSimulation
{
    /// <summary>One zero-copy native call over caller-owned AoSoA8 tiles.</summary>
    public sealed class AgentSimulationAosoa8NativeRunner : IDisposable
    {
        private const int AgentTileByteSize = 192;
        private readonly JadrenNativeArrayView<AgentTile> view;
        private bool disposed;

        public AgentSimulationAosoa8NativeRunner(AgentSimulationAosoa8State agents)
        {
            view = new JadrenNativeArrayView<AgentTile>(agents.Tiles);
            if (view.ElementSize != AgentTileByteSize)
            {
                view.Dispose();
                throw new InvalidOperationException(
                    $"AgentTile must remain six Float8 fields (expected {AgentTileByteSize} bytes, got {view.ElementSize}).");
            }
        }

        public void Step(float deltaTime)
        {
            ThrowIfDisposed();
            if (float.IsNaN(deltaTime) || float.IsInfinity(deltaTime) || deltaTime < 0.0f)
            {
                throw new ArgumentOutOfRangeException(nameof(deltaTime));
            }

            using (var lease = view.Acquire(writable: true))
            {
                StepNative(lease.Pointer, new UIntPtr((uint)lease.Length), deltaTime);
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
                throw new ObjectDisposedException(nameof(AgentSimulationAosoa8NativeRunner));
            }
        }

        [DllImport(
            "jadren_native",
            EntryPoint = "jadren_agent_step_batch_aosoa8",
            CallingConvention = CallingConvention.Cdecl,
            ExactSpelling = true)]
        private static extern void StepNative(
            IntPtr tilesPointer,
            UIntPtr tilesLength,
            float deltaTime);
    }
}
