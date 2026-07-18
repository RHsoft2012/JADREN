using UnityEngine;

namespace Jadren.Animation.Samples
{
    /// <summary>
    /// Self-contained Play mode sample for the opt-in procedural GPU crowd
    /// renderer. It owns one reusable vertex/matrix batch and does not create
    /// an Animator or GameObject per agent.
    /// </summary>
    public sealed class JadrenGpuSkinningCrowdSample : MonoBehaviour
    {
        [SerializeField, Min(1)] private int agentCount = 100;
        [SerializeField, Min(0.1f)] private float spacing = 0.75f;
        [SerializeField] private bool animate = true;
        [SerializeField] private ComputeShader skinningShader;
        [SerializeField] private Shader crowdShader;

        private JadrenGpuSkinningCrowdRenderer rendererHost;
        private JadrenGpuSkinningVertex[] vertices;
        private Matrix4x4[] boneMatrices;
        private Mesh mesh;
        private bool built;

        public bool IsBuilt { get { return built && rendererHost != null && rendererHost.IsReady; } }
        public int ActiveAgentCount { get { return rendererHost == null ? 0 : rendererHost.AgentCount; } }
        public int DrawSubmissionCount
        {
            get { return rendererHost == null ? 0 : rendererHost.DrawSubmissionCount; }
        }

        private void OnEnable()
        {
            TryBuild(out _);
        }

        private void Update()
        {
            if (!IsBuilt || !animate)
            {
                return;
            }
            UpdateBoneMatrices(Time.time);
        }

        private void OnDestroy()
        {
            if (rendererHost != null)
            {
                rendererHost.Dispose();
            }
            DestroyMesh();
        }

        /// <summary>
        /// Builds the sample batch. Call this after changing serialized
        /// settings; the arrays and mesh are reused until the next rebuild.
        /// </summary>
        public bool TryBuild(out string failureReason)
        {
            failureReason = string.Empty;
            DestroyMesh();
            var count = Mathf.Max(1, agentCount);
            mesh = CreateSourceMesh();
            vertices = CreateVertices(count);
            boneMatrices = new Matrix4x4[count];
            UpdateBoneMatrices(0.0f);

            rendererHost = GetComponent<JadrenGpuSkinningCrowdRenderer>();
            if (rendererHost == null)
            {
                rendererHost = gameObject.AddComponent<JadrenGpuSkinningCrowdRenderer>();
            }
            if (skinningShader != null)
            {
                rendererHost.SetSkinningShader(skinningShader);
            }
            if (crowdShader != null)
            {
                rendererHost.SetCrowdShader(crowdShader);
            }

            var extent = Mathf.Max(20.0f, spacing * Mathf.Sqrt(count) * 2.0f);
            rendererHost.SetDrawBounds(new Bounds(Vector3.zero, Vector3.one * extent));
            if (!rendererHost.TrySetCrowdData(mesh, null, vertices, boneMatrices, out failureReason))
            {
                built = false;
                return false;
            }
            rendererHost.SetGpuSkinningEnabled(true);
            built = rendererHost.IsReady;
            if (!built)
            {
                failureReason = rendererHost.LastFailureReason;
            }
            return built;
        }

        private void UpdateBoneMatrices(float time)
        {
            var count = boneMatrices.Length;
            var side = Mathf.CeilToInt(Mathf.Sqrt(count));
            for (var agent = 0; agent < count; agent++)
            {
                var x = (agent % side - (side - 1) * 0.5f) * spacing;
                var y = (agent / side - (side - 1) * 0.5f) * spacing;
                var wave = animate ? Mathf.Sin(time * 1.5f + agent * 0.17f) * 0.08f : 0.0f;
                var rotation = animate
                    ? Quaternion.Euler(0.0f, 0.0f, Mathf.Sin(time + agent * 0.11f) * 8.0f)
                    : Quaternion.identity;
                boneMatrices[agent] = Matrix4x4.TRS(
                    new Vector3(x, y + wave, 0.0f),
                    rotation,
                    Vector3.one);
            }
        }

        private static JadrenGpuSkinningVertex[] CreateVertices(int count)
        {
            var vertices = new JadrenGpuSkinningVertex[count * 3];
            var weights = new Vector4(1.0f, 0.0f, 0.0f, 0.0f);
            for (var agent = 0; agent < count; agent++)
            {
                var offset = agent * 3;
                var indices = new Vector4(agent, agent, agent, agent);
                vertices[offset] = new JadrenGpuSkinningVertex(
                    new Vector3(-0.25f, -0.25f, 0.0f), weights, indices);
                vertices[offset + 1] = new JadrenGpuSkinningVertex(
                    new Vector3(0.25f, -0.25f, 0.0f), weights, indices);
                vertices[offset + 2] = new JadrenGpuSkinningVertex(
                    new Vector3(0.0f, 0.25f, 0.0f), weights, indices);
            }
            return vertices;
        }

        private static Mesh CreateSourceMesh()
        {
            var source = new Mesh { name = "JadrenGpuSkinningCrowdSampleMesh" };
            source.vertices = new[]
            {
                new Vector3(-0.25f, -0.25f, 0.0f),
                new Vector3(0.25f, -0.25f, 0.0f),
                new Vector3(0.0f, 0.25f, 0.0f)
            };
            source.normals = new[] { Vector3.forward, Vector3.forward, Vector3.forward };
            source.tangents = new[]
            {
                new Vector4(1.0f, 0.0f, 0.0f, 1.0f),
                new Vector4(1.0f, 0.0f, 0.0f, 1.0f),
                new Vector4(1.0f, 0.0f, 0.0f, 1.0f)
            };
            source.uv = new[] { Vector2.zero, Vector2.right, Vector2.up };
            source.triangles = new[] { 0, 1, 2 };
            source.bounds = new Bounds(Vector3.zero, Vector3.one * 2.0f);
            return source;
        }

        private void DestroyMesh()
        {
            if (mesh == null)
            {
                return;
            }
            if (Application.isPlaying)
            {
                Destroy(mesh);
            }
            else
            {
                DestroyImmediate(mesh);
            }
            mesh = null;
            vertices = null;
            boneMatrices = null;
            built = false;
        }
    }
}
