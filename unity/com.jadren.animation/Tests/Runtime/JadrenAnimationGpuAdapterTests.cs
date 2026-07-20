using NUnit.Framework;
using UnityEngine;
using UnityEngine.Rendering;

namespace Jadren.Animation.Tests
{
    public sealed class JadrenAnimationGpuAdapterTests
    {
        private static JadrenAnimationGpuRequest Ready(
            JadrenAnimationGpuTarget target,
            bool allowCpuFallback)
        {
            return new JadrenAnimationGpuRequest(
                target,
                128,
                true,
                true,
                allowCpuFallback);
        }

        [Test]
        public void AutoPrefersEntitiesWhenAllCapabilitiesArePresent()
        {
            var capabilities = new JadrenAnimationGpuCapabilities(
                true, true, true, true, true, GraphicsDeviceType.Direct3D11);
            var plan = JadrenAnimationGpuAdapter.Plan(
                Ready(JadrenAnimationGpuTarget.Auto, false), capabilities);

            Assert.That(plan.IsSupported, Is.True);
            Assert.That(plan.Route, Is.EqualTo(JadrenAnimationGpuRoute.EntitiesGraphics));
        }

        [Test]
        public void AutoUsesComputeWhenEntitiesIsUnavailable()
        {
            var capabilities = new JadrenAnimationGpuCapabilities(
                true, true, true, false, true, GraphicsDeviceType.Direct3D11);
            var plan = JadrenAnimationGpuAdapter.Plan(
                Ready(JadrenAnimationGpuTarget.Auto, false), capabilities);

            Assert.That(plan.IsSupported, Is.True);
            Assert.That(plan.Route, Is.EqualTo(JadrenAnimationGpuRoute.ComputeShader));
        }

        [Test]
        public void MissingBoundsUsesExplicitCpuFallback()
        {
            var capabilities = new JadrenAnimationGpuCapabilities(
                true, true, true, true, true, GraphicsDeviceType.Direct3D11);
            var plan = JadrenAnimationGpuAdapter.Plan(
                new JadrenAnimationGpuRequest(
                    JadrenAnimationGpuTarget.Auto, 128, false, true, true), capabilities);

            Assert.That(plan.UsesCpuFallback, Is.True);
            Assert.That(plan.Reason, Is.EqualTo("bounds_unavailable"));
        }

        [Test]
        public void MissingResidentBufferCanBeHardRejected()
        {
            var capabilities = new JadrenAnimationGpuCapabilities(
                true, true, true, true, true, GraphicsDeviceType.Direct3D11);
            var plan = JadrenAnimationGpuAdapter.Plan(
                new JadrenAnimationGpuRequest(
                    JadrenAnimationGpuTarget.ComputeShader, 128, true, false, false), capabilities);

            Assert.That(plan.IsRejected, Is.True);
            Assert.That(plan.Reason, Is.EqualTo("buffer_not_resident"));
        }

        [Test]
        public void GpuPoseResultValidatesReducedSampleCountBeforePublication()
        {
            using (var result = new JadrenAnimationGpuPoseResult(3, JadrenAnimationLod.Reduced))
            {
                var source = new[]
                {
                    Quaternion.identity,
                    Quaternion.Euler(0.0f, 45.0f, 0.0f),
                    Quaternion.Euler(0.0f, 90.0f, 0.0f)
                };

                Assert.That(result.TryPublishCompleted(source, 3), Is.False);
                Assert.That(result.IsComplete, Is.False);
                Assert.That(result.FailureReason, Is.EqualTo("sample_count_invalid"));
                Assert.That(result.TryPublishCompleted(source, 2), Is.True);
                Assert.That(result.Succeeded, Is.True);
                Assert.That(result.SampledBoneCount, Is.EqualTo(2));
                Assert.That(result.TryGetRotation(0, out _), Is.True);
                Assert.That(result.TryGetRotation(1, out _), Is.False);
                Assert.That(result.TryGetRotation(2, out _), Is.True);
            }
        }

