using ArgumentOutOfRangeException = System.ArgumentOutOfRangeException;
using NUnit.Framework;
using UnityEngine;

namespace Jadren.Animation.Tests
{
    public sealed class JadrenAnimationPosePipelineTests
    {
        [Test]
        public void SlerpUnclampedMatchesUnityReferenceAcrossContractSamples()
        {
            var endpoints = new[]
            {
                new Quaternion(0.0f, 0.0f, 0.0f, 1.0f),
                Quaternion.Euler(17.0f, -38.0f, 71.0f),
                Quaternion.Euler(-120.0f, 48.0f, 15.0f)
            };
            var weights = new[] { -0.75f, 0.0f, 0.125f, 0.5f, 1.0f, 1.75f };

            for (var pair = 0; pair < endpoints.Length - 1; pair++)
            {
                var a = endpoints[pair];
                var b = endpoints[pair + 1];
                foreach (var weight in weights)
                {
                    var expected = Quaternion.SlerpUnclamped(a, b, weight);
                    var actual = JadrenQuaternionMath.SlerpUnclamped(a, b, weight);
                    Assert.That(
                        Quaternion.Angle(actual, expected),
                        Is.LessThan(0.0005f),
                        $"pair={pair}, weight={weight}");
                }
            }
        }

        [Test]
        public void SlerpUnclampedUsesShortestArcAndNearLinearFallback()
        {
            var a = Quaternion.Euler(10.0f, 20.0f, 30.0f);
            var sameRotationOppositeSign = new Quaternion(-a.x, -a.y, -a.z, -a.w);
            var shortest = JadrenQuaternionMath.SlerpUnclamped(a, sameRotationOppositeSign, 0.5f);
            Assert.That(Quaternion.Angle(shortest, a), Is.LessThan(0.0005f));

            var near = Quaternion.Normalize(new Quaternion(0.00001f, 0.0f, 0.0f, 1.0f));
            var nearExpected = Quaternion.SlerpUnclamped(a, near, 0.25f);
            var nearActual = JadrenQuaternionMath.SlerpUnclamped(a, near, 0.25f);
            Assert.That(Quaternion.Angle(nearActual, nearExpected), Is.LessThan(0.0005f));

            var extrapolated = JadrenQuaternionMath.SlerpUnclamped(a, near, -0.5f);
            Assert.That(extrapolated.x, Is.Not.NaN);
            Assert.That(extrapolated.w, Is.Not.NaN);
        }

        [Test]
        public void WorkerSamplesCrossFadeWithJadrenSlerp()
        {
            var rig = CreateRig(string.Empty);
            var previousClip = CreateClip(
                Quaternion.identity,
                Quaternion.Euler(0.0f, 0.0f, 0.0f));
            var currentClip = CreateClip(
                Quaternion.Euler(0.0f, 90.0f, 0.0f),
                Quaternion.Euler(0.0f, 90.0f, 0.0f));
            var controller = ScriptableObject.CreateInstance<JadrenControllerAsset>();
            controller.SetBakedData(
                new[]
                {
                    new JadrenAnimationStateDefinition { name = "Previous", clip = previousClip, playbackSpeed = 1.0f },
                    new JadrenAnimationStateDefinition { name = "Current", clip = currentClip, playbackSpeed = 1.0f }
                },
                new JadrenAnimationTransition[0]);

            try
            {
                var worker = new JadrenAnimationPoseWorker(rig, controller);
                var output = new JadrenPoseBuffer();
                var sampled = worker.Evaluate(
                    1,
                    0.0f,
                    0.0f,
                    0,
                    0.0f,
                    0.5f,
                    JadrenAnimationLod.Full,
                    output);

                Assert.That(sampled, Is.EqualTo(1));
                Assert.That(output.SampledBoneCount, Is.EqualTo(1));
                var expected = JadrenQuaternionMath.SlerpUnclamped(
                    Quaternion.identity,
                    Quaternion.Euler(0.0f, 90.0f, 0.0f),
                    0.5f);
                Assert.That(Quaternion.Angle(output.Rotations[0], expected), Is.LessThan(0.0005f));
            }
            finally
            {
                Object.DestroyImmediate(controller);
                Object.DestroyImmediate(currentClip);
                Object.DestroyImmediate(previousClip);
                Object.DestroyImmediate(rig);
            }
        }

