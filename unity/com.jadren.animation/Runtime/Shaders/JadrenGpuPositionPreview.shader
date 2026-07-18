Shader "Jadren/Animation/GpuPositionPreview"
{
    Properties
    {
        _BaseColor ("Base Color", Color) = (0.2, 0.7, 1.0, 1.0)
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
            int _JadrenGpuVertexCount;
            float4 _BaseColor;

            struct Attributes
            {
                float3 vertex : POSITION;
                uint vertexId : SV_VertexID;
            };

            struct Varyings
            {
                float4 position : SV_POSITION;
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
                return output;
            }

            fixed4 frag(Varyings input) : SV_Target
            {
                return _BaseColor;
            }
            ENDCG
        }
    }
}
