using System;
using System.Collections.Generic;
using UnityEngine;

namespace Jadren.Animation
{
    /// <summary>
    /// Opt-in bridge from one baked Jadren rig/controller to a procedural GPU
    /// crowd. The source prefab is inspected once on the main thread; runtime
    /// frames update only caller-owned pose/matrix arrays and procedural draws.
    /// One material keeps the one-draw fast path; a mesh with one material per
    /// submesh shares the same pose buffer and submits one draw per submesh.
    /// </summary>
    [DisallowMultipleComponent]
    [DefaultExecutionOrder(-1000)]
    public sealed class JadrenAnimationGpuCrowdAnimator : MonoBehaviour, IDisposable
    {
        [Header("Source and GPU route")]
        public GameObject CharacterPrefab;
        public ComputeShader SkinningShader;
        public Shader CrowdShader;

        [Header("Crowd layout")]
        [Min(1)] public int AgentCount = 250;
        [Min(1)] public int AgentColumns = 25;
        [Min(0.1f)] public float AgentSpacing = 2.0f;
        public Vector3 AgentOrigin = Vector3.zero;
        public bool AutoBuild = true;
        public bool Animate = true;
        public bool PreferNativeSlerp = true;
        [Tooltip("Experimental: use weighted AoSoA8 crowd blending. Disabled by default until packed native sampling removes managed pose conversion overhead.")]
        public bool PreferNativePoseTiles = false;
        public bool PreferGpuAnimation = true;
        [Min(0)] public int SourceLodLevel = 0;
        [Tooltip("Optional parity/benchmark override. Negative keeps the normal per-agent speed pattern.")]
        public float UniformSpeed = -1.0f;
        [Min(0.0001f)] public float DeltaTime = 1.0f / 60.0f;

        private JadrenGpuSkinningCrowdRenderer rendererHost;
        private JadrenAnimationBatchPoseEvaluator evaluator;
        private JadrenAnimationGpuCrowdPoseStream gpuPoseStream;
        private JadrenRigAsset rig;
        private JadrenControllerAsset controller;
        private Mesh sourceMesh;
        private Material sourceMaterial;
        private Material[] sourceMaterials = Array.Empty<Material>();
        private Matrix4x4[] sourceBindposes = Array.Empty<Matrix4x4>();
        private JadrenGpuSkinningVertex[] crowdVertices = Array.Empty<JadrenGpuSkinningVertex>();
        private Matrix4x4[] crowdBoneMatrices = Array.Empty<Matrix4x4>();
        private Matrix4x4[] rigBoneMatrices = Array.Empty<Matrix4x4>();
        private int[] meshBoneToRig = Array.Empty<int>();
        private float[] agentSpeeds = Array.Empty<float>();
        private float[] stoppedAgentSpeeds = Array.Empty<float>();
        private int rootBoneIndex = -1;
        private Vector3 rootReferencePosition;
        private bool rootReferenceCaptured;
        private bool built;
        private bool disposed;
        private string status = "Not built";

        public bool IsBuilt { get { return built && !disposed; } }
        public string Status { get { return status; } }
        public string FailureReason { get; private set; } = string.Empty;
        public int BuiltAgentCount { get { return built ? AgentCount : 0; } }
        public int MeshBoneCount { get { return meshBoneToRig.Length; } }
        public int VerticesPerAgent { get { return sourceMesh == null ? 0 : sourceMesh.vertexCount; } }
        public JadrenGpuSkinningCrowdRenderer RendererHost { get { return rendererHost; } }
        public bool UsesGpuAnimation { get { return gpuPoseStream != null && gpuPoseStream.IsReady; } }
        public int LastPoseUpdateCount { get; private set; }
        public ulong LastPoseChecksum { get; private set; }
        public int SelectedLodLevel { get; private set; }

        /// <summary>
        /// Diagnostic-only copy of the GPU pose matrix stream. This waits for
        /// GPU completion and must not be used in a frame-time path.
        /// </summary>
        public bool TryReadbackGpuBoneMatrices(
            out Matrix4x4[] matrices,
            out string failureReason)
        {
            if (gpuPoseStream == null)
            {
                matrices = Array.Empty<Matrix4x4>();
                failureReason = "gpu_pose_stream_unavailable";
                return false;
            }
            return gpuPoseStream.TryReadbackBoneMatrices(out matrices, out failureReason);
        }