        [Test]
        public void WorkerAndScalarFallbackPreserveUnclampedCrossFadeWeights()
        {
            var rig = CreateRig(string.Empty);
            var previousClip = CreateClip(Quaternion.identity, Quaternion.identity);
            var currentClip = CreateClip(
                Quaternion.Euler(0.0f, 90.0f, 0.0f),
                Quaternion.Euler(0.0f, 90.0f, 0.0f));
            var controller = ScriptableObject.CreateInstance<JadrenControllerAsset>();
            controller.SetBakedData(
                new[]
                {
                    new JadrenAnimationStateDefinition { name = "Previous", clip = previousClip, playbackSpeed = 1.0f },
                    new JadrenAnimationStateDefinition { name = "Current", clip = currentClip, playbackSpeed = 1.0f }
                },
                new JadrenAnimationTransition[0]);

            try
            {
                var worker = new JadrenAnimationPoseWorker(rig, controller);
                foreach (var weight in new[] { -0.5f, 1.5f })
                {
                    var workerOutput = new JadrenPoseBuffer();
                    var scalarOutput = new JadrenPoseBuffer();
                    worker.Evaluate(
                        1, 0.0f, 0.0f, 0, 0.0f, weight,
                        JadrenAnimationLod.Full, workerOutput);
                    JadrenPoseKernel.Sample(
                        rig, currentClip, 0.0f, 0.0f, previousClip, 0.0f, weight,
                        JadrenAnimationLod.Full, scalarOutput);

                    var expected = JadrenQuaternionMath.SlerpUnclamped(
                        Quaternion.identity,
                        Quaternion.Euler(0.0f, 90.0f, 0.0f),
                        weight);
                    Assert.That(Quaternion.Angle(workerOutput.Rotations[0], expected), Is.LessThan(0.0005f));
                    Assert.That(Quaternion.Angle(scalarOutput.Rotations[0], expected), Is.LessThan(0.0005f));
                }

                Assert.Throws<ArgumentOutOfRangeException>(() => worker.Evaluate(
                    1, 0.0f, 0.0f, 0, 0.0f, float.NaN,
                    JadrenAnimationLod.Full, new JadrenPoseBuffer()));
                Assert.Throws<ArgumentOutOfRangeException>(() => JadrenPoseKernel.Sample(
                    rig, currentClip, 0.0f, 0.0f, previousClip, 0.0f, float.PositiveInfinity,
                    JadrenAnimationLod.Full, new JadrenPoseBuffer()));
            }
            finally
            {
                Object.DestroyImmediate(controller);
                Object.DestroyImmediate(currentClip);
                Object.DestroyImmediate(previousClip);
                Object.DestroyImmediate(rig);
            }
        }

        [Test]
        public void WorkerNativeSlerpBridgeMatchesManagedPoseWhenAvailable()
        {
            var rig = CreateRig(string.Empty);
            var previousClip = CreateClip(
                Quaternion.identity,
                Quaternion.identity);
            var currentClip = CreateClip(
                Quaternion.Euler(0.0f, 90.0f, 0.0f),
                Quaternion.Euler(0.0f, 90.0f, 0.0f));
            var controller = ScriptableObject.CreateInstance<JadrenControllerAsset>();
            controller.SetBakedData(
                new[]
                {
                    new JadrenAnimationStateDefinition { name = "Previous", clip = previousClip, playbackSpeed = 1.0f },
                    new JadrenAnimationStateDefinition { name = "Current", clip = currentClip, playbackSpeed = 1.0f }
                },
                new JadrenAnimationTransition[0]);

            try
            {
                var managedWorker = new JadrenAnimationPoseWorker(rig, controller);
                var nativeWorker = new JadrenAnimationPoseWorker(rig, controller, preferNativeSlerp: true);
                var managedOutput = new JadrenPoseBuffer();
                var nativeOutput = new JadrenPoseBuffer();
                managedWorker.Evaluate(
                    1, 0.0f, 0.0f, 0, 0.0f, 0.5f,
                    JadrenAnimationLod.Full, managedOutput);
                nativeWorker.Evaluate(
                    1, 0.0f, 0.0f, 0, 0.0f, 0.5f,
                    JadrenAnimationLod.Full, nativeOutput);

                Assert.That(nativeOutput.SampledBoneCount, Is.EqualTo(managedOutput.SampledBoneCount));
                Assert.That(
                    Quaternion.Angle(nativeOutput.Rotations[0], managedOutput.Rotations[0]),
                    Is.LessThan(0.001f));
                if (JadrenAnimationNativeBatch.IsAvailable)
                {
                    Assert.That(nativeWorker.UsesNativeSlerp, Is.True);
                }
            }
            finally
            {
                Object.DestroyImmediate(controller);
                Object.DestroyImmediate(currentClip);
                Object.DestroyImmediate(previousClip);
                Object.DestroyImmediate(rig);
            }
        }

