using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using UnityEngine;
using UnityEngine.Rendering;

namespace Jadren.Animation
{
    /// <summary>
    /// GPU pose producer for the Phase-1 crowd path. It evaluates the baked
    /// clip samples and parent hierarchy once per agent in a compute kernel,
    /// writing the same [agent][mesh-bone] matrix ABI consumed by the GPU
    /// skinning renderer. The CPU evaluator remains the correctness fallback.
    /// </summary>
    internal sealed class JadrenAnimationGpuCrowdPoseStream : IDisposable
    {
        [StructLayout(LayoutKind.Sequential)]
        private struct ClipInfo
        {
            public int frameOffset;
            public int frameCount;
            public int boneCount;
            public float sampleRate;
            public float duration;
            public int loop;
            public float speedThreshold;
            public float playbackSpeed;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct MeshPathInfo
        {
            public int offset;
            public int count;
        }

        private readonly ComputeShader shader;
        private readonly int kernel;
        private readonly int agentCount;
        private readonly int rigBoneCount;
        private readonly int meshBoneCount;
        private readonly ComputeBuffer clipInfoBuffer;
        private readonly ComputeBuffer translationBuffer;
        private readonly ComputeBuffer rotationBuffer;
        private readonly ComputeBuffer scaleBuffer;
        private readonly ComputeBuffer meshPathInfoBuffer;
        private readonly ComputeBuffer meshPathIndexBuffer;
        private readonly ComputeBuffer bindposeBuffer;
        private readonly ComputeBuffer agentTimeBuffer;
        private readonly ComputeBuffer agentSpeedBuffer;
        private readonly ComputeBuffer outputBuffer;
        private bool disposed;

        public bool IsReady { get { return !disposed && outputBuffer != null; } }
        public ComputeBuffer BoneMatrixBuffer { get { return outputBuffer; } }

        private JadrenAnimationGpuCrowdPoseStream(
            ComputeShader shader,
            int kernel,
            int agentCount,
            int rigBoneCount,
            int meshBoneCount,
            ComputeBuffer clipInfoBuffer,
            ComputeBuffer translationBuffer,
            ComputeBuffer rotationBuffer,
            ComputeBuffer scaleBuffer,
            ComputeBuffer meshPathInfoBuffer,
            ComputeBuffer meshPathIndexBuffer,
            ComputeBuffer bindposeBuffer,
            ComputeBuffer agentTimeBuffer,
            ComputeBuffer agentSpeedBuffer,
            ComputeBuffer outputBuffer)
        {
            this.shader = shader;
            this.kernel = kernel;
            this.agentCount = agentCount;
            this.rigBoneCount = rigBoneCount;
            this.meshBoneCount = meshBoneCount;
            this.clipInfoBuffer = clipInfoBuffer;
            this.translationBuffer = translationBuffer;
            this.rotationBuffer = rotationBuffer;
            this.scaleBuffer = scaleBuffer;
            this.meshPathInfoBuffer = meshPathInfoBuffer;
            this.meshPathIndexBuffer = meshPathIndexBuffer;
            this.bindposeBuffer = bindposeBuffer;
            this.agentTimeBuffer = agentTimeBuffer;
            this.agentSpeedBuffer = agentSpeedBuffer;
            this.outputBuffer = outputBuffer;
        }

