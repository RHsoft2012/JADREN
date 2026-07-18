using System;
using Unity.Collections;
using Jadren.Unity;

namespace Jadren.Unity.Samples.AgentSimulation
{
    /// <summary>
    /// Deterministic NativeArray reference simulation for JAD-1011.
    ///
    /// This is deliberately a managed host baseline. The borrowed view proves
    /// the same ownership/lifetime boundary that a generated batch kernel will
    /// use once Slice/Buffer kernel lowering is available.
    /// </summary>
    public sealed class AgentSimulationWorld : IDisposable
    {
        private NativeArray<AgentState> agents;
        private readonly JadrenNativeArrayView<AgentState> borrowedView;
        private bool disposed;

        public AgentSimulationWorld(int count, Allocator allocator)
        {
            if (count < 0)
            {
                throw new ArgumentOutOfRangeException(nameof(count));
            }

            agents = new NativeArray<AgentState>(count, allocator);
            borrowedView = new JadrenNativeArrayView<AgentState>(agents);
            Reset();
        }

        public int Count
        {
            get
            {
                ThrowIfDisposed();
                return agents.Length;
            }
        }

        /// <summary>Returns a copy of the caller-owned NativeArray handle.</summary>
        public NativeArray<AgentState> Agents
        {
            get
            {
                ThrowIfDisposed();
                return agents;
            }
        }

        public void Reset()
        {
            ThrowIfDisposed();
            AgentSimulationWorkload.Initialize(agents);
        }

        public void Step(float deltaTime)
        {
            ThrowIfDisposed();
            if (float.IsNaN(deltaTime) || float.IsInfinity(deltaTime) || deltaTime < 0.0f)
            {
                throw new ArgumentOutOfRangeException(nameof(deltaTime));
            }

            // Keep the borrowed lease alive across the complete future native
            // call boundary. The baseline still uses the NativeArray indexer.
            using (var lease = borrowedView.Acquire(writable: true))
            {
                _ = lease.Pointer;
                AgentSimulationWorkload.StepManaged(agents, deltaTime);
            }
        }

        public double Checksum()
        {
            ThrowIfDisposed();
            return AgentSimulationWorkload.Checksum(agents);
        }

        public void Dispose()
        {
            if (disposed)
            {
                return;
            }

            borrowedView.Dispose();
            if (agents.IsCreated)
            {
                agents.Dispose();
            }
            disposed = true;
        }

        private void ThrowIfDisposed()
        {
            if (disposed)
            {
                throw new ObjectDisposedException(nameof(AgentSimulationWorld));
            }
        }
    }
}
