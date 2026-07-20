using ArgumentOutOfRangeException = System.ArgumentOutOfRangeException;
using NUnit.Framework;
using System.Runtime.InteropServices;
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
        public void NativeAoSoA8PoseTileHasStableLayoutAndLinearBlendWhenAvailable()
        {
            Assert.That(
                Marshal.SizeOf(typeof(JadrenAnimationNativeFloat8)),
                Is.EqualTo(JadrenAnimationNativeFloat8.ByteSize));
            Assert.That(
                Marshal.SizeOf(typeof(JadrenAnimationNativePoseTile8)),
                Is.EqualTo(JadrenAnimationNativePoseTile8.ByteSize));
            if (!JadrenAnimationNativeBatch.IsAvailable)
            {
                Assert.Ignore("Jadren animation native backend is not available in this test runner.");
            }

            var previous = new JadrenAnimationNativePoseTile8[1];
            var current = new JadrenAnimationNativePoseTile8[1];
            var output = new JadrenAnimationNativePoseTile8[1];
            previous[0].PositionX = new JadrenAnimationNativeFloat8 { Lane0 = 0.0f, Lane7 = 7.0f };
            previous[0].RotationW = new JadrenAnimationNativeFloat8 { Lane0 = 1.0f, Lane7 = 1.0f };
            current[0].PositionX = new JadrenAnimationNativeFloat8 { Lane0 = 8.0f, Lane7 = 15.0f };
            current[0].RotationW = new JadrenAnimationNativeFloat8 { Lane0 = 1.0f, Lane7 = 1.0f };

            Assert.That(
                JadrenAnimationNativeBatch.BlendLinearAoSoA8(previous, current, output, 1, 0.25f),
                Is.EqualTo(1));
            Assert.That(output[0].PositionX.Lane0, Is.EqualTo(2.0f));
            Assert.That(output[0].PositionX.Lane7, Is.EqualTo(9.0f));

            Assert.That(
                JadrenAnimationNativeBatch.BlendLinearAoSoA8(previous, current, output, 1, -1.0f),
                Is.EqualTo(1));
            Assert.That(output[0].PositionX.Lane0, Is.EqualTo(0.0f));
            Assert.That(output[0].PositionX.Lane7, Is.EqualTo(7.0f));

            Assert.That(
                JadrenAnimationNativeBatch.BlendLinearAoSoA8(previous, current, output, 1, 2.0f),
                Is.EqualTo(1));
            Assert.That(output[0].PositionX.Lane0, Is.EqualTo(8.0f));
            Assert.That(output[0].PositionX.Lane7, Is.EqualTo(15.0f));
        }

        [Test]
        public void NativeAoSoA8WeightedPoseTilesBlendDistinctCrowdWeightsWhenAvailable()
        {
            if (!JadrenAnimationNativeBatch.IsAvailable)
            {
                Assert.Ignore("Jadren animation native backend is not available in this test runner.");
            }

            var previous = new JadrenAnimationNativePoseTile8[2];
            var current = new JadrenAnimationNativePoseTile8[2];
            var output = new JadrenAnimationNativePoseTile8[2];
            var weights = new[] { 0.25f, 0.75f };
            previous[0].PositionX.Lane0 = 0.0f;
            current[0].PositionX.Lane0 = 8.0f;
            previous[1].PositionX.Lane0 = 10.0f;
            current[1].PositionX.Lane0 = 18.0f;

            Assert.That(
                JadrenAnimationNativeBatch.BlendLinearAoSoA8Weighted(
                    previous, current, output, weights, 2),
                Is.EqualTo(2));
            Assert.That(output[0].PositionX.Lane0, Is.EqualTo(2.0f));
            Assert.That(output[1].PositionX.Lane0, Is.EqualTo(16.0f));
        }

        [Test]
        public void WorkerNativePoseTileBridgePreservesManagedTrsWhenAvailable()
        {
            var rig = CreateRig(string.Empty);
            var previousClip = CreatePoseClip(
                new Vector3(-2.0f, 1.0f, 4.0f),
                Quaternion.Euler(0.0f, 10.0f, 0.0f),
                new Vector3(1.0f, 2.0f, 3.0f));
            var currentClip = CreatePoseClip(
                new Vector3(6.0f, 5.0f, -4.0f),
                Quaternion.Euler(0.0f, 90.0f, 0.0f),
                new Vector3(3.0f, 4.0f, 5.0f));
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
                var nativeWorker = new JadrenAnimationPoseWorker(
                    rig,
                    controller,
                    preferNativeSlerp: false,
                    preferNativePoseTiles: true);
                foreach (var weight in new[] { 0.25f, -0.5f, 1.5f })
                {
                    var managedOutput = new JadrenPoseBuffer();
                    var nativeOutput = new JadrenPoseBuffer();
                    managedWorker.Evaluate(
                        1, 0.0f, 0.0f, 0, 0.0f, weight,
                        JadrenAnimationLod.Full, managedOutput);
                    nativeWorker.Evaluate(
                        1, 0.0f, 0.0f, 0, 0.0f, weight,
                        JadrenAnimationLod.Full, nativeOutput);

                    Assert.That(
                        Vector3.Distance(nativeOutput.Positions[0], managedOutput.Positions[0]),
                        Is.LessThan(0.000001f));
                    Assert.That(
                        Vector3.Distance(nativeOutput.Scales[0], managedOutput.Scales[0]),
                        Is.LessThan(0.000001f));
                    Assert.That(
                        Quaternion.Angle(nativeOutput.Rotations[0], managedOutput.Rotations[0]),
                        Is.LessThan(0.0005f));
                    if (weight < 0.0f || weight > 1.0f)
                    {
                        Assert.That(
                            nativeOutput.Checksum,
                            Is.EqualTo(managedOutput.Checksum),
                            $"weight={weight}; managed={DescribePoseBits(managedOutput, 0)}; native={DescribePoseBits(nativeOutput, 0)}");
                    }
                }

                if (JadrenAnimationNativeBatch.IsAvailable)
                {
                    Assert.That(nativeWorker.UsesNativePoseTiles, Is.True);
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
        public void WorkerAggregatePoseTileBridgePreservesManagedCrowdTrsWhenAvailable()
        {
            var rig = CreateRig(string.Empty);
            var previousClip = CreatePoseClip(
                new Vector3(-2.0f, 1.0f, 4.0f),
                Quaternion.Euler(0.0f, 10.0f, 0.0f),
                new Vector3(1.0f, 2.0f, 3.0f));
            var currentClip = CreatePoseClip(
                new Vector3(6.0f, 5.0f, -4.0f),
                Quaternion.Euler(0.0f, 90.0f, 0.0f),
                new Vector3(3.0f, 4.0f, 5.0f));
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
                var nativeWorker = new JadrenAnimationPoseWorker(
                    rig,
                    controller,
                    preferNativeSlerp: false,
                    preferNativePoseTiles: true);
                var requests = new[]
                {
                    new JadrenAnimationPoseBatchRequest
                    {
                        CurrentState = 1,
                        PreviousState = 0,
                        FadeWeight = 0.25f,
                        Lod = JadrenAnimationLod.Full
                    },
                    new JadrenAnimationPoseBatchRequest
                    {
                        CurrentState = 1,
                        PreviousState = 0,
                        FadeWeight = 0.75f,
                        Lod = JadrenAnimationLod.Full
                    }
                };
                var managedOutputs = new[] { new JadrenPoseBuffer(), new JadrenPoseBuffer() };
                var nativeOutputs = new[] { new JadrenPoseBuffer(), new JadrenPoseBuffer() };

                managedWorker.EvaluateBatch(requests, managedOutputs, requests.Length);
                nativeWorker.EvaluateBatch(requests, nativeOutputs, requests.Length);
                for (var agent = 0; agent < requests.Length; agent++)
                {
                    Assert.That(
                        Vector3.Distance(nativeOutputs[agent].Positions[0], managedOutputs[agent].Positions[0]),
                        Is.LessThan(0.000001f));
                    Assert.That(
                        Vector3.Distance(nativeOutputs[agent].Scales[0], managedOutputs[agent].Scales[0]),
                        Is.LessThan(0.000001f));
                    Assert.That(
                        Quaternion.Angle(nativeOutputs[agent].Rotations[0], managedOutputs[agent].Rotations[0]),
                        Is.LessThan(0.0005f));
                }
                Assert.That(nativeWorker.UsesNativeCrowdPoseTiles, Is.True);
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
        public void PoseApplierPreservesSpawnRootWhenRootMotionIsDisabled()
        {
            var root = new GameObject("JadrenRootMotionGuardTest");
            root.transform.localPosition = new Vector3(7.0f, 0.0f, 11.0f);
            var rig = CreateRig(string.Empty);
            var applier = root.AddComponent<JadrenAnimationPoseApplier>();
            var pose = new JadrenPoseBuffer();
            pose.EnsureCapacity(1);
            pose.Positions[0] = new Vector3(30.80725f, 0.0f, -19.979212f);
            pose.Rotations[0] = Quaternion.identity;
            pose.Scales[0] = Vector3.one;
            applier.RebuildBindings(rig, root.transform);

            try
            {
                applier.Apply(pose, JadrenAnimationLod.Full);
                Assert.That(root.transform.localPosition, Is.EqualTo(new Vector3(7.0f, 0.0f, 11.0f)));

                applier.ApplyRootMotion = true;
                applier.Apply(pose, JadrenAnimationLod.Full);
                Assert.That(root.transform.localPosition, Is.EqualTo(pose.Positions[0]));
            }
            finally
            {
                Object.DestroyImmediate(root);
                Object.DestroyImmediate(rig);
            }
        }

        [Test]
        public void BatchPoseEvaluatorKeepsAgentStateIndependent()
        {
            var rig = CreateRig(string.Empty);
            var clip = CreateClip(
                Quaternion.identity,
                Quaternion.Euler(0.0f, 90.0f, 0.0f));
            var controller = ScriptableObject.CreateInstance<JadrenControllerAsset>();
            controller.SetBakedData(
                new[]
                {
                    new JadrenAnimationStateDefinition
                    {
                        name = "Test",
                        clip = clip,
                        playbackSpeed = 1.0f
                    }
                },
                new JadrenAnimationTransition[0]);

            try
            {
                using (var batch = new JadrenAnimationBatchPoseEvaluator(rig, controller, 2))
                {
                    Assert.That(batch.Step(0, 0.5f, 1.0f, JadrenAnimationLod.Full), Is.True);
                    Assert.That(batch.Step(1, 0.25f, 1.0f, JadrenAnimationLod.Full), Is.True);
                    Assert.That(
                        Quaternion.Angle(batch.GetPose(0).Rotations[0], Quaternion.Euler(0.0f, 45.0f, 0.0f)),
                        Is.LessThan(0.001f));
                    Assert.That(
                        Quaternion.Angle(batch.GetPose(1).Rotations[0], Quaternion.Euler(0.0f, 22.5f, 0.0f)),
                        Is.LessThan(0.001f));
                }
            }
            finally
            {
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

        private static JadrenClipAsset CreatePoseClip(
            Vector3 position,
            Quaternion rotation,
            Vector3 scale)
        {
            var clip = ScriptableObject.CreateInstance<JadrenClipAsset>();
            clip.SetBakedData(
                "test",
                1,
                2,
                1.0f,
                1.0f,
                false,
                new[] { position, position },
                new[] { rotation, rotation },
                new[] { scale, scale },
                "test-pose-clip");
            return clip;
        }

        private static string DescribePoseBits(JadrenPoseBuffer pose, int boneIndex)
        {
            var position = pose.Positions[boneIndex];
            var rotation = pose.Rotations[boneIndex];
            var scale = pose.Scales[boneIndex];
            return string.Join(
                ",",
                System.BitConverter.SingleToInt32Bits(position.x),
                System.BitConverter.SingleToInt32Bits(position.y),
                System.BitConverter.SingleToInt32Bits(position.z),
                System.BitConverter.SingleToInt32Bits(rotation.x),
                System.BitConverter.SingleToInt32Bits(rotation.y),
                System.BitConverter.SingleToInt32Bits(rotation.z),
                System.BitConverter.SingleToInt32Bits(rotation.w),
                System.BitConverter.SingleToInt32Bits(scale.x),
                System.BitConverter.SingleToInt32Bits(scale.y),
                System.BitConverter.SingleToInt32Bits(scale.z));
        }
    }
}
