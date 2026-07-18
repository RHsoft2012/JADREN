using System;
using System.IO;
using UnityEditor;
using UnityEngine;
using UnityEngine.Rendering;
using Jadren.Animation;

namespace Jadren.Unity.Samples.AgentSimulation.Editor
{
    /// <summary>
    /// Runs a deterministic route-selection smoke. It validates the adapter's
    /// capability contract, not shader execution, frame time or GPU throughput.
    /// </summary>
    public static class JadrenAnimationGpuCapabilityBatchRunner
    {
        private const string ReportFileName = "jadren-animation-gpu-capability-smoke.json";

        [Serializable]
        private sealed class CaseReport
        {
            public string name;
            public string route;
            public string reason;
            public bool supported;
            public bool rejected;
        }

        [Serializable]
        private sealed class Report
        {
            public string schema;
            public string status;
            public string unity_version;
            public string platform;
            public string report_path;
            public bool gpu_execution_claim;
            public string claim_scope;
            public CaseReport[] cases;
        }

        public static void Run()
        {
            try
            {
                var allGpu = new JadrenAnimationGpuCapabilities(
                    true, true, true, true, true, GraphicsDeviceType.Direct3D11);
                var computeOnly = new JadrenAnimationGpuCapabilities(
                    true, true, true, false, true, GraphicsDeviceType.Direct3D11);

                var entities = JadrenAnimationGpuAdapter.Plan(
                    new JadrenAnimationGpuRequest(
                        JadrenAnimationGpuTarget.Auto, 1000, true, true, false), allGpu);
                var compute = JadrenAnimationGpuAdapter.Plan(
                    new JadrenAnimationGpuRequest(
                        JadrenAnimationGpuTarget.Auto, 1000, true, true, false), computeOnly);
                var boundsFallback = JadrenAnimationGpuAdapter.Plan(
                    new JadrenAnimationGpuRequest(
                        JadrenAnimationGpuTarget.Auto, 1000, false, true, true), allGpu);
                var rejectedResidency = JadrenAnimationGpuAdapter.Plan(
                    new JadrenAnimationGpuRequest(
                        JadrenAnimationGpuTarget.ComputeShader, 1000, true, false, false), allGpu);
                var liveCapabilities = JadrenAnimationGpuCapabilities.Probe();
                var live = JadrenAnimationGpuAdapter.Plan(
                    new JadrenAnimationGpuRequest(
                        JadrenAnimationGpuTarget.Auto, 1000, true, true, true), liveCapabilities);

                Require(entities.Route == JadrenAnimationGpuRoute.EntitiesGraphics && entities.IsSupported,
                    "Entities route contract failed.");
                Require(compute.Route == JadrenAnimationGpuRoute.ComputeShader && compute.IsSupported,
                    "Compute route contract failed.");
                Require(boundsFallback.UsesCpuFallback && boundsFallback.Reason == "bounds_unavailable",
                    "Bounds fallback contract failed.");
                Require(rejectedResidency.IsRejected && rejectedResidency.Reason == "buffer_not_resident",
                    "Resident-buffer rejection contract failed.");

                var reportPath = Path.Combine(
                    Directory.GetParent(Application.dataPath).FullName, ReportFileName);
                var report = new Report
                {
                    schema = "jadren-unity-animation-gpu-capability-0.1",
                    status = "capability_only",
                    unity_version = Application.unityVersion,
                    platform = Application.platform.ToString(),
                    report_path = reportPath,
                    gpu_execution_claim = false,
                    claim_scope = "route/capability contract only; no shader execution, GPU completion, frame-time or FPS claim",
                    cases = new[]
                    {
                        ToReport("entities_auto", entities),
                        ToReport("compute_auto", compute),
                        ToReport("bounds_missing_cpu_fallback", boundsFallback),
                        ToReport("resident_buffer_missing_rejected", rejectedResidency),
                        ToReport("live_probe", live)
                    }
                };
                File.WriteAllText(reportPath, JsonUtility.ToJson(report, true));
                Debug.Log("JADREN animation GPU capability report=" + reportPath);
                EditorApplication.Exit(0);
            }
            catch (Exception error)
            {
                Debug.LogException(error);
                EditorApplication.Exit(1);
            }
        }

        private static CaseReport ToReport(string name, JadrenAnimationGpuPlan plan)
        {
            return new CaseReport
            {
                name = name,
                route = plan.Route.ToString(),
                reason = plan.Reason,
                supported = plan.IsSupported,
                rejected = plan.IsRejected
            };
        }

        private static void Require(bool condition, string message)
        {
            if (!condition)
            {
                throw new InvalidOperationException(message);
            }
        }
    }
}
