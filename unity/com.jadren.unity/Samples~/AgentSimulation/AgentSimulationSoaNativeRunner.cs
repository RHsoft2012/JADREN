using System;
using System.Runtime.InteropServices;
using Jadren.Unity;

namespace Jadren.Unity.Samples.AgentSimulation
{
    /// <summary>
    /// One zero-copy C ABI call over six explicit SoA borrowed slices.
    /// Writable position arrays are exclusive; velocity arrays are read-only.
    /// </summary>
    public sealed class AgentSimulationSoaNativeRunner : IDisposable
    {
        private readonly JadrenNativeArrayView<float> positionX;
        private readonly JadrenNativeArrayView<float> positionY;
        private readonly JadrenNativeArrayView<float> positionZ;
        private readonly JadrenNativeArrayView<float> velocityX;
        private readonly JadrenNativeArrayView<float> velocityY;
        private readonly JadrenNativeArrayView<float> velocityZ;
        private readonly int simdLanes;
        private bool disposed;

        public AgentSimulationSoaNativeRunner(AgentSimulationSoaState agents, int simdLanes = 0)
        {
            if (simdLanes != 0 && simdLanes != 4 && simdLanes != 8)
            {
                throw new ArgumentOutOfRangeException(nameof(simdLanes));
            }
            this.simdLanes = simdLanes;
            positionX = new JadrenNativeArrayView<float>(agents.PositionX);
            positionY = new JadrenNativeArrayView<float>(agents.PositionY);
            positionZ = new JadrenNativeArrayView<float>(agents.PositionZ);
            velocityX = new JadrenNativeArrayView<float>(agents.VelocityX);
            velocityY = new JadrenNativeArrayView<float>(agents.VelocityY);
            velocityZ = new JadrenNativeArrayView<float>(agents.VelocityZ);
            if (positionX.Length != positionY.Length ||
                positionX.Length != positionZ.Length ||
                positionX.Length != velocityX.Length ||
                positionX.Length != velocityY.Length ||
                positionX.Length != velocityZ.Length)
            {
                Dispose();
                throw new ArgumentException("all SoA component arrays must have the same length", nameof(agents));
            }

            try
            {
                using (var positionXLease = positionX.Acquire(writable: true))
                using (var positionYLease = positionY.Acquire(writable: true))
                using (var positionZLease = positionZ.Acquire(writable: true))
                using (var velocityXLease = velocityX.Acquire(writable: false))
                using (var velocityYLease = velocityY.Acquire(writable: false))
                using (var velocityZLease = velocityZ.Acquire(writable: false))
                {
                    ValidateDisjointRanges(
                        positionXLease.Pointer, positionXLease.Length, "position_x",
                        positionYLease.Pointer, positionYLease.Length, "position_y",
                        positionZLease.Pointer, positionZLease.Length, "position_z",
                        velocityXLease.Pointer, velocityXLease.Length, "velocity_x",
                        velocityYLease.Pointer, velocityYLease.Length, "velocity_y",
                        velocityZLease.Pointer, velocityZLease.Length, "velocity_z");
                }
            }
            catch
            {
                Dispose();
                throw;
            }
        }