        public static bool TryCreate(
            JadrenRigAsset rig,
            JadrenControllerAsset controller,
            int[] meshBoneToRig,
            Matrix4x4[] bindposes,
            int rootBoneIndex,
            Vector3 rootReferencePosition,
            float[] agentSpeeds,
            int agentCount,
            ComputeShader shader,
            out JadrenAnimationGpuCrowdPoseStream stream,
            out string failureReason)
        {
            stream = null;
            failureReason = string.Empty;
            if (rig == null || controller == null)
            {
                failureReason = "gpu_pose_assets_missing";
                return false;
            }
            if (meshBoneToRig == null || meshBoneToRig.Length < 1
                || bindposes == null || bindposes.Length != meshBoneToRig.Length)
            {
                failureReason = "gpu_pose_mesh_layout_invalid";
                return false;
            }
            if (agentSpeeds == null || agentSpeeds.Length != agentCount || agentCount < 1)
            {
                failureReason = "gpu_pose_agent_state_invalid";
                return false;
            }
            if (shader == null || !SystemInfo.supportsComputeShaders)
            {
                failureReason = "gpu_pose_compute_unavailable";
                return false;
            }
            if (rig.BoneCount < 1 || rig.BoneCount > 256)
            {
                failureReason = "gpu_pose_rig_bone_count_unsupported";
                return false;
            }

            var clipInfos = new List<ClipInfo>();
            var translations = new List<Vector3>();
            var rotations = new List<Vector4>();
            var scales = new List<Vector3>();
            ComputeBuffer clipInfoBuffer = null;
            ComputeBuffer translationBuffer = null;
            ComputeBuffer rotationBuffer = null;
            ComputeBuffer scaleBuffer = null;
            ComputeBuffer meshPathInfoBuffer = null;
            ComputeBuffer meshPathIndexBuffer = null;
            ComputeBuffer bindposeBuffer = null;
            ComputeBuffer agentTimeBuffer = null;
            ComputeBuffer agentSpeedBuffer = null;
            ComputeBuffer outputBuffer = null;
            try
            {
                for (var stateIndex = 0; stateIndex < controller.StateCount; stateIndex++)
                {
                    var state = controller.GetState(stateIndex);
                    var clip = state.clip;
                    if (clip == null || clip.RigBoneCount != rig.BoneCount
                        || clip.FrameCount < 1 || clip.Duration <= 0.0f)
                    {
                        failureReason = "gpu_pose_clip_layout_invalid:" + stateIndex;
                        return false;
                    }

                    clip.CopyBakedData(
                        out var clipTranslations,
                        out var clipRotations,
                        out var clipScales);
                    var expected = checked(clip.FrameCount * rig.BoneCount);
                    if (clipTranslations.Length < expected
                        || clipRotations.Length < expected
                        || clipScales.Length < expected)
                    {
                        failureReason = "gpu_pose_clip_data_short:" + stateIndex;
                        return false;
                    }

                    var frameOffset = translations.Count;
                    for (var sample = 0; sample < expected; sample++)
                    {
                        translations.Add(clipTranslations[sample]);
                        var rotation = clipRotations[sample];
                        rotations.Add(new Vector4(
                            rotation.x,
                            rotation.y,
                            rotation.z,
                            rotation.w));
                        scales.Add(clipScales[sample]);
                    }
                    clipInfos.Add(new ClipInfo
                    {
                        frameOffset = frameOffset,
                        frameCount = clip.FrameCount,
                        boneCount = clip.RigBoneCount,
                        sampleRate = clip.SampleRate,
                        duration = clip.Duration,
                        loop = clip.Loop ? 1 : 0,
                        speedThreshold = state.speedThreshold,
                        playbackSpeed = Mathf.Approximately(state.playbackSpeed, 0.0f)
                            ? 1.0f
                            : state.playbackSpeed
                    });
                }
                if (clipInfos.Count < 1)
                {
                    failureReason = "gpu_pose_clips_missing";
                    return false;
                }

                var meshPaths = new List<MeshPathInfo>(meshBoneToRig.Length);
                var meshPathIndices = new List<int>(meshBoneToRig.Length * 4);
                for (var meshBone = 0; meshBone < meshBoneToRig.Length; meshBone++)
                {
                    var chain = new List<int>();
                    var rigBone = meshBoneToRig[meshBone];
                    var guard = 0;
                    while (rigBone >= 0 && guard++ <= rig.BoneCount)
                    {
                        chain.Add(rigBone);
                        rigBone = rig.GetParentIndex(rigBone);
                    }
                    if (guard > rig.BoneCount + 1)
                    {
                        failureReason = "gpu_pose_rig_parent_cycle";
                        return false;
                    }
                    chain.Reverse();
                    var offset = meshPathIndices.Count;
                    meshPathIndices.AddRange(chain);
                    meshPaths.Add(new MeshPathInfo
                    {
                        offset = offset,
                        count = chain.Count
                    });
                }

                var shaderKernel = shader.FindKernel("EvaluateAnimationCrowd");
                clipInfoBuffer = new ComputeBuffer(
                    clipInfos.Count,
                    Marshal.SizeOf(typeof(ClipInfo)),
                    ComputeBufferType.Structured);
                translationBuffer = new ComputeBuffer(
                    translations.Count,
                    sizeof(float) * 3,
                    ComputeBufferType.Structured);
                rotationBuffer = new ComputeBuffer(
                    rotations.Count,
                    sizeof(float) * 4,
                    ComputeBufferType.Structured);
                scaleBuffer = new ComputeBuffer(
                    scales.Count,
                    sizeof(float) * 3,
                    ComputeBufferType.Structured);
                meshPathInfoBuffer = new ComputeBuffer(
                    meshPaths.Count,
                    Marshal.SizeOf(typeof(MeshPathInfo)),
                    ComputeBufferType.Structured);
                meshPathIndexBuffer = new ComputeBuffer(
                    Mathf.Max(1, meshPathIndices.Count),
                    sizeof(int),
                    ComputeBufferType.Structured);
                bindposeBuffer = new ComputeBuffer(
                    bindposes.Length,
                    sizeof(float) * 16,
                    ComputeBufferType.Structured);
                agentTimeBuffer = new ComputeBuffer(
                    agentCount,
                    sizeof(float),
                    ComputeBufferType.Structured);
                agentSpeedBuffer = new ComputeBuffer(
                    agentCount,
                    sizeof(float),
                    ComputeBufferType.Structured);
                outputBuffer = new ComputeBuffer(
                    checked(agentCount * meshBoneToRig.Length),
                    sizeof(float) * 16,
                    ComputeBufferType.Structured);

                clipInfoBuffer.SetData(clipInfos);
                translationBuffer.SetData(translations);
                rotationBuffer.SetData(rotations);
                scaleBuffer.SetData(scales);
                meshPathInfoBuffer.SetData(meshPaths.ToArray());
                meshPathIndexBuffer.SetData(
                    meshPathIndices.Count == 0 ? new[] { 0 } : meshPathIndices.ToArray());
                bindposeBuffer.SetData(bindposes);
                agentTimeBuffer.SetData(new float[agentCount]);
                agentSpeedBuffer.SetData(agentSpeeds);

                stream = new JadrenAnimationGpuCrowdPoseStream(
                    shader,
                    shaderKernel,
                    agentCount,
                    rig.BoneCount,
                    meshBoneToRig.Length,
                    clipInfoBuffer,
                    translationBuffer,
                    rotationBuffer,
                    scaleBuffer,
                    meshPathInfoBuffer,
                    meshPathIndexBuffer,
                    bindposeBuffer,
                    agentTimeBuffer,
                    agentSpeedBuffer,
                    outputBuffer);
                stream.BindStaticInputs(rootBoneIndex, rootReferencePosition);
                return true;
            }
            catch (Exception error)
            {
                outputBuffer?.Release();
                agentSpeedBuffer?.Release();
                agentTimeBuffer?.Release();
                bindposeBuffer?.Release();
                meshPathIndexBuffer?.Release();
                meshPathInfoBuffer?.Release();
                scaleBuffer?.Release();
                rotationBuffer?.Release();
                translationBuffer?.Release();
                clipInfoBuffer?.Release();
                stream = null;
                failureReason = "gpu_pose_init_exception:" + error.GetType().Name + ":" + error.Message;
                return false;
            }
        }

