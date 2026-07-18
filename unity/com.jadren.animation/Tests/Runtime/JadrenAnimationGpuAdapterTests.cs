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
    }
}
