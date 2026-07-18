using System;
using UnityEngine;
using UnityEngine.Rendering;

namespace Jadren.Animation
{
    /// <summary>
    /// Main-thread-owned ComputeShader dispatcher for one quaternion batch.
    /// Unity submits the work here; worker code must only prepare the borrowed
    /// value arrays. Use the returned handle until readback completion.
    /// </summary>
    public sealed class JadrenAnimationGpuPoseDispatcher : IDisposable
    {
        private readonly ComputeShader shader;
        private readonly int kernel;
        private readonly uint threadsX;

        public JadrenAnimationGpuPoseDispatcher(ComputeShader shader)
        {
            if (shader == null)
            {
                throw new ArgumentNullException(nameof(shader));
            }

            this.shader = shader;
            kernel = shader.FindKernel("SlerpUnclamped");
            shader.GetKernelThreadGroupSizes(kernel, out threadsX, out var threadsY, out var threadsZ);
            if (threadsX < 1 || threadsY != 1 || threadsZ != 1)
            {
                throw new ArgumentException(
                    "Animation Slerp kernel must use a positive one-dimensional workgroup.",
                    nameof(shader));
            }
        }

        public bool IsAvailable
        {
            get
            {
                return shader != null
                    && SystemInfo.supportsComputeShaders
                    && SystemInfo.graphicsDeviceType != GraphicsDeviceType.Null;
            }
        }

        /// <summary>
        /// Submits a bone-indexed batch. The source arrays remain borrowed only
        /// for this call; staging buffers are owned by the returned handle.
        /// The handle must be completed or disposed by the main-thread host.
        /// </summary>
        public bool TryDispatch(
            Quaternion[] previous,
            Quaternion[] current,
            float[] weights,
            int boneCount,
            JadrenAnimationLod lod,
            out JadrenAnimationGpuPoseDispatch dispatch,
            out string failureReason)
        {
            dispatch = null;
            failureReason = string.Empty;
            if (!IsAvailable)
            {
                failureReason = "unity_compute_unavailable";
                return false;
            }
            if (previous == null || current == null || weights == null)
            {
                failureReason = "rotation_input_missing";
                return false;
            }
            if (boneCount < 1
                || boneCount > previous.Length
                || boneCount > current.Length
                || boneCount > weights.Length)
            {
                failureReason = "rotation_input_length_invalid";
                return false;
            }
            if (lod == JadrenAnimationLod.Hidden)
            {
                failureReason = "hidden_lod_no_gpu_dispatch";
                return false;
            }

            ComputeBuffer previousBuffer = null;
            ComputeBuffer currentBuffer = null;
            ComputeBuffer weightBuffer = null;
            ComputeBuffer outputBuffer = null;
            try
            {
                // Vector4 staging keeps the shader ABI explicit instead of
                // relying on a Unity Quaternion serialization layout.
                var previousValues = new Vector4[boneCount];
                var currentValues = new Vector4[boneCount];
                for (var boneIndex = 0; boneIndex < boneCount; boneIndex++)
                {
                    previousValues[boneIndex] = ToVector4(previous[boneIndex]);
                    currentValues[boneIndex] = ToVector4(current[boneIndex]);
                }

                previousBuffer = new ComputeBuffer(
                    boneCount,
                    sizeof(float) * 4,
                    ComputeBufferType.Structured);
                currentBuffer = new ComputeBuffer(
                    boneCount,
                    sizeof(float) * 4,
                    ComputeBufferType.Structured);
                weightBuffer = new ComputeBuffer(
                    boneCount,
                    sizeof(float),
                    ComputeBufferType.Structured);
                outputBuffer = new ComputeBuffer(
                    boneCount,
                    sizeof(float) * 4,
                    ComputeBufferType.Structured);
                previousBuffer.SetData(previousValues);
                currentBuffer.SetData(currentValues);
                weightBuffer.SetData(weights, 0, 0, boneCount);

                shader.SetBuffer(kernel, "PreviousRotation", previousBuffer);
                shader.SetBuffer(kernel, "CurrentRotation", currentBuffer);
                shader.SetBuffer(kernel, "FadeWeight", weightBuffer);
                shader.SetBuffer(kernel, "OutputRotation", outputBuffer);
                shader.SetInt("JadrenElementCount", boneCount);
                var groupCount = (boneCount + (int)threadsX - 1) / (int)threadsX;
                shader.Dispatch(kernel, groupCount, 1, 1);

                var readback = AsyncGPUReadback.Request(outputBuffer);
                dispatch = new JadrenAnimationGpuPoseDispatch(
                    new JadrenAnimationGpuPoseResult(boneCount, lod),
                    readback,
                    previousBuffer,
                    currentBuffer,
                    weightBuffer,
                    outputBuffer,
                    boneCount);
                previousBuffer = null;
                currentBuffer = null;
                weightBuffer = null;
                outputBuffer = null;
                return true;
            }
            catch (Exception error)
            {
                failureReason = "unity_compute_exception:" + error.GetType().Name + ":" + error.Message;
                return false;
            }
            finally
            {
                Release(previousBuffer);
                Release(currentBuffer);
                Release(weightBuffer);
                Release(outputBuffer);
            }
        }

