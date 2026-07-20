using System;
using System.Collections.Generic;
using System.IO;
using Jadren.Animation;
using UnityEditor;
using UnityEngine;
using Stopwatch = System.Diagnostics.Stopwatch;
using UnityDebug = UnityEngine.Debug;

namespace Jadren.Animation.Editor
{
    /// <summary>
    /// Isolated CPU microbenchmark for the managed pose blend and the current
    /// weighted AoSoA8 crowd bridge. It uses one worker batch call for all
    /// agents and verifies full numerical pose parity before reporting timing.
    /// </summary>
    public static class JadrenAnimationNativePoseTileBenchmarkBatchRunner
    {
        private const int BoneCount = 165;
        private const int WarmupCount = 5;
        private const int MeasurementCount = 20;
        private const float FadeWeight = 0.25f;
        private const float ParityTolerance = 0.000001f;
        private const string ReportEnvironmentVariable = "JADREN_POSE_TILE_REPORT";

        [Serializable]
        private sealed class WorkloadReport
        {
            public int agent_count;
            public int bone_count;
            public int warmup_count;
            public int sample_count;
            public double managed_median_ms;
            public double native_aggregate_median_ms;
            public double native_vs_managed_ratio;
            public double native_speedup_percent;
            public ulong managed_checksum;
            public ulong native_checksum;
            public bool checksum_match;
            public float max_abs_error;
        }

        [Serializable]
        private sealed class Report
        {
            public string schema;
            public string status;
            public string unity_version;
            public string platform;
            public bool native_aggregate_available;
            public int bone_count;
            public int warmup_count;
            public int measurement_count;
            public float fade_weight;
            public float parity_tolerance;
            public WorkloadReport[] workloads;
            public string failure_reason;
            public string claim_scope;
            public string report_path;
        }