        public bool Dispatch(
            float deltaTime,
            int agentColumns,
            float agentSpacing,
            Vector3 agentOrigin,
            int rootBoneIndex,
            Vector3 rootReferencePosition,
            out string failureReason)
        {
            failureReason = string.Empty;
            if (!IsReady)
            {
                failureReason = "gpu_pose_stream_unavailable";
                return false;
            }
            try
            {
                BindDispatchInputs(
                    deltaTime,
                    agentColumns,
                    agentSpacing,
                    agentOrigin,
                    rootBoneIndex,
                    rootReferencePosition);
                // One thread group owns one agent; the 64 group threads walk
                // independent mesh-bone paths without a 256-matrix local
                // array or a per-frame CPU matrix upload.
                shader.Dispatch(kernel, agentCount, 1, 1);
                // Ensure the pose UAV is visible to the following skinning
                // dispatch on graphics backends that do not infer the UAV ->
                // SRV transition from SetBuffer alone.
                var fence = Graphics.CreateGraphicsFence(
                    GraphicsFenceType.AsyncQueueSynchronisation,
                    SynchronisationStageFlags.ComputeProcessing);
                Graphics.WaitOnAsyncGraphicsFence(fence);
                return true;
            }
            catch (Exception error)
            {
                failureReason = "gpu_pose_dispatch_exception:" + error.GetType().Name + ":" + error.Message;
                return false;
            }
        }

