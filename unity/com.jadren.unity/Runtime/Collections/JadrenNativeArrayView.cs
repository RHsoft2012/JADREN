using System;
using System.Threading;
using Unity.Collections;
using Unity.Collections.LowLevel.Unsafe;

namespace Jadren.Unity
{
    /// <summary>
    /// Borrowed, zero-copy view over a caller-owned NativeArray.
    /// The view never disposes or resizes the underlying array.
    /// </summary>
    public sealed class JadrenNativeArrayView<T> : IDisposable where T : unmanaged
    {
        private readonly NativeArray<T> array;
        private int disposed;
        private int activeLeases;

        public JadrenNativeArrayView(NativeArray<T> array)
        {
            if (!array.IsCreated)
            {
                throw new ArgumentException("NativeArray must be created", nameof(array));
            }
            this.array = array;
        }

        public int Length
        {
            get
            {
                ThrowIfUnavailable();
                return array.Length;
            }
        }

        public int ElementSize => UnsafeUtility.SizeOf<T>();

        /// <summary>
        /// Returns the caller-owned NativeArray only to another bridge
        /// component while an explicit lease is active.
        /// </summary>
        internal NativeArray<T> BorrowedArray
        {
            get
            {
                ThrowIfUnavailable();
                return array;
            }
        }

        /// <summary>Acquires a lease; the pointer is valid only until lease disposal.</summary>
        public JadrenNativeArrayLease<T> Acquire(bool writable = true)
        {
            ThrowIfUnavailable();
            Interlocked.Increment(ref activeLeases);
            if (Volatile.Read(ref disposed) != 0 || !array.IsCreated)
            {
                Interlocked.Decrement(ref activeLeases);
                throw new ObjectDisposedException(nameof(JadrenNativeArrayView<T>));
            }
            return new JadrenNativeArrayLease<T>(this, writable);
        }

        /// <summary>
        /// Acquires a heap-backed lease for an asynchronous GPU operation. The
        /// caller must dispose the lease only after the GPU completion handle.
        /// </summary>
        public JadrenNativeArrayAsyncLease<T> AcquireAsync(bool writable = true)
        {
            ThrowIfUnavailable();
            Interlocked.Increment(ref activeLeases);
            if (Volatile.Read(ref disposed) != 0 || !array.IsCreated)
            {
                Interlocked.Decrement(ref activeLeases);
                throw new ObjectDisposedException(nameof(JadrenNativeArrayView<T>));
            }
            return new JadrenNativeArrayAsyncLease<T>(this, writable);
        }

        internal unsafe IntPtr GetPointer(bool writable)
        {
            ThrowIfUnavailable();
            return writable
                ? (IntPtr)NativeArrayUnsafeUtility.GetUnsafePtr(array)
                : (IntPtr)NativeArrayUnsafeUtility.GetUnsafeReadOnlyPtr(array);
        }

        internal void ReleaseLease()
        {
            Interlocked.Decrement(ref activeLeases);
        }

        public void Dispose()
        {
            if (Volatile.Read(ref activeLeases) != 0)
            {
                throw new InvalidOperationException("NativeArray view has an active lease");
            }
            Interlocked.Exchange(ref disposed, 1);
            GC.SuppressFinalize(this);
        }

        private void ThrowIfUnavailable()
        {
            if (Volatile.Read(ref disposed) != 0 || !array.IsCreated)
            {
                throw new ObjectDisposedException(nameof(JadrenNativeArrayView<T>));
            }
        }
    }

    /// <summary>Stack-only lifetime token for one native call.</summary>
    public ref struct JadrenNativeArrayLease<T> where T : unmanaged
    {
        private readonly JadrenNativeArrayView<T> owner;
        private readonly bool writable;
        private bool released;

        internal JadrenNativeArrayLease(JadrenNativeArrayView<T> owner, bool writable)
        {
            this.owner = owner;
            this.writable = writable;
            released = false;
        }

        public IntPtr Pointer
        {
            get
            {
                ThrowIfReleased();
                return owner.GetPointer(writable);
            }
        }

        public int Length
        {
            get
            {
                ThrowIfReleased();
                return owner.Length;
            }
        }

        public int ElementSize
        {
            get
            {
                ThrowIfReleased();
                return owner.ElementSize;
            }
        }

        public void Dispose()
        {
            if (released)
            {
                return;
            }
            released = true;
            owner.ReleaseLease();
        }

        private void ThrowIfReleased()
        {
            if (released)
            {
                throw new ObjectDisposedException(nameof(JadrenNativeArrayLease<T>));
            }
        }
    }

    /// <summary>
    /// Heap-backed borrowed lease that can span Unity frames. It is valid only
    /// until Dispose and must be owned by the async completion handle.
    /// </summary>
    public sealed class JadrenNativeArrayAsyncLease<T> : IDisposable where T : unmanaged
    {
        private readonly JadrenNativeArrayView<T> owner;
        private readonly bool writable;
        private int released;

        internal JadrenNativeArrayAsyncLease(JadrenNativeArrayView<T> owner, bool writable)
        {
            this.owner = owner;
            this.writable = writable;
        }

        public IntPtr Pointer
        {
            get
            {
                ThrowIfReleased();
                return owner.GetPointer(writable);
            }
        }

        public NativeArray<T> BorrowedArray
        {
            get
            {
                ThrowIfReleased();
                return owner.BorrowedArray;
            }
        }

        public int Length
        {
            get
            {
                ThrowIfReleased();
                return owner.Length;
            }
        }

        public int ElementSize
        {
            get
            {
                ThrowIfReleased();
                return owner.ElementSize;
            }
        }

        public void Dispose()
        {
            if (Interlocked.Exchange(ref released, 1) == 0)
            {
                owner.ReleaseLease();
            }
            GC.SuppressFinalize(this);
        }

        private void ThrowIfReleased()
        {
            if (Volatile.Read(ref released) != 0)
            {
                throw new ObjectDisposedException(nameof(JadrenNativeArrayAsyncLease<T>));
            }
        }
    }
}
