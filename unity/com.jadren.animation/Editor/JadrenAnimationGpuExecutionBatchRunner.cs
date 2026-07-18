using System;
using System.IO;
using UnityEditor;
using UnityEngine;
using UnityEngine.Rendering;
using Jadren.Animation;

namespace Jadren.Animation.Editor
{
    /// <summary>
    /// Executes a small GPU quaternion batch and compares its completed
    /// readback with the managed worker math. It also exercises the explicit
    /// worker/player/applier route without measuring frame time.
    /// </summary>
    public static class JadrenAnimationGpuExecutionBatchRunner
    {
        private const string ReportFileName = "jadren-animation-gpu-execution-smoke.json";
        private const float ToleranceDegrees = 0.02f;

        [Serializable]
        private sealed class Report
        {
            public string schema;
            public string status;
            public string unity_version;
            public string platform;
            public string graphics_device;
            public bool supports_compute_shaders;
            public bool completion_verified;
            public bool gpu_execution_claim;
            public string claim_scope;
            public string shader_resource;
            public int sample_count;
            public float max_angle_error_degrees;
            public float tolerance_degrees;
            public string failure_reason;
            public string report_path;
            public bool worker_player_route_verified;
        }

        public static void Run()
        {
            var reportPath = Path.Combine(
                Directory.GetParent(Application.dataPath).FullName,
                ReportFileName);
            var capabilities = JadrenAnimationGpuCapabilities.Probe();
            var baseReport = new Report
            {
                schema = "jadren-unity-animation-gpu-execution-0.1",
                unity_version = Application.unityVersion,
                platform = Application.platform.ToString(),
                graphics_device = capabilities.GraphicsDevice.ToString(),
                supports_compute_shaders = capabilities.ComputeShadersSupported,
                completion_verified = false,
                gpu_execution_claim = false,
                claim_scope = "completed ComputeShader Slerp parity only plus explicit worker/player/applier route smoke; no FPS, frame-time or throughput claim",
                shader_resource = "JadrenAnimationSlerpUnclamped",
                sample_count = 0,
                max_angle_error_degrees = 0.0f,
                tolerance_degrees = ToleranceDegrees,
                failure_reason = string.Empty,
                report_path = reportPath,
                worker_player_route_verified = false
            };

            if (!capabilities.ComputeShadersSupported
                || capabilities.GraphicsDevice == GraphicsDeviceType.Null)
            {
                baseReport.status = "skip-no-gpu";
                baseReport.failure_reason = "compute_or_graphics_device_unavailable";
                WriteReport(reportPath, baseReport);
                Debug.Log("JADREN animation GPU execution report=" + reportPath + " status=skip-no-gpu");
                EditorApplication.Exit(0);
                return;
            }

            try
            {
                var shader = Resources.Load<ComputeShader>(baseReport.shader_resource);
                if (shader == null)
                {
                    throw new InvalidOperationException("Animation Slerp ComputeShader resource is missing.");
                }

                var kernel = shader.FindKernel("SlerpUnclamped");
                shader.GetKernelThreadGroupSizes(kernel, out var threadsX, out var threadsY, out var threadsZ);
                if (threadsX != 64 || threadsY != 1 || threadsZ != 1)
                {
                    throw new InvalidOperationException("Animation Slerp kernel workgroup contract is invalid.");
                }

                var samples = CreateSamples();
                var previous = new Quaternion[samples.Length];
                var current = new Quaternion[samples.Length];
                var weights = new float[samples.Length];
                for (var index = 0; index < samples.Length; index++)
                {
                    previous[index] = ToQuaternion(samples[index].previous);
                    current[index] = ToQuaternion(samples[index].current);
                    weights[index] = samples[index].weight;
                }

                var basePose = new JadrenPoseBuffer();
                basePose.EnsureCapacity(samples.Length);
                for (var index = 0; index < samples.Length; index++)
                {
                    basePose.Positions[index] = Vector3.zero;
                    basePose.Rotations[index] = Quaternion.identity;
                    basePose.Scales[index] = Vector3.one;
                }

                var applierObject = new GameObject("JadrenAnimationGpuPoseCoordinatorSmoke");
                try
                {
                    var applier = applierObject.AddComponent<JadrenAnimationPoseApplier>();
                    using (var coordinator = new JadrenAnimationGpuPoseCoordinator(shader))
                    {
                        if (!coordinator.TryQueue(
                                basePose,
                                previous,
                                current,
                                weights,
                                samples.Length,
                                JadrenAnimationLod.Full,
                                out var failureReason))
                        {
                            throw new InvalidOperationException(
                                "Animation Slerp GPU dispatch was rejected: " + failureReason);
                        }
                        if (coordinator.TryQueue(
                                basePose,
                                previous,
                                current,
                                weights,
                                samples.Length,
                                JadrenAnimationLod.Full,
                                out var duplicateFailure)
                            || duplicateFailure != "gpu_pose_already_pending")
                        {
                            throw new InvalidOperationException(
                                "Animation GPU coordinator accepted a second in-flight pose: "
                                + duplicateFailure);
                        }
                        if (coordinator.CompleteAndApply(applier)
                            != JadrenAnimationGpuPoseApplyStatus.Applied
                            || coordinator.LastAppliedPose == null)
                        {
                            throw new InvalidOperationException(
                                "Animation GPU coordinator did not apply the completed pose: "
                                + coordinator.LastFailureReason);
                        }

                        var maxError = 0.0f;
                        for (var index = 0; index < samples.Length; index++)
                        {
                            var actual = coordinator.LastAppliedPose.Rotations[index];
                            var expected = JadrenQuaternionMath.SlerpUnclamped(
                                ToQuaternion(samples[index].previous),
                                ToQuaternion(samples[index].current),
                                samples[index].weight);
                            var error = Quaternion.Angle(actual, expected);
                            if (float.IsNaN(error) || float.IsInfinity(error) || error > ToleranceDegrees)
                            {
                                throw new InvalidOperationException(
                                    "Animation Slerp GPU parity mismatch at sample " + index + ": " + error
                                    + " degrees, actual=(" + actual.x.ToString("R") + "," + actual.y.ToString("R") + ","
                                    + actual.z.ToString("R") + "," + actual.w.ToString("R") + "), expected=("
                                    + expected.x.ToString("R") + "," + expected.y.ToString("R") + ","
                                    + expected.z.ToString("R") + "," + expected.w.ToString("R") + ").");
                            }
                            maxError = Mathf.Max(maxError, error);
                        }

                        VerifyPlayerRoute(shader);

                        baseReport.status = "measured";
                        baseReport.completion_verified = true;
                        baseReport.gpu_execution_claim = true;
                        baseReport.worker_player_route_verified = true;
                        baseReport.sample_count = samples.Length;
                        baseReport.max_angle_error_degrees = maxError;
                    }
                }
                finally
                {
                    UnityEngine.Object.DestroyImmediate(applierObject);
                }

                WriteReport(reportPath, baseReport);
                Debug.Log(
                    "JADREN animation GPU execution report=" + reportPath
                    + " status=measured samples=" + baseReport.sample_count
                    + " max_angle_error_degrees=" + baseReport.max_angle_error_degrees.ToString("R"));
                EditorApplication.Exit(0);
            }
            catch (Exception error)
            {
                baseReport.status = "failed";
                baseReport.failure_reason = error.GetType().Name + ":" + error.Message;
                WriteReport(reportPath, baseReport);
                Debug.LogException(error);
                EditorApplication.Exit(1);
            }
        }