        public void Dispose()
        {
            GC.SuppressFinalize(this);
        }

        private static Vector4 ToVector4(Quaternion value)
        {
            return new Vector4(value.x, value.y, value.z, value.w);
        }

        private static void Release(ComputeBuffer buffer)
        {
            if (buffer != null)
            {
                buffer.Release();
            }
        }
    }

    /// <summary>
    /// Frame-spanning GPU completion handle. Dispose waits for completion and
    /// releases all temporary buffers, so no ComputeBuffer can outlive the
    /// readback that consumes it.
    /// </summary>
    public sealed class JadrenAnimationGpuPoseDispatch : IDisposable
    {
        private readonly JadrenAnimationGpuPoseResult result;
        private readonly AsyncGPUReadbackRequest readback;
        private readonly ComputeBuffer previousBuffer;
        private readonly ComputeBuffer currentBuffer;
        private readonly ComputeBuffer weightBuffer;
        private readonly ComputeBuffer outputBuffer;
        private readonly int boneCount;
        private bool completed;

        internal JadrenAnimationGpuPoseDispatch(
            JadrenAnimationGpuPoseResult result,
            AsyncGPUReadbackRequest readback,
            ComputeBuffer previousBuffer,
            ComputeBuffer currentBuffer,
            ComputeBuffer weightBuffer,
            ComputeBuffer outputBuffer,
            int boneCount)
        {
            this.result = result;
            this.readback = readback;
            this.previousBuffer = previousBuffer;
            this.currentBuffer = currentBuffer;
            this.weightBuffer = weightBuffer;
            this.outputBuffer = outputBuffer;
            this.boneCount = boneCount;
        }

        public JadrenAnimationGpuPoseResult Result { get { return result; } }
        public bool IsDone { get { return completed || readback.done; } }
        public bool Succeeded { get { return completed && result.Succeeded; } }

        public bool TryComplete(out JadrenAnimationGpuPoseResult completedResult)
        {
            if (!IsDone)
            {
                completedResult = null;
                return false;
            }

            completedResult = Complete();
            return true;
        }

        public JadrenAnimationGpuPoseResult Complete()
        {
            if (completed)
            {
                return result;
            }

            try
            {
                readback.WaitForCompletion();
                if (readback.hasError)
                {
                    result.TryPublishFailure("unity_compute_readback_error");
                }
                else
                {
                    var data = readback.GetData<Vector4>();
                    var rotations = new Quaternion[boneCount];
                    for (var boneIndex = 0; boneIndex < boneCount; boneIndex++)
                    {
                        var value = data[boneIndex];
                        rotations[boneIndex] = new Quaternion(value.x, value.y, value.z, value.w);
                    }
                    if (!result.TryPublishCompleted(
                            rotations,
                            ExpectedSampleCount(boneCount, result.Lod)))
                    {
                        result.TryPublishFailure("gpu_pose_result_rejected");
                    }
                }
            }
            catch (Exception error)
            {
                result.TryPublishFailure("unity_compute_completion_exception:" + error.GetType().Name);
            }
            finally
            {
                Release(previousBuffer);
                Release(currentBuffer);
                Release(weightBuffer);
                Release(outputBuffer);
                completed = true;
            }
            return result;
        }

        public void Dispose()
        {
            Complete();
            GC.SuppressFinalize(this);
        }

        private static int ExpectedSampleCount(int count, JadrenAnimationLod lod)
        {
            return lod == JadrenAnimationLod.Hidden
                ? 0
                : lod == JadrenAnimationLod.Reduced
                    ? (count + 1) / 2
                    : count;
        }

        private static void Release(ComputeBuffer buffer)
        {
            if (buffer != null)
            {
                buffer.Release();
            }
        }
    }
}