        [Test]
        public void NativeSlerpReportsTheLodSampleCountExpectedByTheWorker()
        {
            if (!JadrenAnimationNativeBatch.IsAvailable)
            {
                Assert.Ignore("Jadren animation native backend is not available in this test runner.");
            }

            var previous = new JadrenAnimationNativePose[3];
            var current = new JadrenAnimationNativePose[3];
            var output = new JadrenAnimationNativePose[3];
            for (var index = 0; index < previous.Length; index++)
            {
                previous[index].RotationW = 1.0f;
                current[index].RotationW = 1.0f;
            }

            Assert.That(
                JadrenAnimationNativeBatch.BlendSlerpUnclamped(
                    previous, current, output, 3, 0.5f, JadrenAnimationLod.Full),
                Is.EqualTo(3));
            Assert.That(
                JadrenAnimationNativeBatch.BlendSlerpUnclamped(
                    previous, current, output, 3, 0.5f, JadrenAnimationLod.Reduced),
                Is.EqualTo(2));
            Assert.That(
                JadrenAnimationNativeBatch.BlendSlerpUnclamped(
                    previous, current, output, 3, 0.5f, JadrenAnimationLod.Hidden),
                Is.EqualTo(0));
        }

        [Test]
        public void WorkerPreparesGpuRotationInputsWithoutUnityObjects()
        {
            var rig = CreateRig(string.Empty);
            var previousClip = CreateClip(Quaternion.identity, Quaternion.identity);
            var currentClip = CreateClip(
                Quaternion.Euler(0.0f, 90.0f, 0.0f),
                Quaternion.Euler(0.0f, 90.0f, 0.0f));
            var controller = ScriptableObject.CreateInstance<JadrenControllerAsset>();
            controller.SetBakedData(
                new[]
                {
                    new JadrenAnimationStateDefinition { name = "Previous", clip = previousClip, playbackSpeed = 1.0f },
                    new JadrenAnimationStateDefinition { name = "Current", clip = currentClip, playbackSpeed = 1.0f }
                },
                new JadrenAnimationTransition[0]);

            try
            {
                var worker = new JadrenAnimationPoseWorker(rig, controller);
                var previous = new Quaternion[1];
                var current = new Quaternion[1];
                var weights = new float[1];
                var sampled = worker.PrepareGpuRotationInputs(
                    1,
                    0.0f,
                    0,
                    0.0f,
                    0.5f,
                    JadrenAnimationLod.Full,
                    previous,
                    current,
                    weights);

                Assert.That(sampled, Is.EqualTo(1));
                Assert.That(weights[0], Is.EqualTo(0.5f));
                Assert.That(Quaternion.Angle(previous[0], Quaternion.identity), Is.LessThan(0.001f));
                Assert.That(
                    Quaternion.Angle(current[0], Quaternion.Euler(0.0f, 90.0f, 0.0f)),
                    Is.LessThan(0.001f));
            }
            finally
            {
                Object.DestroyImmediate(controller);
                Object.DestroyImmediate(currentClip);
                Object.DestroyImmediate(previousClip);
                Object.DestroyImmediate(rig);
            }
        }