        private static void VerifyPlayerRoute(ComputeShader shader)
        {
            var root = new GameObject("JadrenAnimationGpuPlayerRouteSmoke");
            var bone = new GameObject("Bone");
            bone.transform.SetParent(root.transform, false);
            var rig = ScriptableObject.CreateInstance<JadrenRigAsset>();
            var previousClip = ScriptableObject.CreateInstance<JadrenClipAsset>();
            var currentClip = ScriptableObject.CreateInstance<JadrenClipAsset>();
            var controller = ScriptableObject.CreateInstance<JadrenControllerAsset>();
            try
            {
                rig.SetBakedData(
                    "gpu-player-route-smoke",
                    new[] { "Bone" },
                    new[] { "Bone" },
                    new[] { -1 },
                    new[] { Vector3.zero },
                    new[] { Quaternion.identity },
                    new[] { Vector3.one });
                previousClip.SetBakedData(
                    "gpu-player-route-previous",
                    1,
                    1,
                    30.0f,
                    0.0f,
                    false,
                    new[] { Vector3.zero },
                    new[] { Quaternion.identity },
                    new[] { Vector3.one });
                var targetRotation = Quaternion.Euler(0.0f, 90.0f, 0.0f);
                currentClip.SetBakedData(
                    "gpu-player-route-current",
                    1,
                    1,
                    30.0f,
                    0.0f,
                    false,
                    new[] { Vector3.zero },
                    new[] { targetRotation },
                    new[] { Vector3.one });
                controller.SetBakedData(
                    new[]
                    {
                        new JadrenAnimationStateDefinition
                        {
                            name = "Previous",
                            clip = previousClip,
                            speedThreshold = 0.0f,
                            playbackSpeed = 1.0f
                        },
                        new JadrenAnimationStateDefinition
                        {
                            name = "Current",
                            clip = currentClip,
                            speedThreshold = 0.5f,
                            playbackSpeed = 1.0f
                        }
                    },
                    new JadrenAnimationTransition[0]);

                var authoring = root.AddComponent<JadrenAnimationAuthoring>();
                authoring.AssignBakedAssets(rig, controller);
                var player = root.AddComponent<JadrenAnimationPlayer>();
                player.SetGpuRotationShader(shader);
                player.Step(0.0f);
                player.SetSpeed(1.0f);
                player.Step(0.075f);
                if (!player.TryQueueGpuPose())
                {
                    throw new InvalidOperationException(
                        "JadrenAnimationPlayer rejected GPU pose: "
                        + player.LastGpuPoseFailureReason);
                }
                if (player.CompleteGpuPose() != JadrenAnimationGpuPoseApplyStatus.Applied)
                {
                    throw new InvalidOperationException(
                        "JadrenAnimationPlayer did not apply GPU pose: "
                        + player.LastGpuPoseFailureReason);
                }

                var expected = JadrenQuaternionMath.SlerpUnclamped(
                    Quaternion.identity,
                    targetRotation,
                    0.5f);
                var error = Quaternion.Angle(bone.transform.localRotation, expected);
                if (float.IsNaN(error) || float.IsInfinity(error) || error > ToleranceDegrees)
                {
                    throw new InvalidOperationException(
                        "JadrenAnimationPlayer GPU route parity mismatch: " + error + " degrees.");
                }
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(root);
                UnityEngine.Object.DestroyImmediate(controller);
                UnityEngine.Object.DestroyImmediate(currentClip);
                UnityEngine.Object.DestroyImmediate(previousClip);
                UnityEngine.Object.DestroyImmediate(rig);
            }
        }