        public void Step(float deltaTime)
        {
            ThrowIfDisposed();
            if (float.IsNaN(deltaTime) || float.IsInfinity(deltaTime) || deltaTime < 0.0f)
            {
                throw new ArgumentOutOfRangeException(nameof(deltaTime));
            }

            using (var positionXLease = positionX.Acquire(writable: true))
            using (var positionYLease = positionY.Acquire(writable: true))
            using (var positionZLease = positionZ.Acquire(writable: true))
            using (var velocityXLease = velocityX.Acquire(writable: false))
            using (var velocityYLease = velocityY.Acquire(writable: false))
            using (var velocityZLease = velocityZ.Acquire(writable: false))
            {
                var positionXLength = new UIntPtr((uint)positionXLease.Length);
                var positionYLength = new UIntPtr((uint)positionYLease.Length);
                var positionZLength = new UIntPtr((uint)positionZLease.Length);
                var velocityXLength = new UIntPtr((uint)velocityXLease.Length);
                var velocityYLength = new UIntPtr((uint)velocityYLease.Length);
                var velocityZLength = new UIntPtr((uint)velocityZLease.Length);
                if (simdLanes == 4)
                {
                    StepBatchSimdNative(
                        positionXLease.Pointer, positionXLength,
                        positionYLease.Pointer, positionYLength,
                        positionZLease.Pointer, positionZLength,
                        velocityXLease.Pointer, velocityXLength,
                        velocityYLease.Pointer, velocityYLength,
                        velocityZLease.Pointer, velocityZLength,
                        new UIntPtr((uint)positionXLease.Length), deltaTime);
                }
                else if (simdLanes == 8)
                {
                    StepBatchSimd8Native(
                        positionXLease.Pointer, positionXLength,
                        positionYLease.Pointer, positionYLength,
                        positionZLease.Pointer, positionZLength,
                        velocityXLease.Pointer, velocityXLength,
                        velocityYLease.Pointer, velocityYLength,
                        velocityZLease.Pointer, velocityZLength,
                        new UIntPtr((uint)positionXLease.Length), deltaTime);
                }
                else
                {
                    StepBatchNative(
                        positionXLease.Pointer, positionXLength,
                        positionYLease.Pointer, positionYLength,
                        positionZLease.Pointer, positionZLength,
                        velocityXLease.Pointer, velocityXLength,
                        velocityYLease.Pointer, velocityYLength,
                        velocityZLease.Pointer, velocityZLength,
                        deltaTime);
                }
            }
        }

        public void Dispose()
        {
            if (disposed)
            {
                return;
            }

            positionX.Dispose();
            positionY.Dispose();
            positionZ.Dispose();
            velocityX.Dispose();
            velocityY.Dispose();
            velocityZ.Dispose();
            disposed = true;
            GC.SuppressFinalize(this);
        }

        private void ThrowIfDisposed()
        {
            if (disposed)
            {
                throw new ObjectDisposedException(nameof(AgentSimulationSoaNativeRunner));
            }
        }

        private static void ValidateDisjointRanges(
            IntPtr positionXPointer, int positionXLength, string positionXName,
            IntPtr positionYPointer, int positionYLength, string positionYName,
            IntPtr positionZPointer, int positionZLength, string positionZName,
            IntPtr velocityXPointer, int velocityXLength, string velocityXName,
            IntPtr velocityYPointer, int velocityYLength, string velocityYName,
            IntPtr velocityZPointer, int velocityZLength, string velocityZName)
        {
            EnsureDisjoint(positionXPointer, positionXLength, positionXName, positionYPointer, positionYLength, positionYName);
            EnsureDisjoint(positionXPointer, positionXLength, positionXName, positionZPointer, positionZLength, positionZName);
            EnsureDisjoint(positionXPointer, positionXLength, positionXName, velocityXPointer, velocityXLength, velocityXName);
            EnsureDisjoint(positionXPointer, positionXLength, positionXName, velocityYPointer, velocityYLength, velocityYName);
            EnsureDisjoint(positionXPointer, positionXLength, positionXName, velocityZPointer, velocityZLength, velocityZName);
            EnsureDisjoint(positionYPointer, positionYLength, positionYName, positionZPointer, positionZLength, positionZName);
            EnsureDisjoint(positionYPointer, positionYLength, positionYName, velocityXPointer, velocityXLength, velocityXName);
            EnsureDisjoint(positionYPointer, positionYLength, positionYName, velocityYPointer, velocityYLength, velocityYName);
            EnsureDisjoint(positionYPointer, positionYLength, positionYName, velocityZPointer, velocityZLength, velocityZName);
            EnsureDisjoint(positionZPointer, positionZLength, positionZName, velocityXPointer, velocityXLength, velocityXName);
            EnsureDisjoint(positionZPointer, positionZLength, positionZName, velocityYPointer, velocityYLength, velocityYName);
            EnsureDisjoint(positionZPointer, positionZLength, positionZName, velocityZPointer, velocityZLength, velocityZName);
            EnsureDisjoint(velocityXPointer, velocityXLength, velocityXName, velocityYPointer, velocityYLength, velocityYName);
            EnsureDisjoint(velocityXPointer, velocityXLength, velocityXName, velocityZPointer, velocityZLength, velocityZName);
            EnsureDisjoint(velocityYPointer, velocityYLength, velocityYName, velocityZPointer, velocityZLength, velocityZName);
        }

