Shader "Jadren/Animation/GpuCrowdSkinnedMeshPreview"
{
    Properties
    {
        _BaseColor ("Base Color", Color) = (0.2, 0.7, 1.0, 1.0)
        _MainTex ("Albedo", 2D) = "white" {}
        _BumpMap ("Normal Map", 2D) = "bump" {}
        _BumpScale ("Normal Scale", Range(0, 2)) = 1.0
        _JadrenAlbedoIntensity ("Jadren Albedo Intensity", Range(0, 1)) = 0.78
        _JadrenCull ("Jadren Cull", Float) = 0
        _JadrenZWrite ("Jadren ZWrite", Float) = 0
        _JadrenLightDirection ("Jadren Light Direction", Vector) = (0.0, 0.0, 1.0, 0.0)
    }
    SubShader
    {
        Tags { "RenderType" = "Opaque" "Queue" = "Geometry" }
        Pass
        {
            Cull [_JadrenCull]
            ZTest LEqual
            ZWrite [_JadrenZWrite]
            CGPROGRAM
            #pragma target 4.5
            #pragma vertex vert
            #pragma fragment frag
            #pragma multi_compile_instancing
            #pragma instancing_options procedural:setup
            #include "UnityCG.cginc"

            void setup() { }

            StructuredBuffer<float3> _JadrenGpuPositions;
            struct JadrenSkinningVertex
            {
                float3 position;
                float4 weights;
                float4 indices;
            };

            StructuredBuffer<JadrenSkinningVertex> _JadrenGpuSkinningVertices;
            StructuredBuffer<float4x4> _JadrenGpuSkinningBoneMatrices;
            int _JadrenGpuVertexCount;
            int _JadrenGpuSkinningInputVertexCount;
            int _JadrenGpuSkinningBoneCount;
            int _JadrenGpuCrowdVerticesPerInstance;
            int _JadrenGpuCrowdBonesPerInstance;
            int _JadrenGpuCrowdInstanceCount;
            float4 _BaseColor;
            float4 _JadrenLightDirection;
            sampler2D _MainTex;
            sampler2D _BumpMap;
            float _BumpScale;
            float _JadrenAlbedoIntensity;
            float _JadrenCull;
            float _JadrenZWrite;

            struct Attributes
            {
                float3 vertex : POSITION;
                float3 normal : NORMAL;
                float4 tangent : TANGENT;
                float4 color : COLOR;
                float2 uv : TEXCOORD0;
                uint vertexId : SV_VertexID;
                UNITY_VERTEX_INPUT_INSTANCE_ID
            };

            struct Varyings
            {
                float4 position : SV_POSITION;
                float2 uv : TEXCOORD0;
                float3 worldNormal : TEXCOORD1;
                float3 worldTangent : TEXCOORD2;
                float tangentSign : TEXCOORD3;
                float4 color : COLOR;
            };

            Varyings vert(Attributes input, uint proceduralInstanceId : SV_InstanceID)
            {
                Varyings output;
                UNITY_SETUP_INSTANCE_ID(input);
                uint globalVertex = input.vertexId;
                if (_JadrenGpuCrowdVerticesPerInstance > 0)
                {
                    globalVertex = input.vertexId
                        + proceduralInstanceId
                        * (uint)_JadrenGpuCrowdVerticesPerInstance;
                }
                float3 position = input.vertex;
                bool hasGpuVertex = globalVertex < (uint)_JadrenGpuVertexCount
                    && proceduralInstanceId < (uint)_JadrenGpuCrowdInstanceCount;
                uint sourceVertex = (_JadrenGpuCrowdVerticesPerInstance > 0
                    && _JadrenGpuSkinningInputVertexCount
                        == _JadrenGpuCrowdVerticesPerInstance)
                    ? input.vertexId
                    : globalVertex;
                if (hasGpuVertex)
                {
                    position = _JadrenGpuPositions[globalVertex];
                }
                // Skinning output already contains the world-space agent
                // origin. Do not apply the host transform a second time.
                output.position = mul(UNITY_MATRIX_VP, float4(position, 1.0));
                output.uv = input.uv;
                // Kyle's URP Lit fixture opts into _USE_VERTEX_COLOR. Keep
                // the same per-vertex albedo in the procedural preview.
                output.color = input.color;

                float3 normal = input.normal;
                if (_JadrenGpuSkinningBoneCount > 0 && hasGpuVertex)
                {
                    JadrenSkinningVertex skin = _JadrenGpuSkinningVertices[sourceVertex];
                    uint boneBase = _JadrenGpuCrowdBonesPerInstance > 0
                        ? proceduralInstanceId * (uint)_JadrenGpuCrowdBonesPerInstance
                        : 0u;
                    uint bonesPerInstance = _JadrenGpuCrowdBonesPerInstance > 0
                        ? (uint)_JadrenGpuCrowdBonesPerInstance
                        : (uint)_JadrenGpuSkinningBoneCount;
                    float3 skinnedNormal = float3(0.0, 0.0, 0.0);
                    float weightSum = 0.0;
                    [unroll]
                    for (int influence = 0; influence < 4; influence++)
                    {
                        float weight = skin.weights[influence];
                        int boneIndex = (int)skin.indices[influence];
                        if (weight > 0.0 && boneIndex >= 0
                            && boneIndex < bonesPerInstance
                            && boneBase + (uint)boneIndex < (uint)_JadrenGpuSkinningBoneCount)
                        {
                            skinnedNormal += mul(
                                (float3x3)_JadrenGpuSkinningBoneMatrices[boneBase + (uint)boneIndex],
                                input.normal) * weight;
                            weightSum += weight;
                        }
                    }
                    if (weightSum > 0.00001)
                    {
                        normal = skinnedNormal / weightSum;
                    }
                }
                if (dot(normal, normal) < 0.00001)
                {
                    normal = float3(0.0, 0.0, 1.0);
                }
                output.worldNormal = UnityObjectToWorldNormal(normalize(normal));

                float3 tangent = input.tangent.xyz;
                if (_JadrenGpuSkinningBoneCount > 0 && hasGpuVertex)
                {
                    JadrenSkinningVertex skin = _JadrenGpuSkinningVertices[sourceVertex];
                    uint boneBase = _JadrenGpuCrowdBonesPerInstance > 0
                        ? proceduralInstanceId * (uint)_JadrenGpuCrowdBonesPerInstance
                        : 0u;
                    uint bonesPerInstance = _JadrenGpuCrowdBonesPerInstance > 0
                        ? (uint)_JadrenGpuCrowdBonesPerInstance
                        : (uint)_JadrenGpuSkinningBoneCount;
                    float3 skinnedTangent = float3(0.0, 0.0, 0.0);
                    float tangentWeightSum = 0.0;
                    [unroll]
                    for (int influence = 0; influence < 4; influence++)
                    {
                        float weight = skin.weights[influence];
                        int boneIndex = (int)skin.indices[influence];
                        if (weight > 0.0 && boneIndex >= 0
                            && boneIndex < bonesPerInstance
                            && boneBase + (uint)boneIndex < (uint)_JadrenGpuSkinningBoneCount)
                        {
                            skinnedTangent += mul(
                                (float3x3)_JadrenGpuSkinningBoneMatrices[boneBase + (uint)boneIndex],
                                input.tangent.xyz) * weight;
                            tangentWeightSum += weight;
                        }
                    }
                    if (tangentWeightSum > 0.00001)
                    {
                        tangent = skinnedTangent / tangentWeightSum;
                    }
                }
                if (dot(tangent, tangent) < 0.00001)
                {
                    tangent = float3(1.0, 0.0, 0.0);
                }
                output.worldTangent = UnityObjectToWorldDir(normalize(tangent));
                output.tangentSign = input.tangent.w < 0.0 ? -1.0 : 1.0;
                return output;
            }

            fixed4 frag(Varyings input) : SV_Target
            {
                float3 lightDirection = normalize(_JadrenLightDirection.xyz);
                float3 normal = normalize(input.worldNormal);
                float3 tangent = normalize(
                    input.worldTangent - normal * dot(normal, input.worldTangent));
                float3 bitangent = normalize(cross(normal, tangent) * input.tangentSign);
                float3 tangentNormal = UnpackNormal(tex2D(_BumpMap, input.uv));
                tangentNormal.xy *= _BumpScale;
                tangentNormal = normalize(tangentNormal);
                normal = normalize(
                    tangent * tangentNormal.x
                    + bitangent * tangentNormal.y
                    + normal * tangentNormal.z);
                // Keep the preview visibly opaque even when a URP Lit source
                // material relies on scene ambient lighting that this compact
                // pass does not reproduce. The bounded range also prevents
                // bright texels from disappearing against a white diagnostic
                // background.
                float diffuse = 0.18
                    + 0.62 * saturate(dot(normal, lightDirection));
                float4 albedo = tex2D(_MainTex, input.uv)
                    * _BaseColor
                    * input.color
                    * _JadrenAlbedoIntensity;
                // This Phase-1 preview pass is opaque. The source URP Lit
                // material stores alpha=0 in _BaseColor while still writing
                // an opaque surface; never propagate that metadata into the
                // framebuffer alpha and make Play-mode captures transparent.
                return fixed4(albedo.rgb * diffuse, 1.0);
            }
            ENDCG
        }
    }
}