        [Test]
        public void GpuSkinningVertexRejectsNonIntegralOrOutOfRangeBoneIndices()
        {
            Assert.That(
                System.Runtime.InteropServices.Marshal.SizeOf(typeof(JadrenGpuSkinningVertex)),
                Is.EqualTo(JadrenGpuSkinningVertex.StrideBytes));
            var invalid = new JadrenGpuSkinningVertex(
                Vector3.zero,
                new Vector4(1.0f, 0.0f, 0.0f, 0.0f),
                new Vector4(1.5f, 0.0f, 0.0f, 0.0f));
            Assert.That(invalid.TryValidate(2, out var reason), Is.False);
            Assert.That(reason, Is.EqualTo("bone_index_invalid"));

            var outOfRange = new JadrenGpuSkinningVertex(
                Vector3.zero,
                new Vector4(1.0f, 0.0f, 0.0f, 0.0f),
                new Vector4(2.0f, 0.0f, 0.0f, 0.0f));
            Assert.That(outOfRange.TryValidate(2, out reason), Is.False);
            Assert.That(reason, Is.EqualTo("bone_index_invalid"));
        }

        [Test]
        public void GpuSkinningResultPublishesOnlyFiniteCompletedPositions()
        {
            using (var result = new JadrenAnimationGpuSkinningResult(2))
            {
                Assert.That(
                    result.TryPublishCompleted(
                        new[] { new Vector3(float.NaN, 0.0f, 0.0f), Vector3.zero }),
                    Is.False);
                Assert.That(result.IsComplete, Is.False);
                Assert.That(result.FailureReason, Is.EqualTo("position_non_finite"));
                Assert.That(
                    result.TryPublishCompleted(
                        new[] { new Vector3(1.0f, 2.0f, 3.0f), Vector3.zero }),
                    Is.True);
                Assert.That(result.Succeeded, Is.True);
                Assert.That(result.TryGetPosition(0, out var position), Is.True);
                Assert.That(position, Is.EqualTo(new Vector3(1.0f, 2.0f, 3.0f)));
            }
        }

        [Test]
        public void GpuSkinnedMeshRendererRejectsMissingMeshWithoutDisablingSource()
        {
            var hostObject = new GameObject("JadrenGpuSkinnedMeshRendererTest");
            try
            {
                var source = hostObject.AddComponent<SkinnedMeshRenderer>();
                var host = hostObject.AddComponent<JadrenGpuSkinnedMeshRenderer>();
                host.SetGpuSkinningEnabled(false);

                Assert.That(host.TryInitialize(out var reason), Is.False);
                Assert.That(reason, Is.EqualTo("source_mesh_missing"));
                Assert.That(source.enabled, Is.True);
                Assert.That(host.IsReady, Is.False);
            }
            finally
            {
                Object.DestroyImmediate(hostObject);
            }
        }

        [Test]
        public void GpuCrowdRendererAcceptsIndependentAgentBoneLayout()
        {
            var hostObject = new GameObject("JadrenGpuSkinningCrowdLayoutTest");
            var mesh = new Mesh
            {
                name = "JadrenGpuSkinningCrowdLayoutMesh",
                vertices = new[]
                {
                    Vector3.zero,
                    Vector3.right,
                    Vector3.up
                },
                triangles = new[] { 0, 1, 2 }
            };
            try
            {
                var host = hostObject.AddComponent<JadrenGpuSkinningCrowdRenderer>();
                var vertices = new JadrenGpuSkinningVertex[6];
                for (var index = 0; index < vertices.Length; index++)
                {
                    vertices[index] = new JadrenGpuSkinningVertex(
                        Vector3.zero,
                        new Vector4(1.0f, 0.0f, 0.0f, 0.0f),
                        Vector4.zero);
                }
                var matrices = new Matrix4x4[4];
                Assert.That(
                    host.TrySetCrowdData(mesh, null, vertices, matrices, 2, out var reason),
                    Is.True,
                    reason);
                Assert.That(host.AgentCount, Is.EqualTo(2));
                Assert.That(host.VerticesPerAgent, Is.EqualTo(3));
                Assert.That(host.BonesPerAgent, Is.EqualTo(2));

                Assert.That(
                    host.TrySetCrowdData(
                        mesh,
                        null,
                        vertices,
                        new Matrix4x4[3],
                        2,
                        out reason),
                    Is.False);
                Assert.That(reason, Is.EqualTo("crowd_bone_matrices_not_agent_multiple"));
            }
            finally
            {
                Object.DestroyImmediate(mesh);
                Object.DestroyImmediate(hostObject);
            }
        }

