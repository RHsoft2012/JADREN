using System;
using System.Runtime.InteropServices;

namespace Jadren.Animation
{
    /// <summary>
    /// Versioned blittable record shared by the native Jadren animation batch
    /// ABI. Keep field order and size stable; Unity objects never cross this
    /// boundary.
    /// </summary>
    [StructLayout(LayoutKind.Sequential)]
    public struct JadrenAnimationNativeState
    {
        public uint ClipIndex;
        public uint PreviousClipIndex;
        public float Time;
        public float PreviousTime;
        public float FadeWeight;
        public float Speed;
        public uint Lod;
        public uint Flags;

        public const int ByteSize = 32;
    }

    /// <summary>Blittable packed clip metadata header for the next pose ABI.</summary>
    [StructLayout(LayoutKind.Sequential)]
    public struct JadrenAnimationNativeClipHeader
    {
        public uint FrameOffset;
        public uint FrameCount;
        public uint BoneCount;
        public float SampleRate;
        public float Duration;
        public uint Flags;

        public const int ByteSize = 24;
    }

    /// <summary>Blittable packed local TRS record emitted by the native pose gate.</summary>
    [StructLayout(LayoutKind.Sequential)]
    public struct JadrenAnimationNativePose
    {
        public float PositionX;
        public float PositionY;
        public float PositionZ;
        public float RotationX;
        public float RotationY;
        public float RotationZ;
        public float RotationW;
        public float ScaleX;
        public float ScaleY;
        public float ScaleZ;

        public const int ByteSize = 40;
    }

    /// <summary>
    /// Minimal managed bridge for the native controller-state batch. The
    /// baseline DLL is shipped with the package; AVX2 and NEON artifacts are
    /// selected by the build/capability report and are not silently loaded on
    /// an unknown CPU.
    /// </summary>
    public static class JadrenAnimationNativeBatch
    {
        public const uint AbiVersion = 1;

        public static bool IsAvailable
        {
            get
            {
                try
                {
                    return GetAbiVersionNative() == AbiVersion;
                }
                catch (DllNotFoundException)
                {
                    return false;
                }
                catch (EntryPointNotFoundException)
                {
                    return false;
                }
            }
        }

        public static bool TryGetAbiVersion(out uint version)
        {
            try
            {
                version = GetAbiVersionNative();
                return true;
            }
            catch (DllNotFoundException)
            {
                version = 0;
                return false;
            }
            catch (EntryPointNotFoundException)
            {
                version = 0;
                return false;
            }
        }

        /// <summary>Advances caller-owned AoS state in one synchronous call.</summary>
        public static void Step(JadrenAnimationNativeState[] states, float deltaTime)
        {
            if (states == null)
            {
                throw new ArgumentNullException(nameof(states));
            }
            ValidateDelta(deltaTime);
            if (states.Length == 0)
            {
                return;
            }

            var handle = GCHandle.Alloc(states, GCHandleType.Pinned);
            try
            {
                StepBatchNative(
                    handle.AddrOfPinnedObject(),
                    new UIntPtr((uint)states.Length),
                    deltaTime);
            }
            finally
            {
                handle.Free();
            }
        }

        /// <summary>
        /// Advances the explicit SoA controller layout. All arrays must have
        /// the same logical length; the native function never allocates.
        /// </summary>
        public static void StepSoA(float[] previousTime, float[] time, float[] speed, float deltaTime)
        {
            StepSoAInternal(previousTime, time, speed, deltaTime, simd8: false);
        }

        /// <summary>
        /// Calls the explicit eight-lane SoA export. The package baseline DLL
        /// remains the safe default; a packaging/dispatch layer must replace
        /// it with a validated AVX2 or NEON artifact before using that ISA.
        /// </summary>
        public static void StepSoASimd8(float[] previousTime, float[] time, float[] speed, float deltaTime)
        {
            StepSoAInternal(previousTime, time, speed, deltaTime, simd8: true);
        }