        /// <summary>
        /// Diagnostic-only copy of the CPU fallback matrix stream captured by
        /// the same build inputs as the GPU pose stream. This allocates and is
        /// intentionally excluded from runtime and benchmark hot paths.
        /// </summary>
        public bool TryCopyCpuFallbackBoneMatrices(
            out Matrix4x4[] matrices,
            out string failureReason)
        {
            matrices = Array.Empty<Matrix4x4>();
            failureReason = string.Empty;
            if (!IsBuilt || crowdBoneMatrices == null || crowdBoneMatrices.Length == 0)
            {
                failureReason = "cpu_fallback_matrix_stream_unavailable";
                return false;
            }
            matrices = (Matrix4x4[])crowdBoneMatrices.Clone();
            return true;
        }

        private void Start()
        {
            if (AutoBuild)
            {
                TryBuild(out _);
            }
        }

        private void Update()
        {
            var step = Mathf.Max(0.0f, DeltaTime > 0.0f ? DeltaTime : Time.deltaTime);
            TryStep(step);
        }

        /// <summary>
        /// Advances the shared evaluator synchronously for benchmark hosts
        /// that disable this component's automatic Update. Unity API writes
        /// remain on the main thread; the returned checksum makes the pose
        /// work auditable without a per-agent GameObject loop.
        /// </summary>
        public bool TryStep(float deltaTime)
        {
            LastPoseUpdateCount = 0;
            LastPoseChecksum = 0UL;
            if (!IsBuilt)
            {
                return false;
            }

            var step = Mathf.Max(0.0f, deltaTime);

            if (gpuPoseStream != null && gpuPoseStream.IsReady && Animate)
            {
                if (gpuPoseStream.Dispatch(
                    step,
                    AgentColumns,
                    AgentSpacing,
                    AgentOrigin,
                    rootBoneIndex,
                    rootReferencePosition,
                    out var gpuFailure))
                {
                    LastPoseUpdateCount = AgentCount;
                    // GPU animation has no CPU pose checksum. The renderer
                    // and graphics contract use the independent pixel/draw
                    // evidence for this route; CPU fallback remains hashed.
                    LastPoseChecksum = 0UL;
                    return true;
                }

                // A capability or shader failure must never make Play mode
                // invisible. Drop only the optional GPU pose route and keep
                // the already-built managed evaluator alive.
                FailureReason = gpuFailure;
                if (rendererHost != null)
                {
                    rendererHost.SetExternalBoneMatrices(null, 0);
                }
                gpuPoseStream.Dispose();
                gpuPoseStream = null;
            }

            var evaluationSpeeds = Animate ? agentSpeeds : stoppedAgentSpeeds;
            evaluator.StepAll(step, evaluationSpeeds, JadrenAnimationLod.Full);
            for (var agent = 0; agent < AgentCount; agent++)
            {
                var pose = evaluator.GetPose(agent);
                if (pose == null || pose.SampledBoneCount <= 0)
                {
                    continue;
                }

                WriteAgentBoneMatrices(agent, pose);
                LastPoseUpdateCount++;
                LastPoseChecksum ^= pose.Checksum + (ulong)agent;
            }
            return true;
        }

        private void OnDestroy()
        {
            Dispose();
        }

