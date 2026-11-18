// The heap's shader form, `HeapForm::FixedArray` (crates/rhi-api/README.md, "The heap and the
// sampler table"): fixed-size arrays in one parameter block, the images at binding 0 and the
// samplers at binding 1 of descriptor set 1 on SPIR-V, one argument buffer on Metal. A slot is
// one number on both targets. The lengths are rhi-api's `HEAP_SLOTS_DEFAULT` and
// `SAMPLERS_DEFAULT`.
module heap;

public struct Heap
{
    public Texture2D<float4> images[1024];
    public SamplerState samplers[64];
};

[[vk::binding(0, 1)]]
public ParameterBlock<Heap> heap;

// The non-uniform marker: every index into the heap goes through it. SPIR-V carries the
// decoration only where the marker is inlined into the access, and the cook refuses an access
// that does not carry it; Slang's Metal target refuses `NonUniformResourceIndex`, so the
// marker is the index alone there.
[ForceInline]
public uint nonuniform_slot(uint slot)
{
    __target_switch
    {
    case spirv:
        return NonUniformResourceIndex(slot);
    default:
        return slot;
    }
}