        private static void EnsureDisjoint(
            IntPtr leftPointer, int leftLength, string leftName,
            IntPtr rightPointer, int rightLength, string rightName)
        {
            if (leftLength < 0 || rightLength < 0)
            {
                throw new ArgumentOutOfRangeException("SoA array lengths must be non-negative.");
            }
            if (RangesOverlap(leftPointer, leftLength, rightPointer, rightLength))
            {
                throw new ArgumentException(
                    $"SoA arrays `{leftName}` and `{rightName}` must not overlap.");
            }
        }

        private static bool RangesOverlap(IntPtr leftPointer, int leftLength, IntPtr rightPointer, int rightLength)
        {
            if (leftLength == 0 || rightLength == 0)
            {
                return false;
            }

            var leftStart = unchecked((ulong)leftPointer.ToInt64());
            var rightStart = unchecked((ulong)rightPointer.ToInt64());
            var leftBytes = checked((ulong)leftLength * sizeof(float));
            var rightBytes = checked((ulong)rightLength * sizeof(float));
            var leftEnd = checked(leftStart + leftBytes);
            var rightEnd = checked(rightStart + rightBytes);
            return leftStart < rightEnd && rightStart < leftEnd;
        }

        [DllImport(
            "jadren_native",
            EntryPoint = "jadren_agent_step_batch_soa",
            CallingConvention = CallingConvention.Cdecl,
            ExactSpelling = true)]
        private static extern void StepBatchNative(
            IntPtr positionXPointer, UIntPtr positionXLength,
            IntPtr positionYPointer, UIntPtr positionYLength,
            IntPtr positionZPointer, UIntPtr positionZLength,
            IntPtr velocityXPointer, UIntPtr velocityXLength,
            IntPtr velocityYPointer, UIntPtr velocityYLength,
            IntPtr velocityZPointer, UIntPtr velocityZLength,
            float deltaTime);

        [DllImport(
            "jadren_native",
            EntryPoint = "jadren_agent_step_batch_soa_simd",
            CallingConvention = CallingConvention.Cdecl,
            ExactSpelling = true)]
        private static extern void StepBatchSimdNative(
            IntPtr positionXPointer, UIntPtr positionXLength,
            IntPtr positionYPointer, UIntPtr positionYLength,
            IntPtr positionZPointer, UIntPtr positionZLength,
            IntPtr velocityXPointer, UIntPtr velocityXLength,
            IntPtr velocityYPointer, UIntPtr velocityYLength,
            IntPtr velocityZPointer, UIntPtr velocityZLength,
            UIntPtr count, float deltaTime);

        [DllImport(
            "jadren_native",
            EntryPoint = "jadren_agent_step_batch_soa_simd8",
            CallingConvention = CallingConvention.Cdecl,
            ExactSpelling = true)]
        private static extern void StepBatchSimd8Native(
            IntPtr positionXPointer, UIntPtr positionXLength,
            IntPtr positionYPointer, UIntPtr positionYLength,
            IntPtr positionZPointer, UIntPtr positionZLength,
            IntPtr velocityXPointer, UIntPtr velocityXLength,
            IntPtr velocityYPointer, UIntPtr velocityYLength,
            IntPtr velocityZPointer, UIntPtr velocityZLength,
            UIntPtr count, float deltaTime);
    }
}