        /// <summary>
        /// Builds the shared GPU fixture from a prefab with baked Jadren
        /// authoring data. No prefab instances are created for the crowd.
        /// </summary>
        public bool TryBuild(out string failureReason)
        {
            failureReason = string.Empty;
            DisposeRuntime();
            disposed = false;
            built = false;
            FailureReason = string.Empty;
            status = "Building";
            LastPoseUpdateCount = 0;
            LastPoseChecksum = 0UL;

            AgentCount = Mathf.Max(1, AgentCount);
            AgentColumns = Mathf.Clamp(AgentColumns, 1, AgentCount);
            AgentSpacing = Mathf.Max(0.1f, AgentSpacing);

            if (CharacterPrefab == null)
            {
                return Fail("character_prefab_missing", out failureReason);
            }

            try
            {
                var animator = CharacterPrefab.GetComponentInChildren<Animator>(true);
                if (animator == null)
                {
                    return Fail("character_animator_missing", out failureReason);
                }

                var authoring = animator.GetComponent<JadrenAnimationAuthoring>();
                if (authoring == null || !authoring.IsConfigured)
                {
                    return Fail("character_baked_authoring_missing", out failureReason);
                }

                var sourceRenderer = FindSourceRenderer(
                    CharacterPrefab,
                    SourceLodLevel,
                    out var selectedLodLevel);
                SelectedLodLevel = selectedLodLevel;
                if (sourceRenderer == null || sourceRenderer.sharedMesh == null)
                {
                    return Fail("character_skinned_mesh_missing", out failureReason);
                }
                if (sourceRenderer.bones == null || sourceRenderer.bones.Length < 1)
                {
                    return Fail("character_mesh_bones_missing", out failureReason);
                }

                rig = authoring.Rig;
                controller = authoring.Controller;
                rootBoneIndex = FindRootBoneIndex(rig);
                rootReferencePosition = Vector3.zero;
                rootReferenceCaptured = false;
                sourceMesh = sourceRenderer.sharedMesh;
                sourceBindposes = sourceMesh.bindposes;
                if (sourceBindposes == null || sourceBindposes.Length != sourceRenderer.bones.Length)
                {
                    return Fail("character_mesh_bindposes_invalid", out failureReason);
                }
                sourceMaterials = sourceRenderer.sharedMaterials;
                if (sourceMaterials == null || sourceMaterials.Length == 0)
                {
                    sourceMaterials = new[] { (Material)null };
                }
                if (sourceMaterials.Length != 1
                    && sourceMaterials.Length != Mathf.Max(1, sourceMesh.subMeshCount))
                {
                    return Fail("character_material_count_not_submesh_count", out failureReason);
                }
                sourceMaterial = sourceMaterials[0];
                meshBoneToRig = BuildMeshBoneMap(animator.transform, sourceRenderer.bones, rig);
                if (meshBoneToRig.Length != sourceRenderer.bones.Length)
                {
                    return Fail("character_mesh_bone_map_invalid", out failureReason);
                }

                BuildSharedVertices(sourceMesh);
                crowdBoneMatrices = new Matrix4x4[AgentCount * meshBoneToRig.Length];
                rigBoneMatrices = new Matrix4x4[rig.BoneCount];
                agentSpeeds = new float[AgentCount];
                stoppedAgentSpeeds = new float[AgentCount];
                for (var agent = 0; agent < AgentCount; agent++)
                {
                    if (UniformSpeed >= 0.0f)
                    {
                        agentSpeeds[agent] = UniformSpeed;
                    }
                    else
                    {
                        var phase = agent * 0.37f;
                        agentSpeeds[agent] = agent % 9 == 0
                            ? 0.0f
                            : 0.8f + Mathf.Abs(Mathf.Sin(phase)) * 2.4f;
                    }
                }

                evaluator = new JadrenAnimationBatchPoseEvaluator(
                    rig,
                    controller,
                    AgentCount,
                    PreferNativeSlerp,
                    PreferNativePoseTiles);
                evaluator.StepAll(0.0f, agentSpeeds, JadrenAnimationLod.Full);
                for (var agent = 0; agent < AgentCount; agent++)
                {
                    var pose = evaluator.GetPose(agent);
                    if (pose != null && pose.SampledBoneCount > 0)
                    {
                        CaptureRootReference(pose);
                        WriteAgentBoneMatrices(agent, pose);
                    }
                }

                rendererHost = GetComponent<JadrenGpuSkinningCrowdRenderer>();
                if (rendererHost == null)
                {
                    rendererHost = gameObject.AddComponent<JadrenGpuSkinningCrowdRenderer>();
                }
                if (SkinningShader != null)
                {
                    rendererHost.SetSkinningShader(SkinningShader);
                }
                if (CrowdShader != null)
                {
                    rendererHost.SetCrowdShader(CrowdShader);
                }
                rendererHost.SetDrawBounds(BuildDrawBounds());
                if (!rendererHost.TrySetSharedCrowdMaterials(
                        sourceMesh,
                        sourceMaterials,
                        crowdVertices,
                        crowdBoneMatrices,
                        AgentCount,
                        meshBoneToRig.Length,
                        out var rendererFailure))
                {
                    return Fail(rendererFailure, out failureReason);
                }
                rendererHost.SetGpuSkinningEnabled(true);
                if (!rendererHost.IsReady)
                {
                    return Fail(rendererHost.LastFailureReason, out failureReason);
                }

                if (PreferGpuAnimation)
                {
                    var poseShader = SkinningShader != null
                        ? SkinningShader
                        : Resources.Load<ComputeShader>("JadrenAnimationGpuSkinning");
                    if (JadrenAnimationGpuCrowdPoseStream.TryCreate(
                        rig,
                        controller,
                        meshBoneToRig,
                        sourceBindposes,
                        rootBoneIndex,
                        rootReferencePosition,
                        agentSpeeds,
                        AgentCount,
                        poseShader,
                        out var candidate,
                        out var poseFailure))
                    {
                        gpuPoseStream = candidate;
                        rendererHost.SetExternalBoneMatrices(
                            gpuPoseStream.BoneMatrixBuffer,
                            AgentCount * meshBoneToRig.Length);
                    }
                    else
                    {
                        FailureReason = poseFailure;
                    }
                }

                built = true;
                status = gpuPoseStream != null
                    ? "Ready (Jadren GPU animation + GPU skinning)"
                    : "Ready (Jadren batch + per-agent GPU skinning)";
                return true;
            }
            catch (Exception exception)
            {
                return Fail(
                    exception.GetType().Name + ":" + exception.Message,
                    out failureReason);
            }
        }

