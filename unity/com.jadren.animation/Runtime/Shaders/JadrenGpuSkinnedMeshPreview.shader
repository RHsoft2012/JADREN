Shader "Jadren/Animation/GpuSkinnedMeshPreview"
{
    Properties
    {
        _BaseColor ("Base Color", Color) = (0.2, 0.7, 1.0, 1.0)
        _MainTex ("Albedo", 2D) = "white" {}
        _BumpMap ("Normal Map", 2D) = "bump" {}
        _BumpScale ("Normal Scale", Range(0, 2)) = 1.0
        _JadrenLightDirection ("Jadren Light Direction", Vector) = (0.0, 0.0, 1.0, 0.0)
    }
    SubShader
    {
        Tags { "RenderType" = "Opaque" "Queue" = "Geometry" }
        Pass
        {
            Tags { "LightMode" = "SRPDefaultUnlit" }
            Cull Off
            ZTest Always
            ZWrite Off
            CGPROGRAM
            #pragma target 4.5
            #pragma vertex vert
            #pragma fragment frag
            #include "UnityCG.cginc"

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
            int _JadrenGpuSkinningBoneCount;
            float4 _BaseColor;
            float4 _JadrenLightDirection;
            sampler2D _MainTex;
            sampler2D _BumpMap;
            float _BumpScale;

            struct Attributes
            {
                float3 vertex : POSITION;
                float3 normal : NORMAL;
                float4 tangent : TANGENT;
                float2 uv : TEXCOORD0;
                uint vertexId : SV_VertexID;
            };

            struct Varyings
            {
                float4 position : SV_POSITION;
                float2 uv : TEXCOORD0;
                float3 worldNormal : TEXCOORD1;
                float3 worldTangent : TEXCOORD2;
                float tangentSign : TEXCOORD3;
            };

            Varyings vert(Attributes input)
            {
                Varyings output;
                float3 position = input.vertex;
                if (input.vertexId < (uint)_JadrenGpuVertexCount)
                {
                    position = _JadrenGpuPositions[input.vertexId];
                }
                output.position = UnityObjectToClipPos(float4(position, 1.0));
                output.uv = input.uv;
                float3 normal = input.normal;
                if (_JadrenGpuSkinningBoneCount > 0
                    && input.vertexId < (uint)_JadrenGpuVertexCount)
                {
                    JadrenSkinningVertex skin = _JadrenGpuSkinningVertices[input.vertexId];
                    float3 skinnedNormal = float3(0.0, 0.0, 0.0);
                    float weightSum = 0.0;
                    [unroll]
                    for (int influence = 0; influence < 4; influence++)
                    {
                        float weight = skin.weights[influence];
                        int boneIndex = (int)skin.indices[influence];
                        if (weight > 0.0 && boneIndex >= 0
                            && boneIndex < _JadrenGpuSkinningBoneCount)
                        {
                            skinnedNormal += mul(
                                (float3x3)_JadrenGpuSkinningBoneMatrices[boneIndex],
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
                if (_JadrenGpuSkinningBoneCount > 0
                    && input.vertexId < (uint)_JadrenGpuVertexCount)
                {
                    JadrenSkinningVertex skin = _JadrenGpuSkinningVertices[input.vertexId];
                    float3 skinnedTangent = float3(0.0, 0.0, 0.0);
                    float tangentWeightSum = 0.0;
                    [unroll]
                    for (int influence = 0; influence < 4; influence++)
                    {
                        float weight = skin.weights[influence];
                        int boneIndex = (int)skin.indices[influence];
                        if (weight > 0.0 && boneIndex >= 0
                            && boneIndex < _JadrenGpuSkinningBoneCount)
                        {
                            skinnedTangent += mul(
                                (float3x3)_JadrenGpuSkinningBoneMatrices[boneIndex],
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
                float diffuse = 0.25
                    + 0.75 * saturate(dot(normal, lightDirection));
                return tex2D(_MainTex, input.uv) * _BaseColor * diffuse;
            }
            ENDCG
        }
    }
}