        public static void Run()
        {
            var reportPath = ResolveReportPath();
            var report = new Report
            {
                schema = "jadren-unity-animation-native-pose-crowd-benchmark-0.1",
                status = "failed",
                unity_version = Application.unityVersion,
                platform = Application.platform.ToString(),
                native_aggregate_available = false,
                bone_count = BoneCount,
                warmup_count = WarmupCount,
                measurement_count = MeasurementCount,
                fade_weight = FadeWeight,
                parity_tolerance = ParityTolerance,
                workloads = Array.Empty<WorkloadReport>(),
                failure_reason = string.Empty,
                claim_scope = "local Unity Editor aggregate CPU pose-blend microbenchmark with bounded numerical parity; exact checksums are reported separately and no frame-time, rendering, Rukhanka or cross-device claim is made",
                report_path = reportPath
            };

            JadrenRigAsset rig = null;
            JadrenClipAsset previousClip = null;
            JadrenClipAsset currentClip = null;
            JadrenControllerAsset controller = null;
            try
            {
                rig = CreateRig(BoneCount);
                previousClip = CreateClip(BoneCount, 0.0f);
                currentClip = CreateClip(BoneCount, 1.0f);
                controller = ScriptableObject.CreateInstance<JadrenControllerAsset>();
                controller.SetBakedData(
                    new[]
                    {
                        new JadrenAnimationStateDefinition
                        {
                            name = "Previous",
                            clip = previousClip,
                            playbackSpeed = 1.0f
                        },
                        new JadrenAnimationStateDefinition
                        {
                            name = "Current",
                            clip = currentClip,
                            playbackSpeed = 1.0f
                        }
                    },
                    Array.Empty<JadrenAnimationTransition>());

                var managedWorker = new JadrenAnimationPoseWorker(rig, controller);
                var nativeWorker = new JadrenAnimationPoseWorker(
                    rig,
                    controller,
                    preferNativeSlerp: false,
                    preferNativePoseTiles: true);
                var workloads = new List<WorkloadReport>();
                foreach (var agentCount in new[] { 250, 500, 1000 })
                {
                    var managedOutputs = CreateOutputs(agentCount);
                    var nativeOutputs = CreateOutputs(agentCount);
                    var requests = CreateRequests(agentCount);
                    for (var warmup = 0; warmup < WarmupCount; warmup++)
                    {
                        EvaluateAgents(managedWorker, managedOutputs, requests, agentCount);
                        EvaluateAgents(nativeWorker, nativeOutputs, requests, agentCount);
                    }
                    report.native_aggregate_available = nativeWorker.UsesNativeCrowdPoseTiles;
                    if (!report.native_aggregate_available)
                    {
                        report.status = "skip-native-aggregate-unavailable";
                        report.failure_reason = "native_pose_crowd_capability_unavailable";
                        WriteReport(reportPath, report);
                        EditorApplication.Exit(0);
                        return;
                    }

                    var managedSamples = new double[MeasurementCount];
                    var nativeSamples = new double[MeasurementCount];
                    ulong managedChecksum = 0UL;
                    ulong nativeChecksum = 0UL;
                    var checksumMatch = true;
                    var maxAbsError = 0.0f;
                    for (var sample = 0; sample < MeasurementCount; sample++)
                    {
                        if ((sample & 1) == 0)
                        {
                            managedSamples[sample] = Measure(
                                managedWorker,
                                managedOutputs,
                                requests,
                                agentCount,
                                out managedChecksum);
                            nativeSamples[sample] = Measure(
                                nativeWorker,
                                nativeOutputs,
                                requests,
                                agentCount,
                                out nativeChecksum);
                        }
                        else
                        {
                            nativeSamples[sample] = Measure(
                                nativeWorker,
                                nativeOutputs,
                                requests,
                                agentCount,
                                out nativeChecksum);
                            managedSamples[sample] = Measure(
                                managedWorker,
                                managedOutputs,
                                requests,
                                agentCount,
                                out managedChecksum);
                        }

                        var sampleError = ComputeMaxAbsError(
                            managedOutputs,
                            nativeOutputs,
                            agentCount,
                            out var sampleChecksumMatch);
                        maxAbsError = Mathf.Max(maxAbsError, sampleError);
                        checksumMatch &= sampleChecksumMatch;
                        if (sampleError > ParityTolerance)
                        {
                            throw new InvalidOperationException(
                                "Pose numerical parity failed for agent_count=" + agentCount
                                + ", sample=" + sample
                                + ", max_abs_error=" + sampleError + ".");
                        }
                    }

                    Array.Sort(managedSamples);
                    Array.Sort(nativeSamples);
                    var managedMedian = managedSamples[managedSamples.Length / 2];
                    var nativeMedian = nativeSamples[nativeSamples.Length / 2];
                    workloads.Add(new WorkloadReport
                    {
                        agent_count = agentCount,
                        bone_count = BoneCount,
                        warmup_count = WarmupCount,
                        sample_count = MeasurementCount,
                        managed_median_ms = managedMedian,
                        native_aggregate_median_ms = nativeMedian,
                        native_vs_managed_ratio = managedMedian > 0.0
                            ? nativeMedian / managedMedian
                            : 0.0,
                        native_speedup_percent = managedMedian > 0.0
                            ? (managedMedian - nativeMedian) / managedMedian * 100.0
                            : 0.0,
                        managed_checksum = managedChecksum,
                        native_checksum = nativeChecksum,
                        checksum_match = checksumMatch,
                        max_abs_error = maxAbsError
                    });
                }

                report.workloads = workloads.ToArray();
                report.status = "measured";
                WriteReport(reportPath, report);
                UnityDebug.Log(
                    "JADREN native pose tile benchmark=" + reportPath
                    + " workloads=" + report.workloads.Length);
                EditorApplication.Exit(0);
            }
            catch (Exception error)
            {
                report.status = "failed";
                report.failure_reason = error.GetType().Name + ":" + error.Message;
                WriteReport(reportPath, report);
                UnityDebug.LogException(error);
                EditorApplication.Exit(1);
            }
            finally
            {
                if (controller != null) UnityEngine.Object.DestroyImmediate(controller);
                if (currentClip != null) UnityEngine.Object.DestroyImmediate(currentClip);
                if (previousClip != null) UnityEngine.Object.DestroyImmediate(previousClip);
                if (rig != null) UnityEngine.Object.DestroyImmediate(rig);
            }
        }

        private static double Measure(
            JadrenAnimationPoseWorker worker,
            JadrenPoseBuffer[] outputs,
            JadrenAnimationPoseBatchRequest[] requests,
            int agentCount,
            out ulong checksum)
        {
            var stopwatch = Stopwatch.StartNew();
            checksum = EvaluateAgents(worker, outputs, requests, agentCount);
            stopwatch.Stop();
            return stopwatch.Elapsed.TotalMilliseconds;
        }

        private static ulong EvaluateAgents(
            JadrenAnimationPoseWorker worker,
            JadrenPoseBuffer[] outputs,
            JadrenAnimationPoseBatchRequest[] requests,
            int agentCount)
        {
            var checksum = 0UL;
            worker.EvaluateBatch(requests, outputs, agentCount);
            for (var agent = 0; agent < agentCount; agent++)
            {
                checksum ^= outputs[agent].Checksum + (ulong)agent;
            }
            return checksum;
        }

        private static JadrenAnimationPoseBatchRequest[] CreateRequests(int count)
        {
            var requests = new JadrenAnimationPoseBatchRequest[count];
            for (var agent = 0; agent < count; agent++)
            {
                var currentTime = (agent % 60) / 60.0f;
                requests[agent] = new JadrenAnimationPoseBatchRequest
                {
                    CurrentState = 1,
                    CurrentTime = currentTime,
                    CurrentPreviousTime = currentTime - 1.0f / 60.0f,
                    PreviousState = 0,
                    PreviousTime = currentTime * 0.5f,
                    FadeWeight = FadeWeight,
                    Lod = JadrenAnimationLod.Full
                };
            }
            return requests;
        }