        public void Dispose()
        {
            if (disposed)
            {
                return;
            }
            disposed = true;
            DisposeRuntime();
            GC.SuppressFinalize(this);
        }

        private bool Fail(string reason, out string failureReason)
        {
            failureReason = string.IsNullOrEmpty(reason) ? "unknown_failure" : reason;
            FailureReason = failureReason;
            status = "GPU crowd unavailable: " + failureReason;
            DisposeRuntime();
            return false;
        }

        private void DisposeRuntime()
        {
            if (rendererHost != null)
            {
                rendererHost.SetExternalBoneMatrices(null, 0);
                rendererHost.SetGpuSkinningEnabled(false);
                if (rendererHost.gameObject == gameObject)
                {
                    rendererHost = null;
                }
            }
            if (evaluator != null)
            {
                evaluator.Dispose();
                evaluator = null;
            }
            if (gpuPoseStream != null)
            {
                gpuPoseStream.Dispose();
                gpuPoseStream = null;
            }
            sourceMesh = null;
            sourceMaterial = null;
            sourceBindposes = Array.Empty<Matrix4x4>();
            crowdVertices = Array.Empty<JadrenGpuSkinningVertex>();
            crowdBoneMatrices = Array.Empty<Matrix4x4>();
            rigBoneMatrices = Array.Empty<Matrix4x4>();
            meshBoneToRig = Array.Empty<int>();
            agentSpeeds = Array.Empty<float>();
            stoppedAgentSpeeds = Array.Empty<float>();
            rootBoneIndex = -1;
            rootReferencePosition = Vector3.zero;
            rootReferenceCaptured = false;
            built = false;
            LastPoseUpdateCount = 0;
            LastPoseChecksum = 0UL;
        }

        private void BuildSharedVertices(Mesh mesh)
        {
            var sourceVertices = mesh.vertices;
            var sourceWeights = mesh.boneWeights;
            if (sourceVertices == null || sourceVertices.Length < 1
                || sourceWeights == null || sourceWeights.Length != sourceVertices.Length)
            {
                throw new InvalidOperationException("character_mesh_skin_data_invalid");
            }

            crowdVertices = new JadrenGpuSkinningVertex[sourceVertices.Length];
            for (var vertex = 0; vertex < sourceVertices.Length; vertex++)
            {
                var weights = sourceWeights[vertex];
                crowdVertices[vertex] = new JadrenGpuSkinningVertex(
                    sourceVertices[vertex],
                    new Vector4(weights.weight0, weights.weight1, weights.weight2, weights.weight3),
                    new Vector4(
                        weights.boneIndex0,
                        weights.boneIndex1,
                        weights.boneIndex2,
                        weights.boneIndex3));
            }
        }

