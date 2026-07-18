using System;
using System.Reflection;
using UnityEngine;
using UnityEngine.Rendering;

namespace Jadren.Animation
{
    /// <summary>Requested destination for the optional animation render path.</summary>
    public enum JadrenAnimationGpuTarget : byte
    {
        Auto = 0,
        Cpu = 1,
        ComputeShader = 2,
        EntitiesGraphics = 3
    }

    /// <summary>Route selected after capability and resource validation.</summary>
    public enum JadrenAnimationGpuRoute : byte
    {
        CpuFallback = 0,
        ComputeShader = 1,
        EntitiesGraphics = 2
    }

    /// <summary>
    /// Per-dispatch prerequisites owned by the animation host. Bounds and
    /// buffer residency are deliberately explicit; the adapter never guesses
    /// them from a renderer or silently uploads a buffer.
    /// </summary>
    [Serializable]
    public struct JadrenAnimationGpuRequest
    {
        public JadrenAnimationGpuTarget target;
        public int agentCount;
        public bool boundsReady;
        public bool bufferResident;
        public bool allowCpuFallback;

        public JadrenAnimationGpuRequest(
            JadrenAnimationGpuTarget target,
            int agentCount,
            bool boundsReady,
            bool bufferResident,
            bool allowCpuFallback)
        {
            if (agentCount < 1)
            {
                throw new ArgumentOutOfRangeException(nameof(agentCount), "Agent count must be positive.");
            }

            this.target = target;
            this.agentCount = agentCount;
            this.boundsReady = boundsReady;
            this.bufferResident = bufferResident;
            this.allowCpuFallback = allowCpuFallback;
        }

        public static JadrenAnimationGpuRequest Default
        {
            get
            {
                return new JadrenAnimationGpuRequest(
                    JadrenAnimationGpuTarget.Auto,
                    1,
                    false,
                    false,
                    true);
            }
        }
    }

    /// <summary>
    /// Snapshot of platform/package capabilities. Probe is intentionally
    /// reflection-based for Entities Graphics so the animation package stays
    /// installable without the Entities package.
    /// </summary>
    public readonly struct JadrenAnimationGpuCapabilities
    {
        public JadrenAnimationGpuCapabilities(
            bool computeShadersSupported,
            bool instancingSupported,
            bool renderPipelineAvailable,
            bool entitiesGraphicsAvailable,
            bool bufferBindingSupported,
            GraphicsDeviceType graphicsDevice)
        {
            ComputeShadersSupported = computeShadersSupported;
            InstancingSupported = instancingSupported;
            RenderPipelineAvailable = renderPipelineAvailable;
            EntitiesGraphicsAvailable = entitiesGraphicsAvailable;
            BufferBindingSupported = bufferBindingSupported;
            GraphicsDevice = graphicsDevice;
        }

        public bool ComputeShadersSupported { get; }
        public bool InstancingSupported { get; }
        public bool RenderPipelineAvailable { get; }
        public bool EntitiesGraphicsAvailable { get; }
        public bool BufferBindingSupported { get; }
        public GraphicsDeviceType GraphicsDevice { get; }

        public bool HasGpuPrerequisites
        {
            get
            {
                return ComputeShadersSupported
                    && InstancingSupported
                    && RenderPipelineAvailable
                    && BufferBindingSupported;
            }
        }

        public static JadrenAnimationGpuCapabilities Probe()
        {
            var renderPipelineAvailable = GraphicsSettings.currentRenderPipeline != null
                || GraphicsSettings.defaultRenderPipeline != null;
            var computeShadersSupported = SystemInfo.supportsComputeShaders;
            var bufferBindingSupported = computeShadersSupported
                && SystemInfo.maxComputeBufferInputsVertex > 0
                && SystemInfo.maxComputeBufferInputsFragment > 0;

            return new JadrenAnimationGpuCapabilities(
                computeShadersSupported,
                SystemInfo.supportsInstancing,
                renderPipelineAvailable,
                HasType("Unity.Entities.World") && HasType("Unity.Rendering.RenderMeshArray"),
                bufferBindingSupported,
                SystemInfo.graphicsDeviceType);
        }

        private static bool HasType(string fullName)
        {
            var assemblies = AppDomain.CurrentDomain.GetAssemblies();
            for (var index = 0; index < assemblies.Length; index++)
            {
                try
                {
                    if (assemblies[index].GetType(fullName, false) != null)
                    {
                        return true;
                    }
                }
                catch (ReflectionTypeLoadException)
                {
                    // A partially loadable optional package is unavailable to
                    // this process and must not break the CPU animation path.
                }
            }
            return false;
        }
    }

    /// <summary>Auditable result of one animation GPU route selection.</summary>
    public readonly struct JadrenAnimationGpuPlan
    {
        internal JadrenAnimationGpuPlan(
            JadrenAnimationGpuRoute route,
            bool supported,
            bool rejected,
            int agentCount,
            string reason,
            JadrenAnimationGpuCapabilities capabilities)
        {
            Route = route;
            IsSupported = supported;
            IsRejected = rejected;
            AgentCount = agentCount;
            Reason = reason ?? string.Empty;
            Capabilities = capabilities;
        }

        public JadrenAnimationGpuRoute Route { get; }
        public bool IsSupported { get; }
        public bool IsRejected { get; }
        public int AgentCount { get; }
        public string Reason { get; }
        public JadrenAnimationGpuCapabilities Capabilities { get; }
        public bool IsGpu => Route != JadrenAnimationGpuRoute.CpuFallback && IsSupported;
        public bool UsesCpuFallback => Route == JadrenAnimationGpuRoute.CpuFallback;
    }