        private static JadrenPoseBuffer[] CreateOutputs(int count)
        {
            var outputs = new JadrenPoseBuffer[count];
            for (var index = 0; index < outputs.Length; index++)
            {
                outputs[index] = new JadrenPoseBuffer();
            }
            return outputs;
        }

        private static float ComputeMaxAbsError(
            JadrenPoseBuffer[] managed,
            JadrenPoseBuffer[] native,
            int agentCount,
            out bool checksumMatch)
        {
            var maxError = 0.0f;
            checksumMatch = true;
            for (var agent = 0; agent < agentCount; agent++)
            {
                checksumMatch &= managed[agent].Checksum == native[agent].Checksum;
                for (var bone = 0; bone < BoneCount; bone++)
                {
                    maxError = MaxComponentError(
                        maxError,
                        managed[agent].Positions[bone],
                        native[agent].Positions[bone]);
                    maxError = MaxComponentError(
                        maxError,
                        managed[agent].Scales[bone],
                        native[agent].Scales[bone]);
                    var managedRotation = managed[agent].Rotations[bone];
                    var nativeRotation = native[agent].Rotations[bone];
                    maxError = Mathf.Max(maxError, Mathf.Abs(managedRotation.x - nativeRotation.x));
                    maxError = Mathf.Max(maxError, Mathf.Abs(managedRotation.y - nativeRotation.y));
                    maxError = Mathf.Max(maxError, Mathf.Abs(managedRotation.z - nativeRotation.z));
                    maxError = Mathf.Max(maxError, Mathf.Abs(managedRotation.w - nativeRotation.w));
                }
            }
            return maxError;
        }

        private static float MaxComponentError(
            float current,
            Vector3 expected,
            Vector3 actual)
        {
            current = Mathf.Max(current, Mathf.Abs(expected.x - actual.x));
            current = Mathf.Max(current, Mathf.Abs(expected.y - actual.y));
            return Mathf.Max(current, Mathf.Abs(expected.z - actual.z));
        }

        private static JadrenRigAsset CreateRig(int boneCount)
        {
            var names = new string[boneCount];
            var paths = new string[boneCount];
            var parents = new int[boneCount];
            var positions = new Vector3[boneCount];
            var rotations = new Quaternion[boneCount];
            var scales = new Vector3[boneCount];
            for (var bone = 0; bone < boneCount; bone++)
            {
                names[bone] = "Bone" + bone;
                paths[bone] = bone == 0 ? string.Empty : "Bone" + bone;
                parents[bone] = bone - 1;
                positions[bone] = Vector3.zero;
                rotations[bone] = Quaternion.identity;
                scales[bone] = Vector3.one;
            }

            var rig = ScriptableObject.CreateInstance<JadrenRigAsset>();
            rig.SetBakedData(
                "native-pose-tile-benchmark",
                names,
                paths,
                parents,
                positions,
                rotations,
                scales,
                "native-pose-tile-benchmark-rig");
            return rig;
        }

        private static JadrenClipAsset CreateClip(int boneCount, float phase)
        {
            const int frameCount = 2;
            var translations = new Vector3[boneCount * frameCount];
            var rotations = new Quaternion[boneCount * frameCount];
            var scales = new Vector3[boneCount * frameCount];
            for (var frame = 0; frame < frameCount; frame++)
            {
                for (var bone = 0; bone < boneCount; bone++)
                {
                    var index = frame * boneCount + bone;
                    translations[index] = new Vector3(
                        phase + bone * 0.001f,
                        phase * 0.5f + frame * 0.01f,
                        -phase - bone * 0.0005f);
                    rotations[index] = Quaternion.Euler(
                        phase * 10.0f + bone * 0.01f,
                        phase * 45.0f + frame * 2.0f,
                        bone * 0.005f);
                    scales[index] = Vector3.one * (1.0f + phase * 0.1f + bone * 0.0001f);
                }
            }

            var clip = ScriptableObject.CreateInstance<JadrenClipAsset>();
            clip.SetBakedData(
                "native-pose-tile-benchmark",
                boneCount,
                frameCount,
                1.0f,
                1.0f,
                false,
                translations,
                rotations,
                scales,
                "native-pose-tile-benchmark-clip-" + phase);
            return clip;
        }

        private static string ResolveReportPath()
        {
            var explicitPath = Environment.GetEnvironmentVariable(ReportEnvironmentVariable);
            if (!string.IsNullOrWhiteSpace(explicitPath))
            {
                return Path.GetFullPath(explicitPath);
            }
            return Path.Combine(
                Directory.GetParent(Application.dataPath).FullName,
                "jadren-animation-native-pose-tile-benchmark.json");
        }

        private static void WriteReport(string path, Report report)
        {
            Directory.CreateDirectory(Path.GetDirectoryName(path));
            File.WriteAllText(path, JsonUtility.ToJson(report, true));
        }
    }
}