        private void WriteAgentBoneMatrices(int agentIndex, JadrenPoseBuffer pose)
        {
            if (pose == null || rigBoneMatrices.Length != rig.BoneCount)
            {
                return;
            }

            for (var rigBone = 0; rigBone < rig.BoneCount; rigBone++)
            {
                var local = ComposeTrs(
                    NormalizedBonePosition(pose, rigBone),
                    pose.Rotations[rigBone],
                    pose.Scales[rigBone]);
                var parent = rig.GetParentIndex(rigBone);
                rigBoneMatrices[rigBone] = parent >= 0
                    ? MultiplyAffine(rigBoneMatrices[parent], local)
                    : local;
            }

            var agentPosition = PositionFor(agentIndex);
            var matrixOffset = agentIndex * meshBoneToRig.Length;
            for (var meshBone = 0; meshBone < meshBoneToRig.Length; meshBone++)
            {
                var rigBone = meshBoneToRig[meshBone];
                var world = rigBoneMatrices[rigBone];
                // The crowd root is a translation-only matrix. Applying it
                // directly avoids two general Matrix4x4 multiplications for
                // every mesh bone while preserving the affine result.
                world.m03 += agentPosition.x;
                world.m13 += agentPosition.y;
                world.m23 += agentPosition.z;
                crowdBoneMatrices[matrixOffset + meshBone] = MultiplyAffine(
                    world,
                    sourceBindposes[meshBone]);
            }
        }

        private static Matrix4x4 ComposeTrs(
            Vector3 position,
            Quaternion rotation,
            Vector3 scale)
        {
            // Unity's Matrix4x4.TRS is equivalent to this column-major affine
            // construction. Keeping it in managed value math avoids a native
            // helper call in the crowd hot path.
            var x = rotation.x;
            var y = rotation.y;
            var z = rotation.z;
            var w = rotation.w;
            var xx = x + x;
            var yy = y + y;
            var zz = z + z;
            var xx2 = x * xx;
            var yy2 = y * yy;
            var zz2 = z * zz;
            var xy = x * yy;
            var xz = x * zz;
            var yz = y * zz;
            var wx = w * xx;
            var wy = w * yy;
            var wz = w * zz;
            return new Matrix4x4
            {
                m00 = (1.0f - (yy2 + zz2)) * scale.x,
                m01 = (xy - wz) * scale.y,
                m02 = (xz + wy) * scale.z,
                m03 = position.x,
                m10 = (xy + wz) * scale.x,
                m11 = (1.0f - (xx2 + zz2)) * scale.y,
                m12 = (yz - wx) * scale.z,
                m13 = position.y,
                m20 = (xz - wy) * scale.x,
                m21 = (yz + wx) * scale.y,
                m22 = (1.0f - (xx2 + yy2)) * scale.z,
                m23 = position.z,
                m30 = 0.0f,
                m31 = 0.0f,
                m32 = 0.0f,
                m33 = 1.0f
            };
        }

        private static Matrix4x4 MultiplyAffine(Matrix4x4 left, Matrix4x4 right)
        {
            return new Matrix4x4
            {
                m00 = left.m00 * right.m00 + left.m01 * right.m10 + left.m02 * right.m20,
                m01 = left.m00 * right.m01 + left.m01 * right.m11 + left.m02 * right.m21,
                m02 = left.m00 * right.m02 + left.m01 * right.m12 + left.m02 * right.m22,
                m03 = left.m00 * right.m03 + left.m01 * right.m13 + left.m02 * right.m23 + left.m03,
                m10 = left.m10 * right.m00 + left.m11 * right.m10 + left.m12 * right.m20,
                m11 = left.m10 * right.m01 + left.m11 * right.m11 + left.m12 * right.m21,
                m12 = left.m10 * right.m02 + left.m11 * right.m12 + left.m12 * right.m22,
                m13 = left.m10 * right.m03 + left.m11 * right.m13 + left.m12 * right.m23 + left.m13,
                m20 = left.m20 * right.m00 + left.m21 * right.m10 + left.m22 * right.m20,
                m21 = left.m20 * right.m01 + left.m21 * right.m11 + left.m22 * right.m21,
                m22 = left.m20 * right.m02 + left.m21 * right.m12 + left.m22 * right.m22,
                m23 = left.m20 * right.m03 + left.m21 * right.m13 + left.m22 * right.m23 + left.m23,
                m30 = 0.0f,
                m31 = 0.0f,
                m32 = 0.0f,
                m33 = 1.0f
            };
        }