        private static Sample[] CreateSamples()
        {
            var pairs = new[]
            {
                new Pair(new Vector4(0.0f, 0.0f, 0.0f, 1.0f), new Vector4(0.0f, 0.70710677f, 0.0f, 0.70710677f)),
                new Pair(new Vector4(0.10259783f, 0.20519567f, 0.3077935f, 0.9233805f), new Vector4(-0.10259783f, -0.20519567f, -0.3077935f, -0.9233805f)),
                new Pair(new Vector4(0.239287997f, -0.099041479f, 0.369627823f, 0.892360528f), new Vector4(-0.445186762f, 0.196506397f, 0.356789851f, 0.797430239f))
            };
            var weights = new[] { -0.5f, 0.0f, 0.125f, 0.5f, 1.0f, 1.5f };
            var samples = new Sample[pairs.Length * weights.Length];
            var sampleIndex = 0;
            for (var pairIndex = 0; pairIndex < pairs.Length; pairIndex++)
            {
                for (var weightIndex = 0; weightIndex < weights.Length; weightIndex++)
                {
                    samples[sampleIndex++] = new Sample(
                        pairs[pairIndex].previous,
                        pairs[pairIndex].current,
                        weights[weightIndex]);
                }
            }
            return samples;
        }

        private static Quaternion ToQuaternion(Vector4 value)
        {
            return new Quaternion(value.x, value.y, value.z, value.w);
        }

        private static void WriteReport(string reportPath, Report report)
        {
            File.WriteAllText(reportPath, JsonUtility.ToJson(report, true));
        }

        private readonly struct Pair
        {
            public Pair(Vector4 previous, Vector4 current)
            {
                this.previous = previous;
                this.current = current;
            }

            public readonly Vector4 previous;
            public readonly Vector4 current;
        }

        private readonly struct Sample
        {
            public Sample(Vector4 previous, Vector4 current, float weight)
            {
                this.previous = previous;
                this.current = current;
                this.weight = weight;
            }

            public readonly Vector4 previous;
            public readonly Vector4 current;
            public readonly float weight;
        }
    }
}