        /// <summary>
        /// Copies one frame-major packed TRS frame into caller-owned pose
        /// records. The frame offset is measured in bones and the managed
        /// caller keeps the exact clip loop/interpolation policy.
        /// </summary>
        public static int CopyFrame(
            float[] translations,
            float[] rotations,
            float[] scales,
            JadrenAnimationNativePose[] output,
            int frameBoneOffset,
            int boneCount,
            JadrenAnimationLod lod)
        {
            if (translations == null) throw new ArgumentNullException(nameof(translations));
            if (rotations == null) throw new ArgumentNullException(nameof(rotations));
            if (scales == null) throw new ArgumentNullException(nameof(scales));
            if (output == null) throw new ArgumentNullException(nameof(output));
            if (frameBoneOffset < 0) throw new ArgumentOutOfRangeException(nameof(frameBoneOffset));
            if (boneCount < 0 || boneCount > output.Length)
            {
                throw new ArgumentOutOfRangeException(nameof(boneCount));
            }
            var requiredBones = checked(frameBoneOffset + boneCount);
            if (translations.Length < checked(requiredBones * 3)
                || rotations.Length < checked(requiredBones * 4)
                || scales.Length < checked(requiredBones * 3))
            {
                throw new ArgumentException("Packed TRS arrays are shorter than the requested frame.");
            }
            if (boneCount == 0 || lod == JadrenAnimationLod.Hidden)
            {
                return 0;
            }

            var translationHandle = GCHandle.Alloc(translations, GCHandleType.Pinned);
            var rotationHandle = GCHandle.Alloc(rotations, GCHandleType.Pinned);
            var scaleHandle = GCHandle.Alloc(scales, GCHandleType.Pinned);
            var outputHandle = GCHandle.Alloc(output, GCHandleType.Pinned);
            try
            {
                var sampled = CopyFrameNative(
                    translationHandle.AddrOfPinnedObject(), new UIntPtr((uint)translations.Length),
                    rotationHandle.AddrOfPinnedObject(), new UIntPtr((uint)rotations.Length),
                    scaleHandle.AddrOfPinnedObject(), new UIntPtr((uint)scales.Length),
                    outputHandle.AddrOfPinnedObject(), new UIntPtr((uint)output.Length),
                    new UIntPtr((uint)frameBoneOffset), new UIntPtr((uint)boneCount), (uint)lod);
                return checked((int)sampled.ToUInt64());
            }
            finally
            {
                outputHandle.Free();
                scaleHandle.Free();
                rotationHandle.Free();
                translationHandle.Free();
            }
        }

        /// <summary>
        /// Blends two packed poses with the explicit linear quaternion
        /// contract. This is not a replacement for Unity Slerp yet.
        /// </summary>
        public static int BlendLinear(
            JadrenAnimationNativePose[] previous,
            JadrenAnimationNativePose[] current,
            JadrenAnimationNativePose[] output,
            int count,
            float fadeWeight,
            JadrenAnimationLod lod)
        {
            if (previous == null) throw new ArgumentNullException(nameof(previous));
            if (current == null) throw new ArgumentNullException(nameof(current));
            if (output == null) throw new ArgumentNullException(nameof(output));
            if (count < 0 || count > previous.Length || count > current.Length || count > output.Length)
            {
                throw new ArgumentOutOfRangeException(nameof(count));
            }
            if (float.IsNaN(fadeWeight) || float.IsInfinity(fadeWeight))
            {
                throw new ArgumentOutOfRangeException(nameof(fadeWeight));
            }
            if (count == 0 || lod == JadrenAnimationLod.Hidden)
            {
                return 0;
            }

            var previousHandle = GCHandle.Alloc(previous, GCHandleType.Pinned);
            var currentHandle = GCHandle.Alloc(current, GCHandleType.Pinned);
            var outputHandle = GCHandle.Alloc(output, GCHandleType.Pinned);
            try
            {
                var sampled = BlendLinearNative(
                    previousHandle.AddrOfPinnedObject(), new UIntPtr((uint)previous.Length),
                    currentHandle.AddrOfPinnedObject(), new UIntPtr((uint)current.Length),
                    outputHandle.AddrOfPinnedObject(), new UIntPtr((uint)output.Length),
                    new UIntPtr((uint)count), fadeWeight, (uint)lod);
                return checked((int)sampled.ToUInt64());
            }
            finally
            {
                outputHandle.Free();
                currentHandle.Free();
                previousHandle.Free();
            }
        }