        private Bounds BuildDrawBounds()
        {
            var bounds = new Bounds(PositionFor(0), Vector3.one);
            for (var agent = 1; agent < AgentCount; agent++)
            {
                bounds.Encapsulate(PositionFor(agent));
            }
            var extent = sourceMesh == null ? Vector3.one : sourceMesh.bounds.extents;
            bounds.Expand((extent + Vector3.one * 2.0f) * 2.0f);
            return bounds;
        }

        private static SkinnedMeshRenderer FindSourceRenderer(
            GameObject root,
            int requestedLod,
            out int selectedLod)
        {
            selectedLod = 0;
            if (root == null)
            {
                return null;
            }

            var lodGroup = root.GetComponentInChildren<LODGroup>(true);
            if (lodGroup != null)
            {
                var lods = lodGroup.GetLODs();
                if (lods != null && lods.Length > 0)
                {
                    var preferred = Mathf.Clamp(requestedLod, 0, lods.Length - 1);
                    for (var level = preferred; level < lods.Length; level++)
                    {
                        var candidate = FirstSkinnedMeshRenderer(lods[level].renderers);
                        if (candidate != null)
                        {
                            selectedLod = level;
                            return candidate;
                        }
                    }
                    for (var level = 0; level < preferred; level++)
                    {
                        var candidate = FirstSkinnedMeshRenderer(lods[level].renderers);
                        if (candidate != null)
                        {
                            selectedLod = level;
                            return candidate;
                        }
                    }
                }
            }

            return root.GetComponentInChildren<SkinnedMeshRenderer>(true);
        }

        private static SkinnedMeshRenderer FirstSkinnedMeshRenderer(Renderer[] renderers)
        {
            if (renderers == null)
            {
                return null;
            }
            for (var index = 0; index < renderers.Length; index++)
            {
                var renderer = renderers[index] as SkinnedMeshRenderer;
                if (renderer != null && renderer.sharedMesh != null)
                {
                    return renderer;
                }
            }
            return null;
        }

        private void CaptureRootReference(JadrenPoseBuffer pose)
        {
            if (rootReferenceCaptured || pose == null || rootBoneIndex < 0)
            {
                return;
            }

            rootReferencePosition = pose.Positions[rootBoneIndex];
            rootReferenceCaptured = true;
        }

        private Vector3 NormalizedBonePosition(JadrenPoseBuffer pose, int rigBone)
        {
            var position = pose.Positions[rigBone];
            return rigBone == rootBoneIndex
                ? position - rootReferencePosition
                : position;
        }

        private Vector3 PositionFor(int agentIndex)
        {
            var columns = Mathf.Max(1, AgentColumns);
            var row = agentIndex / columns;
            var column = agentIndex % columns;
            return AgentOrigin + new Vector3(
                (column - (columns - 1) * 0.5f) * AgentSpacing,
                0.0f,
                row * AgentSpacing);
        }

        private static int[] BuildMeshBoneMap(
            Transform root,
            Transform[] meshBones,
            JadrenRigAsset bakedRig)
        {
            var map = new int[meshBones.Length];
            for (var index = 0; index < meshBones.Length; index++)
            {
                var path = RelativePath(root, meshBones[index]);
                if (path == null || !bakedRig.TryGetBoneIndex(path, out map[index]))
                {
                    return Array.Empty<int>();
                }
            }
            return map;
        }

        private static int FindRootBoneIndex(JadrenRigAsset bakedRig)
        {
            if (bakedRig == null)
            {
                return -1;
            }

            for (var index = 0; index < bakedRig.BoneCount; index++)
            {
                if (string.IsNullOrEmpty(bakedRig.GetBonePath(index)))
                {
                    return index;
                }
            }
            return -1;
        }

        private static string RelativePath(Transform root, Transform target)
        {
            if (root == null || target == null)
            {
                return null;
            }
            if (root == target)
            {
                return string.Empty;
            }

            var parts = new List<string>();
            var current = target;
            while (current != null && current != root)
            {
                parts.Add(current.name);
                current = current.parent;
            }
            if (current != root)
            {
                return null;
            }
            parts.Reverse();
            return string.Join("/", parts.ToArray());
        }
    }
}
