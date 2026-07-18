cbuffer RootConstants : register(b0) {
    uint length;
};

RWStructuredBuffer<uint> Input : register(u0);
RWStructuredBuffer<uint> Output : register(u1);

[RootSignature("RootConstants(num32BitConstants=1, b0), DescriptorTable(UAV(u0, numDescriptors=2))")]
[numthreads(64, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    if (id.x < length) {
        Output[id.x] = Input[id.x] + 1;
    }
}