        /// <summary>
        /// Blends packed poses with the native shortest-arc
        /// <c>Quaternion.SlerpUnclamped</c> contract. The fade value is not
        /// clamped by the native ABI; callers choose whether to extrapolate.
        /// </summary>
        public static int BlendSlerpUnclamped(
            JadrenAnimationNativePose[] previous,
            JadrenAnimationNativePose[] current,
            JadrenAnimationNativePose[] output,
            int count,
            float fadeWeight,
            JadrenAnimationLod lod)
        {
            if (previous == null) throw new ArgumentNullException(nameof(previous));
            if (current == null) throw new ArgumentNullException(nameof(current));
            if (output == null) throw new ArgumentNullException(nameof(output));
            if (count < 0 || count > previous.Length || count > current.Length || count > output.Length)
            {
                throw new ArgumentOutOfRangeException(nameof(count));
            }
            if (float.IsNaN(fadeWeight) || float.IsInfinity(fadeWeight))
            {
                throw new ArgumentOutOfRangeException(nameof(fadeWeight));
            }
            if (count == 0 || lod == JadrenAnimationLod.Hidden)
            {
                return 0;
            }

            var previousHandle = GCHandle.Alloc(previous, GCHandleType.Pinned);
            var currentHandle = GCHandle.Alloc(current, GCHandleType.Pinned);
            var outputHandle = GCHandle.Alloc(output, GCHandleType.Pinned);
            try
            {
                var sampled = BlendSlerpUnclampedNative(
                    previousHandle.AddrOfPinnedObject(), new UIntPtr((uint)previous.Length),
                    currentHandle.AddrOfPinnedObject(), new UIntPtr((uint)current.Length),
                    outputHandle.AddrOfPinnedObject(), new UIntPtr((uint)output.Length),
                    new UIntPtr((uint)count), fadeWeight, (uint)lod);
                return checked((int)sampled.ToUInt64());
            }
            finally
            {
                outputHandle.Free();
                currentHandle.Free();
                previousHandle.Free();
            }
        }

        /// <summary>Computes the same FNV-1a pose checksum as JadrenPoseKernel.</summary>
        public static ulong ComputePoseChecksum(
            JadrenAnimationNativePose[] poses,
            int boneCount,
            JadrenAnimationLod lod)
        {
            if (poses == null || boneCount <= 0)
            {
                return 0UL;
            }

            var count = Math.Min(boneCount, poses.Length);
            var hash = 14695981039346656037UL;
            Mix(ref hash, (uint)count);
            Mix(ref hash, (uint)lod);
            for (var boneIndex = 0; boneIndex < count; boneIndex++)
            {
                if (lod == JadrenAnimationLod.Reduced && (boneIndex & 1) != 0)
                {
                    continue;
                }
                var pose = poses[boneIndex];
                Mix(ref hash, (uint)BitConverter.SingleToInt32Bits(pose.PositionX));
                Mix(ref hash, (uint)BitConverter.SingleToInt32Bits(pose.PositionY));
                Mix(ref hash, (uint)BitConverter.SingleToInt32Bits(pose.PositionZ));
                Mix(ref hash, (uint)BitConverter.SingleToInt32Bits(pose.RotationX));
                Mix(ref hash, (uint)BitConverter.SingleToInt32Bits(pose.RotationY));
                Mix(ref hash, (uint)BitConverter.SingleToInt32Bits(pose.RotationZ));
                Mix(ref hash, (uint)BitConverter.SingleToInt32Bits(pose.RotationW));
                Mix(ref hash, (uint)BitConverter.SingleToInt32Bits(pose.ScaleX));
                Mix(ref hash, (uint)BitConverter.SingleToInt32Bits(pose.ScaleY));
                Mix(ref hash, (uint)BitConverter.SingleToInt32Bits(pose.ScaleZ));
            }
            return hash;
        }

        private static void StepSoAInternal(
            float[] previousTime,
            float[] time,
            float[] speed,
            float deltaTime,
            bool simd8)
        {
            if (previousTime == null) throw new ArgumentNullException(nameof(previousTime));
            if (time == null) throw new ArgumentNullException(nameof(time));
            if (speed == null) throw new ArgumentNullException(nameof(speed));
            if (previousTime.Length != time.Length || time.Length != speed.Length)
            {
                throw new ArgumentException("SoA arrays must have identical lengths.");
            }
            ValidateDelta(deltaTime);
            if (time.Length == 0)
            {
                return;
            }

            var previousHandle = GCHandle.Alloc(previousTime, GCHandleType.Pinned);
            var timeHandle = GCHandle.Alloc(time, GCHandleType.Pinned);
            var speedHandle = GCHandle.Alloc(speed, GCHandleType.Pinned);
            try
            {
                if (simd8)
                {
                    StepSoASimd8Native(
                        previousHandle.AddrOfPinnedObject(), new UIntPtr((uint)previousTime.Length),
                        timeHandle.AddrOfPinnedObject(), new UIntPtr((uint)time.Length),
                        speedHandle.AddrOfPinnedObject(), new UIntPtr((uint)speed.Length),
                        new UIntPtr((uint)time.Length), deltaTime);
                }
                else
                {
                    StepSoANative(
                        previousHandle.AddrOfPinnedObject(), new UIntPtr((uint)previousTime.Length),
                        timeHandle.AddrOfPinnedObject(), new UIntPtr((uint)time.Length),
                        speedHandle.AddrOfPinnedObject(), new UIntPtr((uint)speed.Length),
                        new UIntPtr((uint)time.Length), deltaTime);
                }
            }
            finally
            {
                speedHandle.Free();
                timeHandle.Free();
                previousHandle.Free();
            }
        }