        /// <summary>
        /// Diagnostic-only copy of the pose matrix stream. It waits for the
        /// GPU and is intentionally excluded from the animation hot path.
        /// </summary>
        public bool TryReadbackBoneMatrices(
            out Matrix4x4[] matrices,
            out string failureReason)
        {
            matrices = Array.Empty<Matrix4x4>();
            failureReason = string.Empty;
            if (!IsReady)
            {
                failureReason = "gpu_pose_stream_unavailable";
                return false;
            }
            var request = AsyncGPUReadback.Request(outputBuffer);
            request.WaitForCompletion();
            if (request.hasError)
            {
                failureReason = "gpu_pose_matrix_readback_error";
                return false;
            }
            var data = request.GetData<Matrix4x4>();
            matrices = new Matrix4x4[data.Length];
            data.CopyTo(matrices);
            return matrices.Length > 0;
        }

        private void BindStaticInputs(int rootBoneIndex, Vector3 rootReferencePosition)
        {
            shader.SetInt("JadrenAnimationRootBoneIndex", rootBoneIndex);
            shader.SetVector("JadrenAnimationRootReference", rootReferencePosition);
        }

        private void BindDispatchInputs(
            float deltaTime,
            int agentColumns,
            float agentSpacing,
            Vector3 agentOrigin,
            int rootBoneIndex,
            Vector3 rootReferencePosition)
        {
            shader.SetInt("JadrenAnimationAgentCount", agentCount);
            shader.SetInt("JadrenAnimationAgentColumns", Mathf.Max(1, agentColumns));
            shader.SetInt("JadrenAnimationRigBoneCount", rigBoneCount);
            shader.SetInt("JadrenAnimationMeshBoneCount", meshBoneCount);
            shader.SetInt("JadrenAnimationClipCount", clipInfoBuffer.count);
            shader.SetInt("JadrenAnimationRootBoneIndex", rootBoneIndex);
            shader.SetFloat("JadrenAnimationDeltaTime", Mathf.Max(0.0f, deltaTime));
            shader.SetFloat("JadrenAnimationAgentSpacing", Mathf.Max(0.1f, agentSpacing));
            shader.SetVector("JadrenAnimationAgentOrigin", agentOrigin);
            shader.SetVector("JadrenAnimationRootReference", rootReferencePosition);
            shader.SetBuffer(kernel, "AnimationClipInfos", clipInfoBuffer);
            shader.SetBuffer(kernel, "AnimationTranslations", translationBuffer);
            shader.SetBuffer(kernel, "AnimationRotations", rotationBuffer);
            shader.SetBuffer(kernel, "AnimationScales", scaleBuffer);
            shader.SetBuffer(kernel, "AnimationMeshPathInfos", meshPathInfoBuffer);
            shader.SetBuffer(kernel, "AnimationMeshPathIndices", meshPathIndexBuffer);
            shader.SetBuffer(kernel, "AnimationBindposes", bindposeBuffer);
            shader.SetBuffer(kernel, "AnimationAgentTimes", agentTimeBuffer);
            shader.SetBuffer(kernel, "AnimationAgentSpeeds", agentSpeedBuffer);
            shader.SetBuffer(kernel, "AnimationBoneMatrices", outputBuffer);
        }

        public void Dispose()
        {
            if (disposed)
            {
                return;
            }
            disposed = true;
            Release(clipInfoBuffer);
            Release(translationBuffer);
            Release(rotationBuffer);
            Release(scaleBuffer);
            Release(meshPathInfoBuffer);
            Release(meshPathIndexBuffer);
            Release(bindposeBuffer);
            Release(agentTimeBuffer);
            Release(agentSpeedBuffer);
            Release(outputBuffer);
            GC.SuppressFinalize(this);
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