        [Test]
        public void PlayerConnectsWorkerToMainThreadApplier()
        {
            var root = new GameObject("JadrenPosePipelineTest");
            var bone = new GameObject("Bone");
            bone.transform.SetParent(root.transform, false);
            var rig = CreateRig("Bone");
            var clip = CreateClip(
                Quaternion.identity,
                Quaternion.Euler(0.0f, 90.0f, 0.0f));
            var controller = ScriptableObject.CreateInstance<JadrenControllerAsset>();
            controller.SetBakedData(
                new[]
                {
                    new JadrenAnimationStateDefinition { name = "Test", clip = clip, playbackSpeed = 1.0f }
                },
                new JadrenAnimationTransition[0]);
            var authoring = root.AddComponent<JadrenAnimationAuthoring>();
            authoring.AssignBakedAssets(rig, controller);
            var player = root.AddComponent<JadrenAnimationPlayer>();

            try
            {
                player.Step(0.5f);
                Assert.That(player.IsReady, Is.True);
                Assert.That(player.SampledBoneCount, Is.EqualTo(1));
                Assert.That(player.PoseChecksum, Is.Not.EqualTo(0UL));
                Assert.That(Quaternion.Angle(bone.transform.localRotation, Quaternion.Euler(0.0f, 45.0f, 0.0f)), Is.LessThan(0.001f));
                Assert.That(player.TryQueueGpuPose(), Is.False);
                Assert.That(player.PollGpuPose(), Is.EqualTo(JadrenAnimationGpuPoseApplyStatus.NoPending));
            }
            finally
            {
                Object.DestroyImmediate(root);
                Object.DestroyImmediate(controller);
                Object.DestroyImmediate(clip);
                Object.DestroyImmediate(rig);
            }
        }

        [Test]
        public void GpuPoseResultAppliesOnlyAfterCompletedReadback()
        {
            var root = new GameObject("JadrenGpuPoseResultTest");
            var bone = new GameObject("Bone");
            bone.transform.SetParent(root.transform, false);
            var rig = CreateRig("Bone");
            var applier = root.AddComponent<JadrenAnimationPoseApplier>();
            applier.RebuildBindings(rig, root.transform);
            var pose = new JadrenPoseBuffer();
            pose.EnsureCapacity(1);
            pose.Positions[0] = Vector3.zero;
            pose.Rotations[0] = Quaternion.identity;
            pose.Scales[0] = Vector3.one;

            using (var result = new JadrenAnimationGpuPoseResult(1, JadrenAnimationLod.Full))
            {
                Assert.That(result.IsComplete, Is.False);
                Assert.That(applier.ApplyGpuResult(pose, result), Is.False);
                Assert.That(bone.transform.localRotation, Is.EqualTo(Quaternion.identity));

                var rotation = Quaternion.Euler(0.0f, 90.0f, 0.0f);
                Assert.That(result.TryPublishCompleted(new[] { rotation }, 1), Is.True);
                Assert.That(result.IsComplete, Is.True);
                Assert.That(result.Succeeded, Is.True);
                Assert.That(applier.ApplyGpuResult(pose, result), Is.True);
                Assert.That(
                    Quaternion.Angle(bone.transform.localRotation, rotation),
                    Is.LessThan(0.001f));
                Assert.That(pose.Checksum, Is.Not.EqualTo(0UL));
            }

            Object.DestroyImmediate(root);
            Object.DestroyImmediate(rig);
        }

        private static JadrenRigAsset CreateRig(string path)
        {
            var rig = ScriptableObject.CreateInstance<JadrenRigAsset>();
            rig.SetBakedData(
                "test",
                new[] { "Root" },
                new[] { path },
                new[] { -1 },
                new[] { Vector3.zero },
                new[] { Quaternion.identity },
                new[] { Vector3.one },
                "test-rig");
            return rig;
        }

        private static JadrenClipAsset CreateClip(Quaternion firstRotation, Quaternion secondRotation)
        {
            var clip = ScriptableObject.CreateInstance<JadrenClipAsset>();
            clip.SetBakedData(
                "test",
                1,
                2,
                1.0f,
                1.0f,
                false,
                new[] { Vector3.zero, Vector3.zero },
                new[] { firstRotation, secondRotation },
                new[] { Vector3.one, Vector3.one },
                "test-clip");
            return clip;
        }
    }
}