    /// <summary>
    /// Capability-only adapter for the future compute/Entities animation
    /// renderer. It selects a route but does not dispatch a shader or mutate
    /// Entities state; those operations remain separate, testable gates.
    /// </summary>
    public static class JadrenAnimationGpuAdapter
    {
        public static JadrenAnimationGpuPlan Plan(JadrenAnimationGpuRequest request)
        {
            return Plan(request, JadrenAnimationGpuCapabilities.Probe());
        }

        public static JadrenAnimationGpuPlan Plan(
            JadrenAnimationGpuRequest request,
            JadrenAnimationGpuCapabilities capabilities)
        {
            if (request.agentCount < 1)
            {
                return Reject(request, capabilities, "agent_count_invalid");
            }
            if (request.target == JadrenAnimationGpuTarget.Cpu)
            {
                return Cpu(request, capabilities, "cpu_explicit");
            }
            if (!request.boundsReady)
            {
                return FallbackOrReject(request, capabilities, "bounds_unavailable");
            }
            if (!request.bufferResident)
            {
                return FallbackOrReject(request, capabilities, "buffer_not_resident");
            }

            if (request.target == JadrenAnimationGpuTarget.EntitiesGraphics)
            {
                return TryEntities(request, capabilities);
            }
            if (request.target == JadrenAnimationGpuTarget.ComputeShader)
            {
                return TryCompute(request, capabilities);
            }

            var entities = TryEntities(request, capabilities);
            if (entities.IsSupported)
            {
                return entities;
            }
            var compute = TryCompute(request, capabilities);
            if (compute.IsSupported)
            {
                return compute;
            }
            return FallbackOrReject(request, capabilities, "no_gpu_route_available");
        }

        private static JadrenAnimationGpuPlan TryEntities(
            JadrenAnimationGpuRequest request,
            JadrenAnimationGpuCapabilities capabilities)
        {
            if (!capabilities.EntitiesGraphicsAvailable)
            {
                return FallbackOrReject(request, capabilities, "entities_graphics_unavailable");
            }
            if (!capabilities.RenderPipelineAvailable)
            {
                return FallbackOrReject(request, capabilities, "render_pipeline_unavailable");
            }
            if (!capabilities.InstancingSupported)
            {
                return FallbackOrReject(request, capabilities, "instancing_unavailable");
            }
            if (!capabilities.BufferBindingSupported)
            {
                return FallbackOrReject(request, capabilities, "buffer_binding_unavailable");
            }
            return new JadrenAnimationGpuPlan(
                JadrenAnimationGpuRoute.EntitiesGraphics,
                true,
                false,
                request.agentCount,
                "entities_graphics_capabilities_passed",
                capabilities);
        }

        private static JadrenAnimationGpuPlan TryCompute(
            JadrenAnimationGpuRequest request,
            JadrenAnimationGpuCapabilities capabilities)
        {
            if (!capabilities.ComputeShadersSupported)
            {
                return FallbackOrReject(request, capabilities, "compute_shader_unavailable");
            }
            if (!capabilities.RenderPipelineAvailable)
            {
                return FallbackOrReject(request, capabilities, "render_pipeline_unavailable");
            }
            if (!capabilities.InstancingSupported)
            {
                return FallbackOrReject(request, capabilities, "instancing_unavailable");
            }
            if (!capabilities.BufferBindingSupported)
            {
                return FallbackOrReject(request, capabilities, "buffer_binding_unavailable");
            }
            return new JadrenAnimationGpuPlan(
                JadrenAnimationGpuRoute.ComputeShader,
                true,
                false,
                request.agentCount,
                "compute_shader_capabilities_passed",
                capabilities);
        }

        private static JadrenAnimationGpuPlan FallbackOrReject(
            JadrenAnimationGpuRequest request,
            JadrenAnimationGpuCapabilities capabilities,
            string reason)
        {
            return request.allowCpuFallback
                ? Cpu(request, capabilities, reason)
                : Reject(request, capabilities, reason);
        }

        private static JadrenAnimationGpuPlan Cpu(
            JadrenAnimationGpuRequest request,
            JadrenAnimationGpuCapabilities capabilities,
            string reason)
        {
            return new JadrenAnimationGpuPlan(
                JadrenAnimationGpuRoute.CpuFallback,
                true,
                false,
                request.agentCount,
                reason,
                capabilities);
        }

        private static JadrenAnimationGpuPlan Reject(
            JadrenAnimationGpuRequest request,
            JadrenAnimationGpuCapabilities capabilities,
            string reason)
        {
            return new JadrenAnimationGpuPlan(
                JadrenAnimationGpuRoute.CpuFallback,
                false,
                true,
                request.agentCount,
                reason,
                capabilities);
        }
    }
}