        [Test]
        public void GpuCrowdRendererAcceptsSharedMeshLayout()
        {
            var hostObject = new GameObject("JadrenGpuSkinningSharedLayoutTest");
            var mesh = new Mesh
            {
                name = "JadrenGpuSkinningSharedMesh",
                vertices = new[]
                {
                    Vector3.zero,
                    Vector3.right,
                    Vector3.up
                },
                triangles = new[] { 0, 1, 2 }
            };
            try
            {
                var host = hostObject.AddComponent<JadrenGpuSkinningCrowdRenderer>();
                var vertices = new JadrenGpuSkinningVertex[3];
                for (var index = 0; index < vertices.Length; index++)
                {
                    vertices[index] = new JadrenGpuSkinningVertex(
                        Vector3.zero,
                        new Vector4(1.0f, 0.0f, 0.0f, 0.0f),
                        Vector4.zero);
                }
                Assert.That(
                    host.TrySetSharedCrowdData(
                        mesh,
                        null,
                        vertices,
                        new Matrix4x4[4],
                        2,
                        2,
                        out var reason),
                    Is.True,
                    reason);
                Assert.That(host.AgentCount, Is.EqualTo(2));
                Assert.That(host.VerticesPerAgent, Is.EqualTo(3));
                Assert.That(host.BonesPerAgent, Is.EqualTo(2));
                Assert.That(host.UsesSharedVertexLayout, Is.True);

                Assert.That(
                    host.TrySetSharedCrowdData(
                        mesh,
                        null,
                        vertices,
                        new Matrix4x4[2],
                        2,
                        2,
                        out reason),
                    Is.False);
                Assert.That(reason, Is.EqualTo("crowd_bone_matrices_not_agent_multiple"));
            }
            finally
            {
                Object.DestroyImmediate(mesh);
                Object.DestroyImmediate(hostObject);
            }
        }

        [Test]
        public void GpuCrowdRendererPreservesMultiMaterialSubmeshLayout()
        {
            var hostObject = new GameObject("JadrenGpuSkinningMultiMaterialTest");
            var mesh = new Mesh
            {
                name = "JadrenGpuSkinningMultiMaterialMesh",
                vertices = new[]
                {
                    Vector3.zero,
                    Vector3.right,
                    Vector3.up,
                    Vector3.forward
                }
            };
            mesh.subMeshCount = 2;
            mesh.SetIndices(new[] { 0, 1, 2 }, MeshTopology.Triangles, 0);
            mesh.SetIndices(new[] { 0, 2, 3 }, MeshTopology.Triangles, 1);
            try
            {
                var host = hostObject.AddComponent<JadrenGpuSkinningCrowdRenderer>();
                var vertices = new JadrenGpuSkinningVertex[4];
                for (var index = 0; index < vertices.Length; index++)
                {
                    vertices[index] = new JadrenGpuSkinningVertex(
                        mesh.vertices[index],
                        new Vector4(1.0f, 0.0f, 0.0f, 0.0f),
                        Vector4.zero);
                }

                Assert.That(
                    host.TrySetSharedCrowdMaterials(
                        mesh,
                        new Material[] { null, null },
                        vertices,
                        new Matrix4x4[2],
                        2,
                        1,
                        out var reason),
                    Is.True,
                    reason);
                Assert.That(host.MaterialCount, Is.EqualTo(2));
                Assert.That(host.DrawSubmeshCount, Is.EqualTo(2));
                Assert.That(host.AgentCount, Is.EqualTo(2));

                Assert.That(
                    host.TrySetSharedCrowdMaterials(
                        mesh,
                        new Material[] { null, null, null },
                        vertices,
                        new Matrix4x4[2],
                        2,
                        1,
                        out reason),
                    Is.False);
                Assert.That(reason, Is.EqualTo("crowd_material_count_not_submesh_count"));
            }
            finally
            {
                Object.DestroyImmediate(mesh);
                Object.DestroyImmediate(hostObject);
            }
        }
    }
}
