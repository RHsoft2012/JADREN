using System;
using System.Runtime.InteropServices;

namespace Jadren.Unity
{
    /// <summary>Stable runtime ABI version advertised by Jadren 0.1.</summary>
    public readonly struct JadrenAbiVersion : IEquatable<JadrenAbiVersion>
    {
        public JadrenAbiVersion(uint major, uint minor)
        {
            Major = major;
            Minor = minor;
        }

        public uint Major { get; }
        public uint Minor { get; }

        public bool IsCompatibleWith(JadrenAbiVersion required)
        {
            return Major == required.Major && Minor >= required.Minor;
        }

        public bool Equals(JadrenAbiVersion other) => Major == other.Major && Minor == other.Minor;
        public override bool Equals(object obj) => obj is JadrenAbiVersion other && Equals(other);
        public override int GetHashCode() => HashCode.Combine(Major, Minor);
        public override string ToString() => $"{Major}.{Minor}";
    }

    /// <summary>Pure managed ABI identity helpers. Native loading is JAD-1004.</summary>
    public static class JadrenRuntime
    {
        public const int NativeUnavailable = -100;
        public const int LayoutMismatch = -101;
        public static readonly JadrenAbiVersion CurrentAbi = new JadrenAbiVersion(0u, 10u);

        public readonly struct Identity
        {
            public Identity(JadrenAbiVersion abi, ulong buildId, int pointerBits)
            {
                Abi = abi;
                BuildId = buildId;
                PointerBits = pointerBits;
            }

            public JadrenAbiVersion Abi { get; }
            public ulong BuildId { get; }
            public int PointerBits { get; }
        }

        public static bool IsCompatible(ulong packedVersion)
        {
            var major = (uint)(packedVersion >> 32);
            var minor = (uint)(packedVersion & 0xffffffffu);
            return CurrentAbi.IsCompatibleWith(new JadrenAbiVersion(major, minor));
        }

        /// <summary>
        /// Lazily loads the platform plugin and performs the fixed-width ABI handshake.
        /// Missing editor/player plugins are reported as a status instead of throwing.
        /// </summary>
        public static bool TryInitialize(out JadrenAbiVersion nativeAbi, out int status)
        {
            Identity identity;
            var ok = TryInitialize(out identity, out status);
            nativeAbi = identity.Abi;
            return ok;
        }

        /// <summary>Initializes the native runtime and returns its ABI/build identity.</summary>
        public static bool TryInitialize(out Identity identity, out int status)
        {
            identity = default(Identity);
            status = NativeUnavailable;
            try
            {
                var packed = Native.jadren_rt_abi_version();
                var nativeAbi = new JadrenAbiVersion((uint)(packed >> 32), (uint)packed);
                identity = new Identity(nativeAbi, Native.jadren_rt_build_id(), IntPtr.Size * 8);
                if (!nativeAbi.IsCompatibleWith(CurrentAbi))
                {
                    status = -2;
                    return false;
                }

                status = Native.jadren_rt_initialize(CurrentAbi.Major, CurrentAbi.Minor);
                return status == 0 || status == 1;
            }
            catch (DllNotFoundException)
            {
                return false;
            }
            catch (EntryPointNotFoundException)
            {
                return false;
            }
            catch (BadImageFormatException)
            {
                return false;
            }
        }

        /// <summary>Initializes only when the host pointer width matches generated layout.</summary>
        public static bool TryInitialize(
            ushort expectedPointerBits,
            out Identity identity,
            out int status)
        {
            identity = default(Identity);
            if (expectedPointerBits != IntPtr.Size * 8)
            {
                status = LayoutMismatch;
                return false;
            }
            return TryInitialize(out identity, out status);
        }

        private static class Native
        {
            [DllImport("jadren_runtime", CallingConvention = CallingConvention.Cdecl,
                EntryPoint = "jadren_rt_abi_version")]
            internal static extern ulong jadren_rt_abi_version();

            [DllImport("jadren_runtime", CallingConvention = CallingConvention.Cdecl,
                EntryPoint = "jadren_rt_build_id")]
            internal static extern ulong jadren_rt_build_id();

            [DllImport("jadren_runtime", CallingConvention = CallingConvention.Cdecl,
                EntryPoint = "jadren_rt_initialize")]
            internal static extern int jadren_rt_initialize(uint requiredMajor, uint requiredMinor);
        }
    }
}