        private static void ValidateDelta(float deltaTime)
        {
            if (float.IsNaN(deltaTime) || float.IsInfinity(deltaTime) || deltaTime < 0.0f)
            {
                throw new ArgumentOutOfRangeException(nameof(deltaTime));
            }
        }

        private static void Mix(ref ulong hash, uint value)
        {
            hash ^= value;
            hash *= 1099511628211UL;
        }

        [DllImport(
            "jadren_animation_native",
            EntryPoint = "jadren_animation_batch_abi_version",
            CallingConvention = CallingConvention.Cdecl,
            ExactSpelling = true)]
        private static extern uint GetAbiVersionNative();

        [DllImport(
            "jadren_animation_native",
            EntryPoint = "jadren_animation_state_batch",
            CallingConvention = CallingConvention.Cdecl,
            ExactSpelling = true)]
        private static extern void StepBatchNative(
            IntPtr statesPointer,
            UIntPtr statesLength,
            float deltaTime);

        [DllImport(
            "jadren_animation_native",
            EntryPoint = "jadren_animation_state_batch_soa",
            CallingConvention = CallingConvention.Cdecl,
            ExactSpelling = true)]
        private static extern void StepSoANative(
            IntPtr previousTimePointer,
            UIntPtr previousTimeLength,
            IntPtr timePointer,
            UIntPtr timeLength,
            IntPtr speedPointer,
            UIntPtr speedLength,
            UIntPtr count,
            float deltaTime);

        [DllImport(
            "jadren_animation_native",
            EntryPoint = "jadren_animation_state_batch_soa_simd8",
            CallingConvention = CallingConvention.Cdecl,
            ExactSpelling = true)]
        private static extern void StepSoASimd8Native(
            IntPtr previousTimePointer,
            UIntPtr previousTimeLength,
            IntPtr timePointer,
            UIntPtr timeLength,
            IntPtr speedPointer,
            UIntPtr speedLength,
            UIntPtr count,
            float deltaTime);

        [DllImport(
            "jadren_animation_native",
            EntryPoint = "jadren_animation_pose_copy_frame",
            CallingConvention = CallingConvention.Cdecl,
            ExactSpelling = true)]
        private static extern UIntPtr CopyFrameNative(
            IntPtr translationsPointer,
            UIntPtr translationsLength,
            IntPtr rotationsPointer,
            UIntPtr rotationsLength,
            IntPtr scalesPointer,
            UIntPtr scalesLength,
            IntPtr outputPointer,
            UIntPtr outputLength,
            UIntPtr frameBoneOffset,
            UIntPtr boneCount,
            uint lod);

        [DllImport(
            "jadren_animation_native",
            EntryPoint = "jadren_animation_pose_blend_linear",
            CallingConvention = CallingConvention.Cdecl,
            ExactSpelling = true)]
        private static extern UIntPtr BlendLinearNative(
            IntPtr previousPointer,
            UIntPtr previousLength,
            IntPtr currentPointer,
            UIntPtr currentLength,
            IntPtr outputPointer,
            UIntPtr outputLength,
            UIntPtr count,
            float fadeWeight,
            uint lod);

        [DllImport(
            "jadren_animation_native",
            EntryPoint = "jadren_animation_pose_blend_slerp_unclamped",
            CallingConvention = CallingConvention.Cdecl,
            ExactSpelling = true)]
        private static extern UIntPtr BlendSlerpUnclampedNative(
            IntPtr previousPointer,
            UIntPtr previousLength,
            IntPtr currentPointer,
            UIntPtr currentLength,
            IntPtr outputPointer,
            UIntPtr outputLength,
            UIntPtr count,
            float fadeWeight,
            uint lod);
    }
}
