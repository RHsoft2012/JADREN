#define JADREN_REGION_RUNTIME_HAS_STACK_SUPPORT 0
#define JADREN_REGION_RUNTIME_HAS_BOUNDS_SUPPORT 0
#include <stdint.h>
#include <stddef.h>

typedef void *HANDLE;
typedef unsigned long DWORD;
typedef unsigned long long SIZE_T;
typedef unsigned int UINT;
extern HANDLE GetProcessHeap(void);
extern void *HeapAlloc(HANDLE heap, DWORD flags, SIZE_T bytes);
extern void *HeapReAlloc(HANDLE heap, DWORD flags, void *memory, SIZE_T bytes);
extern int HeapFree(HANDLE heap, DWORD flags, void *memory);
extern void ExitProcess(UINT result);
#if JADREN_REGION_RUNTIME_HAS_STACK_SUPPORT
int _fltused = 0;
#endif

static void jadren_copy_bytes(void *destination, const void *source, size_t count) {
    size_t index;
    unsigned char *out = (unsigned char *)destination;
    const unsigned char *in = (const unsigned char *)source;
    for (index = 0U; index < count; index += 1U) {
        out[index] = in[index];
    }
}

static void jadren_move_bytes(void *destination, const void *source, size_t count) {
    size_t index;
    unsigned char *out = (unsigned char *)destination;
    const unsigned char *in = (const unsigned char *)source;
    uintptr_t out_address = (uintptr_t)out;
    uintptr_t in_address = (uintptr_t)in;
    if (out_address <= in_address || out_address - in_address >= count) {
        jadren_copy_bytes(destination, source, count);
        return;
    }
    for (index = count; index != 0U; index -= 1U) {
        out[index - 1U] = in[index - 1U];
    }
}

static void jadren_zero_bytes(void *destination, size_t count) {
    size_t index;
    unsigned char *out = (unsigned char *)destination;
    for (index = 0U; index < count; index += 1U) {
        out[index] = 0U;
    }
}

#if JADREN_REGION_RUNTIME_HAS_STACK_SUPPORT
__asm__(
    ".text\n"
    ".globl __chkstk\n"
    ".p2align 4, 0x90\n"
    "__chkstk:\n"
    "  pushq %rcx\n"
    "  pushq %rax\n"
    "  cmpq $4096, %rax\n"
    "  leaq 24(%rsp), %rcx\n"
    "  jb .Ljadren_region_chkstk_done\n"
    ".Ljadren_region_chkstk_loop:\n"
    "  subq $4096, %rcx\n"
    "  testq %rax, (%rcx)\n"
    "  subq $4096, %rax\n"
    "  cmpq $4096, %rax\n"
    "  jae .Ljadren_region_chkstk_loop\n"
    ".Ljadren_region_chkstk_done:\n"
    "  subq %rax, %rcx\n"
    "  testq %rax, (%rcx)\n"
    "  popq %rax\n"
    "  popq %rcx\n"
    "  retq\n");
#endif

#if JADREN_REGION_RUNTIME_HAS_BOUNDS_SUPPORT
void jadren_rt_bounds_panic_u64(unsigned __int64 index, unsigned __int64 length) {
    (void)index;
    (void)length;
    ExitProcess(1U);
}
#endif

typedef struct JadrenRegionAllocation {
    void *raw;
    struct JadrenRegionAllocation *next;
} JadrenRegionAllocation;

typedef struct JadrenRegion {
    JadrenRegionAllocation *allocations;
} JadrenRegion;

static void *jadren_region_alloc_raw(SIZE_T size) {
    return HeapAlloc(GetProcessHeap(), 0, size);
}

static void jadren_region_free_raw(void *pointer) {
    if (pointer != 0) {
        (void)HeapFree(GetProcessHeap(), 0, pointer);
    }
}

void *jadren_rt_native_region_create(void) {
    JadrenRegion *region = (JadrenRegion *)jadren_region_alloc_raw(sizeof(JadrenRegion));
    if (region == 0) {
        return 0;
    }
    region->allocations = 0;
    return region;
}

void *jadren_rt_native_region_allocate(void *handle, uint64_t element_size,
                                       uint64_t count, uint64_t alignment) {
    JadrenRegion *region = (JadrenRegion *)handle;
    uint64_t payload;
    uint64_t total;
    uintptr_t base;
    uintptr_t aligned;
    void *raw;
    void *user;
    JadrenRegionAllocation *entry;
    uint64_t index;
    if (region == 0 || element_size == 0 || count == 0 || alignment == 0 ||
        (alignment & (alignment - 1)) != 0 || count > UINT64_MAX / element_size) {
        return 0;
    }
    payload = element_size * count;
    if (payload > UINT64_MAX - alignment - (uint64_t)sizeof(void *)) {
        return 0;
    }
    total = payload + alignment + (uint64_t)sizeof(void *);
    raw = jadren_region_alloc_raw((SIZE_T)total);
    if (raw == 0) {
        return 0;
    }
    base = (uintptr_t)raw + sizeof(void *);
    aligned = (base + (uintptr_t)alignment - 1U) & ~((uintptr_t)alignment - 1U);
    user = (void *)aligned;
    for (index = 0; index < payload; index += 1U) {
        ((unsigned char *)user)[index] = 0U;
    }
    entry = (JadrenRegionAllocation *)jadren_region_alloc_raw(sizeof(JadrenRegionAllocation));
    if (entry == 0) {
        jadren_region_free_raw(raw);
        return 0;
    }
    entry->raw = raw;
    entry->next = region->allocations;
    region->allocations = entry;
    return user;
}

typedef struct JadrenBufferDescriptor {
    void *data;
    uint64_t length;
    uint64_t capacity;
} JadrenBufferDescriptor;

typedef struct JadrenSliceDescriptor {
    void *data;
    uint64_t length;
} JadrenSliceDescriptor;

typedef struct JadrenCarrierDropBranch {
    uint64_t payload_variant;
    uint64_t depth;
    uint64_t leaf_element_size;
    uint64_t leaf_alignment;
} JadrenCarrierDropBranch;

typedef struct JadrenCarrierDropField {
    uint64_t payload_variant;
    uint64_t payload_offset;
    uint64_t depth;
    uint64_t leaf_element_size;
    uint64_t leaf_alignment;
} JadrenCarrierDropField;

#define JADREN_RECORD_FIELD_OWNED_STRING UINT64_C(4294967295)
#define JADREN_ENUM_FIELD_MULTI_TAG_MARKER (UINT64_C(3) << 62)
#define JADREN_ENUM_FIELD_MULTI_TAG_MASK ((UINT64_C(1) << 20) - 1U)
#define JADREN_RECORD_FIELD_NAMED_ENUM_TAG_MARKER (UINT64_C(1) << 62)
#define JADREN_RECORD_FIELD_NAMED_ENUM_TAG_DISTANCE_MASK ((UINT64_C(1) << 30) - 1U)
#define JADREN_ENUM_FIELD_PATH_MARKER_BASE (UINT64_C(0xe0) << 56)
#define JADREN_ENUM_FIELD_PATH_MAX_TAGS 7U
extern int32_t jadren_rt_buffer_destroy(
    void *descriptor, uint64_t element_size, uint64_t alignment);
extern int32_t jadren_rt_buffer_destroy_nested_buffer_recursive(
    void *descriptor, uint64_t element_size, uint64_t alignment,
    uint64_t depth, uint64_t leaf_element_size, uint64_t leaf_alignment);
int32_t jadren_rt_carrier_destroy_record_fields(
    void *record, uint64_t element_size,
    const JadrenCarrierDropField *fields, uint64_t field_count);
static int32_t jadren_destroy_record_field(
    unsigned char *record, const JadrenCarrierDropField *field) {
    if (field->depth == JADREN_RECORD_FIELD_OWNED_STRING) {
        JadrenBufferDescriptor *string = (JadrenBufferDescriptor *)
            (record + field->payload_offset);
        if ((string->data == 0 && string->capacity != 0U) ||
            string->length > string->capacity) {
            return -41;
        }
        if (string->data != 0) {
#if defined(_WIN32)
            HANDLE heap = GetProcessHeap();
            if (heap == 0 || !HeapFree(heap, 0, string->data)) {
                return -10;
            }
#else
            free(string->data);
#endif
        }
        string->data = 0;
        string->length = 0U;
        string->capacity = 0U;
        return 0;
    }
    {
        JadrenBufferDescriptor *nested = (JadrenBufferDescriptor *)
            (record + field->payload_offset);
        if (field->depth == 1U) {
            return jadren_rt_buffer_destroy(
                nested, field->leaf_element_size, field->leaf_alignment);
        }
        return jadren_rt_buffer_destroy_nested_buffer_recursive(
            nested, (uint64_t)sizeof(JadrenBufferDescriptor), 8U,
            field->depth - 1U, field->leaf_element_size,
            field->leaf_alignment);
    }
}

uint64_t buffer_length(const void *descriptor) {
    const JadrenBufferDescriptor *buffer = (const JadrenBufferDescriptor *)descriptor;
    return buffer == 0 ? 0U : buffer->length;
}

uint64_t buffer_capacity(const void *descriptor) {
    const JadrenBufferDescriptor *buffer = (const JadrenBufferDescriptor *)descriptor;
    return buffer == 0 ? 0U : buffer->capacity;
}

JadrenSliceDescriptor buffer_slice(const void *descriptor, uint64_t start,
                                   uint64_t count, uint64_t element_size,
                                   uint64_t alignment) {
    const JadrenBufferDescriptor *buffer = (const JadrenBufferDescriptor *)descriptor;
    JadrenSliceDescriptor result;
    uintptr_t offset;
    uintptr_t pointer;
    result.data = 0;
    result.length = 0U;
    if (buffer == 0 || element_size == 0U || alignment == 0U ||
        (alignment & (alignment - 1U)) != 0U ||
        (buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity || start > buffer->length ||
        count > buffer->length - start || count == 0U ||
        start > UINT64_MAX / element_size) {
        return result;
    }
    offset = (uintptr_t)(start * element_size);
    pointer = (uintptr_t)buffer->data;
    if (pointer > UINTPTR_MAX - offset ||
        ((pointer + offset) & (uintptr_t)(alignment - 1U)) != 0U) {
        return result;
    }
    result.data = (void *)(pointer + offset);
    result.length = count;
    return result;
}

JadrenSliceDescriptor buffer_slice_write(const void *descriptor, uint64_t start,
                                         uint64_t count, uint64_t element_size,
                                         uint64_t alignment) {
    return buffer_slice(descriptor, start, count, element_size, alignment);
}

int32_t buffer_resize_status(void *descriptor, uint64_t new_length) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    if (buffer == 0) {
        return -15;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    if (new_length > buffer->capacity) {
        return -20;
    }
    buffer->length = new_length;
    return 0;
}

int32_t buffer_resize(void *descriptor, uint64_t new_length) {
    return buffer_resize_status(descriptor, new_length) == 0;
}

int32_t buffer_clear_status(void *descriptor) {
    return buffer_resize_status(descriptor, 0U);
}

int32_t buffer_clear(void *descriptor) {
    return buffer_clear_status(descriptor) == 0;
}

int32_t buffer_append_i32(void *descriptor, int32_t value) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    if (buffer == 0 || buffer->data == 0 || buffer->length >= buffer->capacity) {
        return 0;
    }
    ((int32_t *)buffer->data)[buffer->length] = value;
    buffer->length += 1U;
    return 1;
}

typedef union JadrenBufferPayload {
    JadrenBufferDescriptor value;
    int32_t error;
    unsigned char bytes[24];
} JadrenBufferPayload;

typedef struct JadrenBufferResult {
    unsigned int tag;
    JadrenBufferPayload payload;
} JadrenBufferResult;

static JadrenBufferResult buffer_error_i32(int32_t status) {
    JadrenBufferResult result;
    unsigned int index;
    result.tag = 0U;
    for (index = 0U; index < sizeof(result.payload.bytes); index += 1U) {
        result.payload.bytes[index] = 0U;
    }
    result.payload.error = status;
    return result;
}

static JadrenBufferResult buffer_ok_i32(JadrenBufferDescriptor value) {
    JadrenBufferResult result;
    result.tag = 1U;
    result.payload.value = value;
    return result;
}

JadrenBufferResult buffer_create_i32(uint64_t capacity) {
    JadrenBufferDescriptor value;
    uint64_t bytes;
    uint64_t index;
    value.data = 0;
    value.length = 0U;
    value.capacity = 0U;
    if (capacity > UINT64_MAX / (uint64_t)sizeof(int32_t)) {
        return buffer_error_i32(-13);
    }
    if (capacity == 0U) {
        return buffer_ok_i32(value);
    }
    bytes = capacity * (uint64_t)sizeof(int32_t);
    value.data = HeapAlloc(GetProcessHeap(), 0, (SIZE_T)bytes);
    if (value.data == 0) {
        return buffer_error_i32(-14);
    }
    for (index = 0U; index < bytes; index += 1U) {
        ((unsigned char *)value.data)[index] = 0U;
    }
    value.capacity = capacity;
    return buffer_ok_i32(value);
}

/* Generic owning Buffer<T> ABI.  The compiler appends element size and
 * alignment constants after the source-level capacity argument. */
JadrenBufferResult buffer_create(uint64_t capacity, uint64_t element_size,
                                 uint64_t alignment) {
    JadrenBufferDescriptor value;
    uint64_t bytes;
    uint64_t index;
    value.data = 0;
    value.length = 0U;
    value.capacity = 0U;
    if (element_size == 0U) {
        return buffer_error_i32(-11);
    }
    if (alignment == 0U || (alignment & (alignment - 1U)) != 0U) {
        return buffer_error_i32(-12);
    }
    if (capacity > UINT64_MAX / element_size) {
        return buffer_error_i32(-13);
    }
    if (capacity == 0U) {
        return buffer_ok_i32(value);
    }
    bytes = capacity * element_size;
    value.data = HeapAlloc(GetProcessHeap(), 0, (SIZE_T)bytes);
    if (value.data == 0) {
        return buffer_error_i32(-14);
    }
    for (index = 0U; index < bytes; index += 1U) {
        ((unsigned char *)value.data)[index] = 0U;
    }
    value.capacity = capacity;
    return buffer_ok_i32(value);
}

int32_t buffer_reserve_status(void *descriptor, uint64_t minimum_capacity,
                              uint64_t element_size, uint64_t alignment);

/* Insert and move an owning nested Buffer descriptor into an outer buffer. */
int32_t buffer_insert_move_status(void *descriptor, uint64_t index,
                                  const void *value, uint64_t element_size,
                                  uint64_t alignment, uint64_t nested_element_size,
                                  uint64_t nested_alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    JadrenBufferDescriptor *incoming = (JadrenBufferDescriptor *)value;
    JadrenBufferDescriptor *base;
    uint64_t current_index;
    int32_t status;
    if (buffer == 0 || value == 0) {
        return -15;
    }
    if (element_size != (uint64_t)sizeof(JadrenBufferDescriptor)) {
        return -11;
    }
    if (alignment == 0U || (alignment & (alignment - 1U)) != 0U) {
        return -12;
    }
    if (nested_element_size == 0U) {
        return -11;
    }
    if (nested_alignment == 0U || (nested_alignment & (nested_alignment - 1U)) != 0U) {
        return -12;
    }
    if (index > buffer->length) {
        return -20;
    }
    if (buffer->length == UINT64_MAX) {
        return -13;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    status = buffer_reserve_status(buffer, buffer->length + 1U, element_size, alignment);
    if (status != 0) {
        return status;
    }
    base = (JadrenBufferDescriptor *)buffer->data;
    for (current_index = buffer->length; current_index > index; current_index -= 1U) {
        base[current_index] = base[current_index - 1U];
        base[current_index - 1U].data = 0;
        base[current_index - 1U].length = 0U;
        base[current_index - 1U].capacity = 0U;
    }
    base[index] = *incoming;
    incoming->data = 0;
    incoming->length = 0U;
    incoming->capacity = 0U;
    buffer->length += 1U;
    return 0;
}

int32_t buffer_insert_move(void *descriptor, uint64_t index, const void *value,
                           uint64_t element_size, uint64_t alignment,
                           uint64_t nested_element_size, uint64_t nested_alignment) {
    return buffer_insert_move_status(descriptor, index, value, element_size,
                                      alignment, nested_element_size,
                                      nested_alignment) == 0;
}

/* Move any caller-owned element into an owning buffer without aggregate ABI. */
int32_t buffer_insert_move_from_status(void *descriptor, uint64_t index,
                                       void *source, uint64_t element_size,
                                       uint64_t alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    unsigned char *base;
    uint64_t tail_count;
    uint64_t move_bytes;
    int32_t status;
    if (buffer == 0 || source == 0) {
        return -15;
    }
    if (element_size == 0U) {
        return -11;
    }
    if (alignment == 0U || (alignment & (alignment - 1U)) != 0U ||
        ((uintptr_t)source % alignment) != 0U) {
        return -12;
    }
    if (index > buffer->length) {
        return -20;
    }
    if (buffer->length == UINT64_MAX) {
        return -13;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    tail_count = buffer->length - index;
    if (tail_count > UINT64_MAX / element_size) {
        return -13;
    }
    move_bytes = tail_count * element_size;
    status = buffer_reserve_status(buffer, buffer->length + 1U,
                                   element_size, alignment);
    if (status != 0) {
        return status;
    }
    base = (unsigned char *)buffer->data;
    if (move_bytes != 0U) {
        jadren_move_bytes(base + (index + 1U) * element_size,
                          base + index * element_size,
                          (size_t)move_bytes);
    }
    jadren_move_bytes(base + index * element_size, source,
                      (size_t)element_size);
    jadren_zero_bytes(source, (size_t)element_size);
    buffer->length += 1U;
    return 0;
}

int32_t buffer_insert_move_from(void *descriptor, uint64_t index, void *source,
                                uint64_t element_size, uint64_t alignment) {
    return buffer_insert_move_from_status(descriptor, index, source,
                                           element_size, alignment) == 0;
}

/* Move a caller-owned element to the end of an owning buffer. */
int32_t buffer_append_move_status(void *descriptor, void *source,
                                  uint64_t element_size, uint64_t alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    if (buffer == 0) {
        return -21;
    }
    return buffer_insert_move_from_status(descriptor, buffer->length, source,
                                           element_size, alignment);
}

int32_t buffer_append_move(void *descriptor, void *source,
                           uint64_t element_size, uint64_t alignment) {
    return buffer_append_move_status(descriptor, source, element_size, alignment) == 0;
}

/* Move the last initialized nested Buffer descriptor out of an outer buffer. */
JadrenBufferResult buffer_pop(void *descriptor, uint64_t element_size,
                              uint64_t alignment, uint64_t nested_element_size,
                              uint64_t nested_alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    JadrenBufferDescriptor *nested;
    JadrenBufferDescriptor value;
    if (buffer == 0) {
        return buffer_error_i32(-15);
    }
    if (element_size != (uint64_t)sizeof(JadrenBufferDescriptor)) {
        return buffer_error_i32(-11);
    }
    if (alignment == 0U || (alignment & (alignment - 1U)) != 0U) {
        return buffer_error_i32(-12);
    }
    if (nested_element_size == 0U) {
        return buffer_error_i32(-11);
    }
    if (nested_alignment == 0U || (nested_alignment & (nested_alignment - 1U)) != 0U) {
        return buffer_error_i32(-12);
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return buffer_error_i32(-21);
    }
    if (buffer->length == 0U) {
        return buffer_error_i32(-20);
    }
    nested = (JadrenBufferDescriptor *)((unsigned char *)buffer->data +
                                        (buffer->length - 1U) * element_size);
    value = *nested;
    nested->data = 0;
    nested->length = 0U;
    nested->capacity = 0U;
    buffer->length -= 1U;
    return buffer_ok_i32(value);
}

/* Move an arbitrary nested Buffer descriptor out and close the gap. */
JadrenBufferResult buffer_remove_move(void *descriptor, uint64_t index,
                                      uint64_t element_size, uint64_t alignment,
                                      uint64_t nested_element_size,
                                      uint64_t nested_alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    JadrenBufferDescriptor *base;
    JadrenBufferDescriptor value;
    uint64_t current_index;
    if (buffer == 0) {
        return buffer_error_i32(-15);
    }
    if (element_size != (uint64_t)sizeof(JadrenBufferDescriptor)) {
        return buffer_error_i32(-11);
    }
    if (alignment == 0U || (alignment & (alignment - 1U)) != 0U) {
        return buffer_error_i32(-12);
    }
    if (nested_element_size == 0U) {
        return buffer_error_i32(-11);
    }
    if (nested_alignment == 0U || (nested_alignment & (nested_alignment - 1U)) != 0U) {
        return buffer_error_i32(-12);
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return buffer_error_i32(-21);
    }
    if (index >= buffer->length) {
        return buffer_error_i32(-20);
    }
    base = (JadrenBufferDescriptor *)buffer->data;
    value = base[index];
    for (current_index = index; current_index + 1U < buffer->length; current_index += 1U) {
        base[current_index] = base[current_index + 1U];
        base[current_index + 1U].data = 0;
        base[current_index + 1U].length = 0U;
        base[current_index + 1U].capacity = 0U;
    }
    buffer->length -= 1U;
    return buffer_ok_i32(value);
}

/* Move one owning record into caller-owned output storage and close the gap. */
int32_t buffer_remove_move_into_status(void *descriptor, uint64_t index,
                                       void *output, uint64_t element_size,
                                       uint64_t alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    unsigned char *base;
    uint64_t trailing_count;
    uint64_t move_bytes;
    if (buffer == 0 || output == 0) {
        return -15;
    }
    if (element_size == 0U) {
        return -11;
    }
    if (alignment == 0U || (alignment & (alignment - 1U)) != 0U) {
        return -12;
    }
    if (((uintptr_t)output % alignment) != 0U) {
        return -12;
    }
    if (buffer->capacity > UINT64_MAX / element_size) {
        return -13;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    if (index >= buffer->length) {
        return -20;
    }
    base = (unsigned char *)buffer->data;
    jadren_move_bytes(output, base + index * element_size, (size_t)element_size);
    trailing_count = buffer->length - index - 1U;
    if (trailing_count != 0U) {
        if (trailing_count > UINT64_MAX / element_size) {
            return -13;
        }
        move_bytes = trailing_count * element_size;
        jadren_move_bytes(base + index * element_size,
                          base + (index + 1U) * element_size,
                          (size_t)move_bytes);
    }
    jadren_zero_bytes(base + (buffer->length - 1U) * element_size,
                      (size_t)element_size);
    buffer->length -= 1U;
    return 0;
}

int32_t buffer_remove_move_into(void *descriptor, uint64_t index,
                                void *output, uint64_t element_size,
                                uint64_t alignment) {
    return buffer_remove_move_into_status(descriptor, index, output,
                                           element_size, alignment) == 0;
}

/* Move the last initialized element into caller-owned output storage. */
int32_t buffer_pop_move_into_status(void *descriptor, void *output,
                                    uint64_t element_size, uint64_t alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    if (buffer == 0) {
        return -15;
    }
    if (buffer->length == 0U) {
        return -20;
    }
    return buffer_remove_move_into_status(descriptor, buffer->length - 1U,
                                           output, element_size, alignment);
}

int32_t buffer_pop_move_into(void *descriptor, void *output,
                             uint64_t element_size, uint64_t alignment) {
    return buffer_pop_move_into_status(descriptor, output, element_size,
                                       alignment) == 0;
}

int32_t buffer_reserve_i32(void *descriptor, uint64_t minimum_capacity) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    uint64_t grown;
    uint64_t bytes;
    void *resized;
    if (buffer == 0 || (buffer->data == 0 && buffer->capacity != 0) ||
        buffer->length > buffer->capacity) {
        return 0;
    }
    if (minimum_capacity <= buffer->capacity) {
        return 1;
    }
    grown = buffer->capacity == 0U ? 1U : buffer->capacity;
    while (grown < minimum_capacity) {
        if (grown > UINT64_MAX / 2U) {
            grown = minimum_capacity;
            break;
        }
        grown *= 2U;
    }
    if (grown > UINT64_MAX / (uint64_t)sizeof(int32_t)) {
        return 0;
    }
    bytes = grown * (uint64_t)sizeof(int32_t);
    resized = buffer->data == 0
        ? HeapAlloc(GetProcessHeap(), 0, (SIZE_T)bytes)
        : HeapReAlloc(GetProcessHeap(), 0, buffer->data, (SIZE_T)bytes);
    if (resized == 0) {
        return 0;
    }
    buffer->data = resized;
    buffer->capacity = grown;
    return 1;
}

int32_t buffer_reserve_status(void *descriptor, uint64_t minimum_capacity,
                              uint64_t element_size, uint64_t alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    uint64_t grown;
    uint64_t bytes;
    void *resized;
    if (buffer == 0) {
        return -15;
    }
    if (element_size == 0U) {
        return -11;
    }
    if (alignment == 0U || (alignment & (alignment - 1U)) != 0U) {
        return -12;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    if (minimum_capacity <= buffer->capacity) {
        return 0;
    }
    grown = buffer->capacity == 0U ? 1U : buffer->capacity;
    while (grown < minimum_capacity) {
        if (grown > UINT64_MAX / 2U) {
            grown = minimum_capacity;
            break;
        }
        grown *= 2U;
    }
    if (grown > UINT64_MAX / element_size) {
        return -13;
    }
    bytes = grown * element_size;
    resized = buffer->data == 0
        ? HeapAlloc(GetProcessHeap(), 0, (SIZE_T)bytes)
        : HeapReAlloc(GetProcessHeap(), 0, buffer->data, (SIZE_T)bytes);
    if (resized == 0) {
        return -14;
    }
    buffer->data = resized;
    buffer->capacity = grown;
    return 0;
}

int32_t buffer_reserve(void *descriptor, uint64_t minimum_capacity,
                       uint64_t element_size, uint64_t alignment) {
    return buffer_reserve_status(descriptor, minimum_capacity, element_size, alignment) == 0;
}

int32_t buffer_append_status(void *descriptor, const void *value, uint64_t element_size,
                             uint64_t alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    unsigned char *destination;
    int32_t status;
    if (buffer == 0 || value == 0) {
        return -15;
    }
    if (element_size == 0U) {
        return -11;
    }
    if (alignment == 0U || (alignment & (alignment - 1U)) != 0U) {
        return -12;
    }
    if (buffer->length == UINT64_MAX) {
        return -13;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    status = buffer_reserve_status(buffer, buffer->length + 1U, element_size, alignment);
    if (status != 0) {
        return status;
    }
    destination = (unsigned char *)buffer->data + buffer->length * element_size;
    jadren_copy_bytes(destination, value, (size_t)element_size);
    buffer->length += 1U;
    return 0;
}

int32_t buffer_append(void *descriptor, const void *value, uint64_t element_size,
                      uint64_t alignment) {
    return buffer_append_status(descriptor, value, element_size, alignment) == 0;
}

int32_t buffer_insert_status(void *descriptor, uint64_t index, const void *value,
                             uint64_t element_size, uint64_t alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    unsigned char *base;
    uint64_t tail_bytes;
    int32_t status;
    if (buffer == 0 || value == 0) {
        return -15;
    }
    if (element_size == 0U) {
        return -11;
    }
    if (alignment == 0U || (alignment & (alignment - 1U)) != 0U) {
        return -12;
    }
    if (index > buffer->length) {
        return -20;
    }
    if (buffer->length == UINT64_MAX) {
        return -13;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    status = buffer_reserve_status(buffer, buffer->length + 1U, element_size, alignment);
    if (status != 0) {
        return status;
    }
    base = (unsigned char *)buffer->data;
    tail_bytes = (buffer->length - index) * element_size;
    if (tail_bytes != 0U) {
        jadren_move_bytes(base + (index + 1U) * element_size,
                          base + index * element_size, (size_t)tail_bytes);
    }
    jadren_copy_bytes(base + index * element_size, value, (size_t)element_size);
    buffer->length += 1U;
    return 0;
}

int32_t buffer_insert(void *descriptor, uint64_t index, const void *value,
                      uint64_t element_size, uint64_t alignment) {
    return buffer_insert_status(descriptor, index, value, element_size, alignment) == 0;
}

int32_t buffer_remove_status(void *descriptor, uint64_t index, uint64_t element_size,
                             uint64_t alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    unsigned char *base;
    uint64_t tail_bytes;
    if (buffer == 0) {
        return -15;
    }
    if (element_size == 0U) {
        return -11;
    }
    if (alignment == 0U || (alignment & (alignment - 1U)) != 0U) {
        return -12;
    }
    if (buffer->data == 0 || buffer->length > buffer->capacity) {
        return -21;
    }
    if (index >= buffer->length) {
        return -20;
    }
    base = (unsigned char *)buffer->data;
    tail_bytes = (buffer->length - index - 1U) * element_size;
    if (tail_bytes != 0U) {
        jadren_move_bytes(base + index * element_size,
                          base + (index + 1U) * element_size, (size_t)tail_bytes);
    }
    buffer->length -= 1U;
    return 0;
}

int32_t buffer_remove(void *descriptor, uint64_t index, uint64_t element_size,
                      uint64_t alignment) {
    return buffer_remove_status(descriptor, index, element_size, alignment) == 0;
}

/* Explicitly discard one copy-safe element and close the gap. */
int32_t buffer_remove_drop_status(void *descriptor, uint64_t index,
                                  uint64_t element_size, uint64_t alignment) {
    return buffer_remove_status(descriptor, index, element_size, alignment);
}

int32_t buffer_remove_drop(void *descriptor, uint64_t index,
                           uint64_t element_size, uint64_t alignment) {
    return buffer_remove_drop_status(descriptor, index, element_size, alignment) == 0;
}

int32_t buffer_append_i32_grow(void *descriptor, int32_t value) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    if (buffer == 0 || buffer->length == UINT64_MAX ||
        !buffer_reserve_i32(buffer, buffer->length + 1U)) {
        return 0;
    }
    ((int32_t *)buffer->data)[buffer->length] = value;
    buffer->length += 1U;
    return 1;
}

int32_t jadren_rt_buffer_destroy(void *descriptor, uint64_t element_size,
                                 uint64_t alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    (void)element_size;
    (void)alignment;
    if (buffer == 0) {
        return -15;
    }
    if ((buffer->data == 0 && buffer->capacity != 0) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    if (buffer->data != 0 && !HeapFree(GetProcessHeap(), 0, buffer->data)) {
        return -10;
    }
    buffer->data = 0;
    buffer->length = 0U;
    buffer->capacity = 0U;
    return 0;
}

/* Destroy a Buffer<OwnedString>; the descriptor layout is shared with Buffer,
 * so this memory runtime does not need the file/string runtime object. */
int32_t jadren_rt_buffer_destroy_owned_string(void *descriptor,
                                              uint64_t element_size,
                                              uint64_t alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    uint64_t index;
    if (buffer == 0 || element_size != (uint64_t)sizeof(JadrenBufferDescriptor) ||
        alignment == 0U || (alignment & (alignment - 1U)) != 0U) {
        return -15;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    for (index = 0U; index < buffer->length; index += 1U) {
        JadrenBufferDescriptor *item = (JadrenBufferDescriptor *)
            ((unsigned char *)buffer->data + index * element_size);
        if (item->data != 0 && !HeapFree(GetProcessHeap(), 0, item->data)) {
            return -10;
        }
        item->data = 0;
        item->length = 0U;
        item->capacity = 0U;
    }
    if (buffer->data != 0 && !HeapFree(GetProcessHeap(), 0, buffer->data)) {
        return -10;
    }
    buffer->data = 0;
    buffer->length = 0U;
    buffer->capacity = 0U;
    return 0;
}

static int32_t jadren_owned_string_descriptor_destroy(
    JadrenBufferDescriptor *value) {
    if (value == 0) {
        return -15;
    }
    if (value->data != 0 && !HeapFree(GetProcessHeap(), 0, value->data)) {
        return -10;
    }
    value->data = 0;
    value->length = 0U;
    value->capacity = 0U;
    return 0;
}

int32_t jadren_rt_buffer_resize_move_owned_string_status(
    void *descriptor, uint64_t new_length, uint64_t element_size,
    uint64_t alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    uint64_t old_length;
    uint64_t index;
    int32_t status;
    if (buffer == 0 || element_size != (uint64_t)sizeof(JadrenBufferDescriptor) ||
        alignment != 8U) {
        return -15;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    old_length = buffer->length;
    if (new_length > old_length) {
        status = buffer_reserve_status(buffer, new_length, element_size, alignment);
        if (status != 0) {
            return status;
        }
        if (new_length - old_length != 0U) {
            jadren_zero_bytes(
                (unsigned char *)buffer->data + old_length * element_size,
                (size_t)((new_length - old_length) * element_size));
        }
    } else if (new_length < old_length) {
        for (index = new_length; index < old_length; index += 1U) {
            status = jadren_owned_string_descriptor_destroy(
                (JadrenBufferDescriptor *)((unsigned char *)buffer->data +
                                           index * element_size));
            if (status != 0) {
                return status;
            }
        }
    }
    buffer->length = new_length;
    return 0;
}

int32_t jadren_rt_buffer_resize_move_owned_string(
    void *descriptor, uint64_t new_length, uint64_t element_size,
    uint64_t alignment) {
    return jadren_rt_buffer_resize_move_owned_string_status(
        descriptor, new_length, element_size, alignment) == 0;
}

int32_t jadren_rt_buffer_clear_move_owned_string_status(
    void *descriptor, uint64_t element_size, uint64_t alignment) {
    return jadren_rt_buffer_resize_move_owned_string_status(
        descriptor, 0U, element_size, alignment);
}

int32_t jadren_rt_buffer_clear_move_owned_string(
    void *descriptor, uint64_t element_size, uint64_t alignment) {
    return jadren_rt_buffer_clear_move_owned_string_status(
        descriptor, element_size, alignment) == 0;
}

int32_t jadren_rt_buffer_remove_drop_owned_string_status(
    void *descriptor, uint64_t index, uint64_t element_size,
    uint64_t alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    unsigned char *base;
    uint64_t current_index;
    int32_t status;
    if (buffer == 0 || element_size != (uint64_t)sizeof(JadrenBufferDescriptor) ||
        alignment != 8U) {
        return -15;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    if (index >= buffer->length) {
        return -20;
    }
    base = (unsigned char *)buffer->data;
    status = jadren_owned_string_descriptor_destroy(
        (JadrenBufferDescriptor *)(base + index * element_size));
    if (status != 0) {
        return status;
    }
    for (current_index = index; current_index + 1U < buffer->length;
         current_index += 1U) {
        jadren_move_bytes(base + current_index * element_size,
                          base + (current_index + 1U) * element_size,
                          (size_t)element_size);
        jadren_zero_bytes(base + (current_index + 1U) * element_size,
                          (size_t)element_size);
    }
    buffer->length -= 1U;
    return 0;
}

int32_t jadren_rt_buffer_remove_drop_owned_string(
    void *descriptor, uint64_t index, uint64_t element_size,
    uint64_t alignment) {
    return jadren_rt_buffer_remove_drop_owned_string_status(
        descriptor, index, element_size, alignment) == 0;
}

int32_t jadren_rt_buffer_destroy_nested_buffer(void *descriptor,
                                               uint64_t element_size,
                                               uint64_t alignment,
                                               uint64_t nested_element_size,
                                               uint64_t nested_alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    uint64_t index;
    int32_t status;
    if (buffer == 0 || element_size != (uint64_t)sizeof(JadrenBufferDescriptor) ||
        alignment == 0U || (alignment & (alignment - 1U)) != 0U) {
        return -15;
    }
    if (nested_element_size == 0U || nested_alignment == 0U ||
        (nested_alignment & (nested_alignment - 1U)) != 0U) {
        return -11;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    for (index = 0U; index < buffer->length; index += 1U) {
        JadrenBufferDescriptor *nested = (JadrenBufferDescriptor *)
            ((unsigned char *)buffer->data + index * element_size);
        status = jadren_rt_buffer_destroy(nested, nested_element_size, nested_alignment);
        if (status != 0) {
            return status;
        }
    }
    if (buffer->data != 0 && !HeapFree(GetProcessHeap(), 0, buffer->data)) {
        return -10;
    }
    buffer->data = 0;
    buffer->length = 0U;
    buffer->capacity = 0U;
    return 0;
}

int32_t jadren_rt_buffer_destroy_nested_buffer_recursive(
    void *descriptor, uint64_t element_size, uint64_t alignment,
    uint64_t depth, uint64_t leaf_element_size, uint64_t leaf_alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    uint64_t index;
    int32_t status;
    if (buffer == 0 || depth == 0U ||
        element_size != (uint64_t)sizeof(JadrenBufferDescriptor) ||
        alignment == 0U || (alignment & (alignment - 1U)) != 0U) {
        return -15;
    }
    if (leaf_element_size == 0U || leaf_alignment == 0U ||
        (leaf_alignment & (leaf_alignment - 1U)) != 0U) {
        return -11;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    for (index = 0U; index < buffer->length; index += 1U) {
        JadrenBufferDescriptor *nested = (JadrenBufferDescriptor *)
            ((unsigned char *)buffer->data + index * element_size);
        if (depth == 1U) {
            status = jadren_rt_buffer_destroy(nested, leaf_element_size, leaf_alignment);
        } else {
            status = jadren_rt_buffer_destroy_nested_buffer_recursive(
                nested, (uint64_t)sizeof(JadrenBufferDescriptor),
                alignment, depth - 1U,
                leaf_element_size, leaf_alignment);
        }
        if (status != 0) {
            return status;
        }
    }
    if (buffer->data != 0 && !HeapFree(GetProcessHeap(), 0, buffer->data)) {
        return -10;
    }
    buffer->data = 0;
    buffer->length = 0U;
    buffer->capacity = 0U;
    return 0;
}

int32_t jadren_rt_buffer_destroy_nested_owned_string(
    void *descriptor, uint64_t element_size, uint64_t alignment,
    uint64_t depth, uint64_t string_element_size,
    uint64_t string_alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    uint64_t index;
    int32_t status;
    if (buffer == 0 || depth == 0U ||
        element_size != (uint64_t)sizeof(JadrenBufferDescriptor) ||
        alignment == 0U || (alignment & (alignment - 1U)) != 0U ||
        string_element_size != (uint64_t)sizeof(JadrenBufferDescriptor) ||
        string_alignment != 8U) {
        return -15;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    for (index = 0U; index < buffer->length; index += 1U) {
        JadrenBufferDescriptor *nested = (JadrenBufferDescriptor *)
            ((unsigned char *)buffer->data + index * element_size);
        if (depth == 1U) {
            status = jadren_rt_buffer_destroy_owned_string(
                nested, string_element_size, string_alignment);
        } else {
            status = jadren_rt_buffer_destroy_nested_owned_string(
                nested, (uint64_t)sizeof(JadrenBufferDescriptor),
                alignment, depth - 1U,
                string_element_size, string_alignment);
        }
        if (status != 0) {
            return status;
        }
    }
    return jadren_rt_buffer_destroy(buffer, element_size, alignment);
}

int32_t jadren_rt_buffer_remove_drop_nested_owned_string_status(
    void *descriptor, uint64_t index, uint64_t element_size,
    uint64_t alignment, uint64_t depth, uint64_t string_element_size,
    uint64_t string_alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    JadrenBufferDescriptor *base;
    uint64_t current;
    int32_t status;
    if (buffer == 0 || depth == 0U ||
        element_size != (uint64_t)sizeof(JadrenBufferDescriptor) ||
        alignment != 8U ||
        string_element_size != (uint64_t)sizeof(JadrenBufferDescriptor) ||
        string_alignment != 8U) {
        return -15;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity ||
        buffer->capacity > UINT64_MAX / element_size) {
        return -21;
    }
    if (index >= buffer->length) {
        return -20;
    }
    base = (JadrenBufferDescriptor *)buffer->data;
    if (depth == 1U) {
        status = jadren_rt_buffer_destroy_owned_string(
            &base[index], string_element_size, string_alignment);
    } else {
        status = jadren_rt_buffer_destroy_nested_owned_string(
            &base[index], element_size, alignment, depth - 1U,
            string_element_size, string_alignment);
    }
    if (status != 0) {
        return status;
    }
    for (current = index; current + 1U < buffer->length; current += 1U) {
        base[current] = base[current + 1U];
        base[current + 1U].data = 0;
        base[current + 1U].length = 0U;
        base[current + 1U].capacity = 0U;
    }
    buffer->length -= 1U;
    return 0;
}

int32_t jadren_rt_buffer_remove_drop_nested_owned_string(
    void *descriptor, uint64_t index, uint64_t element_size,
    uint64_t alignment, uint64_t depth, uint64_t string_element_size,
    uint64_t string_alignment) {
    return jadren_rt_buffer_remove_drop_nested_owned_string_status(
        descriptor, index, element_size, alignment, depth,
        string_element_size, string_alignment) == 0;
}

int32_t jadren_rt_buffer_resize_move_nested_owned_string_status(
    void *descriptor, uint64_t new_length, uint64_t element_size,
    uint64_t alignment, uint64_t depth, uint64_t string_element_size,
    uint64_t string_alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    uint64_t old_length;
    uint64_t index;
    int32_t status;
    if (buffer == 0 || depth == 0U ||
        element_size != (uint64_t)sizeof(JadrenBufferDescriptor) ||
        alignment == 0U || (alignment & (alignment - 1U)) != 0U ||
        string_element_size != (uint64_t)sizeof(JadrenBufferDescriptor) ||
        string_alignment != 8U) {
        return -15;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    old_length = buffer->length;
    if (new_length > old_length) {
        status = buffer_reserve_status(buffer, new_length, element_size, alignment);
        if (status != 0) {
            return status;
        }
        jadren_zero_bytes(
            (unsigned char *)buffer->data + old_length * element_size,
            (size_t)((new_length - old_length) * element_size));
    } else if (new_length < old_length) {
        for (index = new_length; index < old_length; index += 1U) {
            JadrenBufferDescriptor *nested = (JadrenBufferDescriptor *)
                ((unsigned char *)buffer->data + index * element_size);
            if (depth == 1U) {
                status = jadren_rt_buffer_destroy_owned_string(
                    nested, string_element_size, string_alignment);
            } else {
                status = jadren_rt_buffer_destroy_nested_owned_string(
                    nested, (uint64_t)sizeof(JadrenBufferDescriptor),
                    alignment, depth - 1U,
                    string_element_size, string_alignment);
            }
            if (status != 0) {
                return status;
            }
        }
    }
    buffer->length = new_length;
    return 0;
}

int32_t jadren_rt_buffer_resize_move_nested_owned_string(
    void *descriptor, uint64_t new_length, uint64_t element_size,
    uint64_t alignment, uint64_t depth, uint64_t string_element_size,
    uint64_t string_alignment) {
    return jadren_rt_buffer_resize_move_nested_owned_string_status(
        descriptor, new_length, element_size, alignment, depth,
        string_element_size, string_alignment) == 0;
}

int32_t jadren_rt_buffer_clear_move_nested_owned_string_status(
    void *descriptor, uint64_t element_size, uint64_t alignment,
    uint64_t depth, uint64_t string_element_size, uint64_t string_alignment) {
    return jadren_rt_buffer_resize_move_nested_owned_string_status(
        descriptor, 0U, element_size, alignment, depth,
        string_element_size, string_alignment);
}

int32_t jadren_rt_buffer_clear_move_nested_owned_string(
    void *descriptor, uint64_t element_size, uint64_t alignment,
    uint64_t depth, uint64_t string_element_size, uint64_t string_alignment) {
    return jadren_rt_buffer_clear_move_nested_owned_string_status(
        descriptor, element_size, alignment, depth,
        string_element_size, string_alignment) == 0;
}

int32_t buffer_resize_move_status(void *descriptor, uint64_t new_length,
                                  uint64_t element_size, uint64_t alignment,
                                  uint64_t nested_depth,
                                  uint64_t leaf_element_size,
                                  uint64_t leaf_alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    uint64_t old_length;
    uint64_t index;
    uint64_t byte_index;
    int32_t status;
    if (buffer == 0 || element_size != (uint64_t)sizeof(JadrenBufferDescriptor) ||
        alignment == 0U || (alignment & (alignment - 1U)) != 0U ||
        nested_depth == 0U || leaf_element_size == 0U ||
        leaf_alignment == 0U || (leaf_alignment & (leaf_alignment - 1U)) != 0U) {
        return -15;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity ||
        buffer->capacity > UINT64_MAX / element_size) {
        return -21;
    }
    old_length = buffer->length;
    if (new_length > old_length) {
        status = buffer_reserve_status(buffer, new_length, element_size, alignment);
        if (status != 0) {
            return status;
        }
        for (byte_index = old_length * element_size;
             byte_index < new_length * element_size; byte_index += 1U) {
            ((unsigned char *)buffer->data)[byte_index] = 0U;
        }
    } else if (new_length < old_length) {
        for (index = new_length; index < old_length; index += 1U) {
            JadrenBufferDescriptor *nested = (JadrenBufferDescriptor *)
                ((unsigned char *)buffer->data + index * element_size);
            if (nested_depth == 1U) {
                status = jadren_rt_buffer_destroy(nested, leaf_element_size,
                                                  leaf_alignment);
            } else {
                status = jadren_rt_buffer_destroy_nested_buffer_recursive(
                    nested, (uint64_t)sizeof(JadrenBufferDescriptor), alignment,
                    nested_depth - 1U, leaf_element_size, leaf_alignment);
            }
            if (status != 0) {
                return status;
            }
        }
    }
    buffer->length = new_length;
    return 0;
}

int32_t buffer_resize_move(void *descriptor, uint64_t new_length,
                           uint64_t element_size, uint64_t alignment,
                           uint64_t nested_depth, uint64_t leaf_element_size,
                           uint64_t leaf_alignment) {
    return buffer_resize_move_status(descriptor, new_length, element_size,
                                     alignment, nested_depth, leaf_element_size,
                                     leaf_alignment) == 0;
}

int32_t buffer_clear_move_status(void *descriptor, uint64_t element_size,
                                 uint64_t alignment, uint64_t nested_depth,
                                 uint64_t leaf_element_size,
                                 uint64_t leaf_alignment) {
    return buffer_resize_move_status(descriptor, 0U, element_size, alignment,
                                     nested_depth, leaf_element_size,
                                     leaf_alignment);
}

int32_t buffer_clear_move(void *descriptor, uint64_t element_size,
                          uint64_t alignment, uint64_t nested_depth,
                          uint64_t leaf_element_size,
                          uint64_t leaf_alignment) {
    return buffer_clear_move_status(descriptor, element_size, alignment,
                                    nested_depth, leaf_element_size,
                                    leaf_alignment) == 0;
}

int32_t jadren_rt_buffer_destroy_carrier_buffer(
    void *descriptor, uint64_t element_size, uint64_t alignment,
    uint64_t payload_offset, uint64_t payload_variant, uint64_t depth,
    uint64_t leaf_element_size, uint64_t leaf_alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    uint64_t index;
    int32_t status;
    if (buffer == 0 || depth == 0U ||
        element_size == 0U || alignment == 0U ||
        (alignment & (alignment - 1U)) != 0U ||
        payload_offset % alignment != 0U ||
        payload_offset > element_size ||
        element_size - payload_offset < (uint64_t)sizeof(JadrenBufferDescriptor) ||
        leaf_element_size == 0U || leaf_alignment == 0U ||
        (leaf_alignment & (leaf_alignment - 1U)) != 0U) {
        return -15;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    for (index = 0U; index < buffer->length; index += 1U) {
        unsigned char *carrier = (unsigned char *)buffer->data +
            index * element_size;
        uint32_t tag = (uint32_t)carrier[0] |
            ((uint32_t)carrier[1] << 8) |
            ((uint32_t)carrier[2] << 16) |
            ((uint32_t)carrier[3] << 24);
        if ((uint64_t)tag == payload_variant) {
            JadrenBufferDescriptor *nested = (JadrenBufferDescriptor *)
                (carrier + payload_offset);
            if (depth == 1U) {
                status = jadren_rt_buffer_destroy(nested, leaf_element_size,
                                                  leaf_alignment);
            } else {
                status = jadren_rt_buffer_destroy_nested_buffer_recursive(
                    nested, (uint64_t)sizeof(JadrenBufferDescriptor), alignment,
                    depth - 1U, leaf_element_size, leaf_alignment);
            }
            if (status != 0) {
                return status;
            }
        }
    }
    if (buffer->data != 0 && !HeapFree(GetProcessHeap(), 0, buffer->data)) {
        return -10;
    }
    buffer->data = 0;
    buffer->length = 0U;
    buffer->capacity = 0U;
    return 0;
}

int32_t jadren_rt_carrier_destroy_buffer(
    void *carrier, uint64_t element_size, uint64_t payload_offset,
    uint64_t payload_variant, uint64_t depth, uint64_t leaf_element_size,
    uint64_t leaf_alignment) {
    unsigned char *bytes = (unsigned char *)carrier;
    uint32_t tag;
    int32_t status;
    uint64_t index;
    if (bytes == 0 || depth == 0U ||
        element_size == 0U || payload_offset % 8U != 0U ||
        payload_offset > element_size ||
        element_size - payload_offset < (uint64_t)sizeof(JadrenBufferDescriptor) ||
        leaf_element_size == 0U || leaf_alignment == 0U ||
        (leaf_alignment & (leaf_alignment - 1U)) != 0U) {
        return -15;
    }
    tag = (uint32_t)bytes[0] |
        ((uint32_t)bytes[1] << 8) |
        ((uint32_t)bytes[2] << 16) |
        ((uint32_t)bytes[3] << 24);
    if ((uint64_t)tag == payload_variant) {
        JadrenBufferDescriptor *nested = (JadrenBufferDescriptor *)
            (bytes + payload_offset);
        if (depth == 1U) {
            status = jadren_rt_buffer_destroy(nested, leaf_element_size,
                                              leaf_alignment);
        } else {
            status = jadren_rt_buffer_destroy_nested_buffer_recursive(
                nested, (uint64_t)sizeof(JadrenBufferDescriptor), 8U,
                depth - 1U, leaf_element_size, leaf_alignment);
        }
        if (status != 0) {
            return status;
        }
    }
    for (index = 0U; index < element_size; index += 1U) {
        bytes[index] = 0U;
    }
    return 0;
}

int32_t jadren_rt_buffer_destroy_multi_carrier_buffer(
    void *descriptor, uint64_t element_size, uint64_t alignment,
    uint64_t payload_offset, uint64_t first_variant, uint64_t first_depth,
    uint64_t first_leaf_element_size, uint64_t first_leaf_alignment,
    uint64_t second_variant, uint64_t second_depth,
    uint64_t second_leaf_element_size, uint64_t second_leaf_alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    uint64_t index;
    int32_t status;
    if (buffer == 0 || first_depth == 0U || second_depth == 0U ||
        first_variant > 1U || second_variant > 1U ||
        first_variant == second_variant || element_size == 0U ||
        alignment == 0U || (alignment & (alignment - 1U)) != 0U ||
        payload_offset % alignment != 0U || payload_offset > element_size ||
        element_size - payload_offset < (uint64_t)sizeof(JadrenBufferDescriptor) ||
        first_leaf_element_size == 0U || first_leaf_alignment == 0U ||
        (first_leaf_alignment & (first_leaf_alignment - 1U)) != 0U ||
        second_leaf_element_size == 0U || second_leaf_alignment == 0U ||
        (second_leaf_alignment & (second_leaf_alignment - 1U)) != 0U) {
        return -15;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    for (index = 0U; index < buffer->length; index += 1U) {
        unsigned char *carrier = (unsigned char *)buffer->data +
            index * element_size;
        uint32_t tag = (uint32_t)carrier[0] |
            ((uint32_t)carrier[1] << 8) |
            ((uint32_t)carrier[2] << 16) |
            ((uint32_t)carrier[3] << 24);
        uint64_t depth;
        uint64_t leaf_element_size;
        uint64_t leaf_alignment;
        if ((uint64_t)tag == first_variant) {
            depth = first_depth;
            leaf_element_size = first_leaf_element_size;
            leaf_alignment = first_leaf_alignment;
        } else if ((uint64_t)tag == second_variant) {
            depth = second_depth;
            leaf_element_size = second_leaf_element_size;
            leaf_alignment = second_leaf_alignment;
        } else {
            return -21;
        }
        {
            JadrenBufferDescriptor *nested = (JadrenBufferDescriptor *)
                (carrier + payload_offset);
            if (depth == 1U) {
                status = jadren_rt_buffer_destroy(nested, leaf_element_size,
                                                  leaf_alignment);
            } else {
                status = jadren_rt_buffer_destroy_nested_buffer_recursive(
                    nested, (uint64_t)sizeof(JadrenBufferDescriptor), alignment,
                    depth - 1U, leaf_element_size, leaf_alignment);
            }
            if (status != 0) {
                return status;
            }
        }
    }
    if (buffer->data != 0 && !HeapFree(GetProcessHeap(), 0, buffer->data)) {
        return -10;
    }
    buffer->data = 0;
    buffer->length = 0U;
    buffer->capacity = 0U;
    return 0;
}

static int32_t jadren_validate_enum_carrier_branches(
    const JadrenCarrierDropBranch *branches, uint64_t branch_count) {
    uint64_t index;
    uint64_t previous;
    if (branches == 0 || branch_count == 0U || branch_count > 1024U) {
        return -11;
    }
    for (index = 0U; index < branch_count; index += 1U) {
        if (branches[index].depth == 0U ||
            branches[index].leaf_element_size == 0U ||
            branches[index].leaf_alignment == 0U ||
            (branches[index].leaf_alignment &
             (branches[index].leaf_alignment - 1U)) != 0U) {
            return -11;
        }
        for (previous = 0U; previous < index; previous += 1U) {
            if (branches[previous].payload_variant ==
                branches[index].payload_variant) {
                return -21;
            }
        }
    }
    return 0;
}

static int32_t jadren_validate_enum_carrier_fields(
    const JadrenCarrierDropField *fields, uint64_t field_count,
    uint64_t element_size);

static int32_t jadren_enum_carrier_field_is_active(
    const unsigned char *carrier, const JadrenCarrierDropField *field,
    int *active) {
    uint32_t tag;
    if (carrier == 0 || field == 0 || active == 0) {
        return -15;
    }
    if ((field->payload_variant & JADREN_RECORD_FIELD_NAMED_ENUM_TAG_MARKER) != 0U &&
        (field->payload_variant & JADREN_ENUM_FIELD_MULTI_TAG_MARKER) !=
            JADREN_ENUM_FIELD_MULTI_TAG_MARKER) {
        uint64_t distance_words =
            (field->payload_variant >> 32) &
            JADREN_RECORD_FIELD_NAMED_ENUM_TAG_DISTANCE_MASK;
        uint64_t distance_bytes;
        if (distance_words == 0U || distance_words > UINT64_MAX / 8U) {
            return -15;
        }
        distance_bytes = distance_words * 8U;
        if (distance_bytes > field->payload_offset) {
            return -15;
        }
        tag = (uint32_t)carrier[field->payload_offset - distance_bytes] |
            ((uint32_t)carrier[field->payload_offset - distance_bytes + 1U] << 8) |
            ((uint32_t)carrier[field->payload_offset - distance_bytes + 2U] << 16) |
            ((uint32_t)carrier[field->payload_offset - distance_bytes + 3U] << 24);
        *active = ((uint64_t)tag ==
            (field->payload_variant & UINT64_C(0xffffffff)));
        return 0;
    }
    {
        uint64_t path_byte = field->payload_variant >> 56;
        if ((path_byte & UINT64_C(0xf0)) ==
            (JADREN_ENUM_FIELD_PATH_MARKER_BASE >> 56)) {
            uint64_t tag_count = path_byte & UINT64_C(0x0f);
            uint64_t index;
            if (tag_count < 2U || tag_count > JADREN_ENUM_FIELD_PATH_MAX_TAGS ||
                field->payload_offset < tag_count * 8U) {
                return -15;
            }
            for (index = 0U; index < tag_count; index += 1U) {
                uint64_t shift = 8U * (tag_count - index - 1U);
                uint64_t expected = (field->payload_variant >> shift) & UINT64_C(255);
                uint32_t actual =
                    (uint32_t)carrier[field->payload_offset - 8U * (tag_count - index)] |
                    ((uint32_t)carrier[field->payload_offset - 8U * (tag_count - index) + 1U] << 8) |
                    ((uint32_t)carrier[field->payload_offset - 8U * (tag_count - index) + 2U] << 16) |
                    ((uint32_t)carrier[field->payload_offset - 8U * (tag_count - index) + 3U] << 24);
                if ((uint64_t)actual != expected) {
                    *active = 0;
                    return 0;
                }
            }
            *active = 1;
            return 0;
        }
    }
    if ((field->payload_variant & JADREN_ENUM_FIELD_MULTI_TAG_MARKER) ==
            JADREN_ENUM_FIELD_MULTI_TAG_MARKER) {
        uint32_t outer_tag;
        uint32_t first_tag;
        uint32_t second_tag;
        uint64_t outer_variant;
        uint64_t first_variant;
        uint64_t second_variant;
        if (field->payload_offset < 24U ||
            (field->payload_variant & UINT64_C(1)) != 0U) {
            return -15;
        }
        outer_tag = (uint32_t)carrier[0] |
            ((uint32_t)carrier[1] << 8) |
            ((uint32_t)carrier[2] << 16) |
            ((uint32_t)carrier[3] << 24);
        first_tag = (uint32_t)carrier[field->payload_offset - 16U] |
            ((uint32_t)carrier[field->payload_offset - 15U] << 8) |
            ((uint32_t)carrier[field->payload_offset - 14U] << 16) |
            ((uint32_t)carrier[field->payload_offset - 13U] << 24);
        second_tag = (uint32_t)carrier[field->payload_offset - 8U] |
            ((uint32_t)carrier[field->payload_offset - 7U] << 8) |
            ((uint32_t)carrier[field->payload_offset - 6U] << 16) |
            ((uint32_t)carrier[field->payload_offset - 5U] << 24);
        outer_variant = (field->payload_variant >> 41) &
            JADREN_ENUM_FIELD_MULTI_TAG_MASK;
        first_variant = (field->payload_variant >> 21) &
            JADREN_ENUM_FIELD_MULTI_TAG_MASK;
        second_variant = (field->payload_variant >> 1) &
            JADREN_ENUM_FIELD_MULTI_TAG_MASK;
        *active = ((uint64_t)outer_tag == outer_variant &&
                   (uint64_t)first_tag == first_variant &&
                   (uint64_t)second_tag == second_variant);
        return 0;
    }
    if ((field->payload_variant & (UINT64_C(1) << 63)) != 0U) {
        uint64_t outer_variant;
        uint64_t inner_variant;
        if (field->payload_offset < 8U) {
            return -15;
        }
        tag = (uint32_t)carrier[0] |
            ((uint32_t)carrier[1] << 8) |
            ((uint32_t)carrier[2] << 16) |
            ((uint32_t)carrier[3] << 24);
        outer_variant = (field->payload_variant >> 32) & UINT64_C(0x7fffffff);
        if ((uint64_t)tag != outer_variant) {
            *active = 0;
            return 0;
        }
        inner_variant = field->payload_variant & UINT64_C(0xffffffff);
        if (inner_variant == UINT64_C(0xffffffff)) {
            *active = 1;
            return 0;
        }
        tag = (uint32_t)carrier[field->payload_offset - 8U] |
            ((uint32_t)carrier[field->payload_offset - 7U] << 8) |
            ((uint32_t)carrier[field->payload_offset - 6U] << 16) |
            ((uint32_t)carrier[field->payload_offset - 5U] << 24);
        *active = ((uint64_t)tag == inner_variant);
        return 0;
    }
    tag = (uint32_t)carrier[0] |
        ((uint32_t)carrier[1] << 8) |
        ((uint32_t)carrier[2] << 16) |
        ((uint32_t)carrier[3] << 24);
    *active = ((uint64_t)tag == field->payload_variant);
    return 0;
}

static const JadrenCarrierDropBranch *jadren_find_enum_carrier_branch(
    const JadrenCarrierDropBranch *branches, uint64_t branch_count,
    uint64_t tag) {
    uint64_t index;
    for (index = 0U; index < branch_count; index += 1U) {
        if (branches[index].payload_variant == tag) {
            return &branches[index];
        }
    }
    return 0;
}

int32_t jadren_rt_buffer_destroy_enum_carrier_buffer(
    void *descriptor, uint64_t element_size, uint64_t alignment,
    uint64_t payload_offset, const JadrenCarrierDropBranch *branches,
    uint64_t branch_count) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    uint64_t index;
    int32_t status;
    int32_t branch_status = jadren_validate_enum_carrier_branches(
        branches, branch_count);
    if (buffer == 0 || element_size == 0U || alignment == 0U ||
        (alignment & (alignment - 1U)) != 0U || payload_offset % 8U != 0U ||
        payload_offset > element_size ||
        element_size - payload_offset < (uint64_t)sizeof(JadrenBufferDescriptor) ||
        branch_status != 0) {
        return branch_status != 0 ? branch_status : -15;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    for (index = 0U; index < buffer->length; index += 1U) {
        unsigned char *carrier = (unsigned char *)buffer->data +
            index * element_size;
        uint32_t tag = (uint32_t)carrier[0] |
            ((uint32_t)carrier[1] << 8) |
            ((uint32_t)carrier[2] << 16) |
            ((uint32_t)carrier[3] << 24);
        const JadrenCarrierDropBranch *branch =
            jadren_find_enum_carrier_branch(branches, branch_count, (uint64_t)tag);
        if (branch == 0) {
            continue;
        }
        {
            JadrenBufferDescriptor *nested = (JadrenBufferDescriptor *)
                (carrier + payload_offset);
            if (branch->depth == 1U) {
                status = jadren_rt_buffer_destroy(
                    nested, branch->leaf_element_size, branch->leaf_alignment);
            } else {
                status = jadren_rt_buffer_destroy_nested_buffer_recursive(
                    nested, (uint64_t)sizeof(JadrenBufferDescriptor), 8U,
                    branch->depth - 1U, branch->leaf_element_size,
                    branch->leaf_alignment);
            }
            if (status != 0) {
                return status;
            }
        }
    }
    if (buffer->data != 0 && !HeapFree(GetProcessHeap(), 0, buffer->data)) {
        return -10;
    }
    buffer->data = 0;
    buffer->length = 0U;
    buffer->capacity = 0U;
    return 0;
}

int32_t jadren_rt_buffer_destroy_record_fields(
    void *descriptor, uint64_t element_size, uint64_t alignment,
    const JadrenCarrierDropField *fields, uint64_t field_count) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    uint64_t index;
    int32_t status;
    int32_t field_status = jadren_validate_enum_carrier_fields(
        fields, field_count, element_size);
    if (buffer == 0 || element_size == 0U || alignment == 0U ||
        (alignment & (alignment - 1U)) != 0U || field_status != 0) {
        return field_status != 0 ? field_status : -15;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    for (index = 0U; index < buffer->length; index += 1U) {
        unsigned char *record = (unsigned char *)buffer->data +
            index * element_size;
        status = jadren_rt_carrier_destroy_record_fields(
            record, element_size, fields, field_count);
        if (status != 0) {
            return status;
        }
    }
    if (buffer->data != 0 && !HeapFree(GetProcessHeap(), 0, buffer->data)) {
        return -10;
    }
    buffer->data = 0;
    buffer->length = 0U;
    buffer->capacity = 0U;
    return 0;
}

int32_t jadren_rt_buffer_destroy_nested_record_fields(
    void *descriptor, uint64_t element_size, uint64_t alignment,
    uint64_t depth, uint64_t record_element_size, uint64_t record_alignment,
    const JadrenCarrierDropField *fields, uint64_t field_count) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    uint64_t index;
    int32_t status;
    int32_t field_status = jadren_validate_enum_carrier_fields(
        fields, field_count, record_element_size);
    if (buffer == 0 || depth == 0U ||
        element_size != (uint64_t)sizeof(JadrenBufferDescriptor) ||
        alignment != 8U || record_element_size == 0U ||
        record_alignment == 0U ||
        (record_alignment & (record_alignment - 1U)) != 0U ||
        field_status != 0) {
        return field_status != 0 ? field_status : -15;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    for (index = 0U; index < buffer->length; index += 1U) {
        JadrenBufferDescriptor *nested = (JadrenBufferDescriptor *)
            ((unsigned char *)buffer->data + index * element_size);
        if (depth == 1U) {
            status = jadren_rt_buffer_destroy_record_fields(
                nested, record_element_size, record_alignment,
                fields, field_count);
        } else {
            status = jadren_rt_buffer_destroy_nested_record_fields(
                nested, (uint64_t)sizeof(JadrenBufferDescriptor), 8U,
                depth - 1U, record_element_size, record_alignment,
                fields, field_count);
        }
        if (status != 0) {
            return status;
        }
    }
    if (buffer->data != 0 && !HeapFree(GetProcessHeap(), 0, buffer->data)) {
        return -10;
    }
    buffer->data = 0;
    buffer->length = 0U;
    buffer->capacity = 0U;
    return 0;
}

int32_t jadren_rt_buffer_resize_move_nested_record_fields_status(
    void *descriptor, uint64_t new_length, uint64_t element_size,
    uint64_t alignment, uint64_t depth, uint64_t record_element_size,
    uint64_t record_alignment, const JadrenCarrierDropField *fields,
    uint64_t field_count) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    uint64_t old_length;
    uint64_t index;
    uint64_t byte_index;
    uint64_t old_bytes;
    uint64_t new_bytes;
    int32_t status;
    int32_t field_status = jadren_validate_enum_carrier_fields(
        fields, field_count, record_element_size);
    if (buffer == 0 || depth == 0U ||
        element_size != (uint64_t)sizeof(JadrenBufferDescriptor) ||
        alignment != 8U || record_element_size == 0U ||
        record_alignment == 0U ||
        (record_alignment & (record_alignment - 1U)) != 0U ||
        field_status != 0) {
        return field_status != 0 ? field_status : -15;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity ||
        buffer->capacity > UINT64_MAX / element_size) {
        return -21;
    }
    old_length = buffer->length;
    if (new_length > old_length) {
        status = buffer_reserve_status(buffer, new_length, element_size, alignment);
        if (status != 0) {
            return status;
        }
        if (old_length > UINT64_MAX / element_size ||
            new_length > UINT64_MAX / element_size) {
            return -13;
        }
        old_bytes = old_length * element_size;
        new_bytes = new_length * element_size;
        for (byte_index = old_bytes; byte_index < new_bytes; byte_index += 1U) {
            ((unsigned char *)buffer->data)[byte_index] = 0U;
        }
    } else if (new_length < old_length) {
        for (index = new_length; index < old_length; index += 1U) {
            JadrenBufferDescriptor *nested = (JadrenBufferDescriptor *)
                ((unsigned char *)buffer->data + index * element_size);
            if (depth == 1U) {
                status = jadren_rt_buffer_destroy_record_fields(
                    nested, record_element_size, record_alignment,
                    fields, field_count);
            } else {
                status = jadren_rt_buffer_destroy_nested_record_fields(
                    nested, (uint64_t)sizeof(JadrenBufferDescriptor), 8U,
                    depth - 1U, record_element_size, record_alignment,
                    fields, field_count);
            }
            if (status != 0) {
                return status;
            }
        }
    }
    buffer->length = new_length;
    return 0;
}

int32_t jadren_rt_buffer_resize_move_nested_record_fields(
    void *descriptor, uint64_t new_length, uint64_t element_size,
    uint64_t alignment, uint64_t depth, uint64_t record_element_size,
    uint64_t record_alignment, const JadrenCarrierDropField *fields,
    uint64_t field_count) {
    return jadren_rt_buffer_resize_move_nested_record_fields_status(
        descriptor, new_length, element_size, alignment, depth,
        record_element_size, record_alignment, fields, field_count) == 0;
}

int32_t jadren_rt_carrier_destroy_record_fields(
    void *record, uint64_t element_size,
    const JadrenCarrierDropField *fields, uint64_t field_count) {
    unsigned char *bytes = (unsigned char *)record;
    uint64_t field_index;
    int32_t status;
    int32_t field_status = jadren_validate_enum_carrier_fields(
        fields, field_count, element_size);
    if (bytes == 0 || element_size == 0U || field_status != 0) {
        return field_status != 0 ? field_status : -15;
    }
    for (field_index = 0U; field_index < field_count; field_index += 1U) {
        const JadrenCarrierDropField *field = &fields[field_index];
        int active = 0;
        if (field->payload_variant == UINT64_MAX) {
            active = 1;
        } else {
            status = jadren_enum_carrier_field_is_active(bytes, field, &active);
            if (status != 0) {
                return status;
            }
        }
        if (!active) {
            continue;
        }
        status = jadren_destroy_record_field(bytes, field);
        if (status != 0) {
            return status;
        }
    }
    for (field_index = 0U; field_index < element_size; field_index += 1U) {
        bytes[field_index] = 0U;
    }
    return 0;
}

int32_t jadren_rt_buffer_resize_move_record_fields_status(
    void *descriptor, uint64_t new_length, uint64_t element_size,
    uint64_t alignment, const JadrenCarrierDropField *fields,
    uint64_t field_count) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    uint64_t old_length;
    uint64_t index;
    uint64_t byte_index;
    uint64_t old_bytes;
    uint64_t new_bytes;
    int32_t status;
    int32_t field_status = jadren_validate_enum_carrier_fields(
        fields, field_count, element_size);
    if (buffer == 0 || element_size == 0U || alignment == 0U ||
        (alignment & (alignment - 1U)) != 0U || field_status != 0) {
        return field_status != 0 ? field_status : -15;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity ||
        buffer->capacity > UINT64_MAX / element_size) {
        return -21;
    }
    old_length = buffer->length;
    if (new_length > old_length) {
        status = buffer_reserve_status(buffer, new_length, element_size, alignment);
        if (status != 0) {
            return status;
        }
        if (old_length > UINT64_MAX / element_size ||
            new_length > UINT64_MAX / element_size) {
            return -13;
        }
        old_bytes = old_length * element_size;
        new_bytes = new_length * element_size;
        for (byte_index = old_bytes; byte_index < new_bytes; byte_index += 1U) {
            ((unsigned char *)buffer->data)[byte_index] = 0U;
        }
    } else if (new_length < old_length) {
        for (index = new_length; index < old_length; index += 1U) {
            status = jadren_rt_carrier_destroy_record_fields(
                (unsigned char *)buffer->data + index * element_size,
                element_size, fields, field_count);
            if (status != 0) {
                return status;
            }
        }
    }
    buffer->length = new_length;
    return 0;
}

int32_t jadren_rt_buffer_resize_move_record_fields(
    void *descriptor, uint64_t new_length, uint64_t element_size,
    uint64_t alignment, const JadrenCarrierDropField *fields,
    uint64_t field_count) {
    return jadren_rt_buffer_resize_move_record_fields_status(
        descriptor, new_length, element_size, alignment,
        fields, field_count) == 0;
}

int32_t jadren_rt_buffer_remove_drop_record_fields_status(
    void *descriptor, uint64_t index, uint64_t element_size,
    uint64_t alignment, const JadrenCarrierDropField *fields,
    uint64_t field_count) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    unsigned char *base;
    uint64_t current;
    uint64_t byte_index;
    int32_t status;
    int32_t field_status = jadren_validate_enum_carrier_fields(
        fields, field_count, element_size);
    if (buffer == 0 || element_size == 0U || alignment == 0U ||
        (alignment & (alignment - 1U)) != 0U || field_status != 0) {
        return field_status != 0 ? field_status : -15;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity ||
        buffer->capacity > UINT64_MAX / element_size) {
        return -21;
    }
    if (index >= buffer->length) {
        return -20;
    }
    base = (unsigned char *)buffer->data;
    status = jadren_rt_carrier_destroy_record_fields(
        base + index * element_size, element_size, fields, field_count);
    if (status != 0) {
        return status;
    }
    for (current = index; current + 1U < buffer->length; current += 1U) {
        for (byte_index = 0U; byte_index < element_size; byte_index += 1U) {
            base[current * element_size + byte_index] =
                base[(current + 1U) * element_size + byte_index];
        }
    }
    for (byte_index = 0U; byte_index < element_size; byte_index += 1U) {
        base[(buffer->length - 1U) * element_size + byte_index] = 0U;
    }
    buffer->length -= 1U;
    return 0;
}

int32_t jadren_rt_buffer_remove_drop_record_fields(
    void *descriptor, uint64_t index, uint64_t element_size,
    uint64_t alignment, const JadrenCarrierDropField *fields,
    uint64_t field_count) {
    return jadren_rt_buffer_remove_drop_record_fields_status(
        descriptor, index, element_size, alignment, fields, field_count) == 0;
}

int32_t jadren_rt_buffer_remove_drop_nested_record_fields_status(
    void *descriptor, uint64_t index, uint64_t element_size,
    uint64_t alignment, uint64_t depth, uint64_t record_element_size,
    uint64_t record_alignment, const JadrenCarrierDropField *fields,
    uint64_t field_count) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    JadrenBufferDescriptor *base;
    uint64_t current;
    int32_t status;
    int32_t field_status = jadren_validate_enum_carrier_fields(
        fields, field_count, record_element_size);
    if (buffer == 0 || depth == 0U ||
        element_size != (uint64_t)sizeof(JadrenBufferDescriptor) ||
        alignment != 8U || record_element_size == 0U ||
        record_alignment == 0U ||
        (record_alignment & (record_alignment - 1U)) != 0U ||
        field_status != 0) {
        return field_status != 0 ? field_status : -15;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity ||
        buffer->capacity > UINT64_MAX / element_size) {
        return -21;
    }
    if (index >= buffer->length) {
        return -20;
    }
    base = (JadrenBufferDescriptor *)buffer->data;
    if (depth == 1U) {
        status = jadren_rt_buffer_destroy_record_fields(
            &base[index], record_element_size, record_alignment,
            fields, field_count);
    } else {
        status = jadren_rt_buffer_destroy_nested_record_fields(
            &base[index], element_size, alignment, depth - 1U,
            record_element_size, record_alignment, fields, field_count);
    }
    if (status != 0) {
        return status;
    }
    for (current = index; current + 1U < buffer->length; current += 1U) {
        base[current] = base[current + 1U];
        base[current + 1U].data = 0;
        base[current + 1U].length = 0U;
        base[current + 1U].capacity = 0U;
    }
    buffer->length -= 1U;
    return 0;
}

int32_t jadren_rt_buffer_remove_drop_nested_record_fields(
    void *descriptor, uint64_t index, uint64_t element_size,
    uint64_t alignment, uint64_t depth, uint64_t record_element_size,
    uint64_t record_alignment, const JadrenCarrierDropField *fields,
    uint64_t field_count) {
    return jadren_rt_buffer_remove_drop_nested_record_fields_status(
        descriptor, index, element_size, alignment, depth,
        record_element_size, record_alignment, fields, field_count) == 0;
}

int32_t jadren_rt_buffer_remove_drop_nested_buffer_status(
    void *descriptor, uint64_t index, uint64_t element_size,
    uint64_t alignment, uint64_t depth, uint64_t leaf_element_size,
    uint64_t leaf_alignment) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    JadrenBufferDescriptor *base;
    uint64_t current;
    int32_t status;
    if (buffer == 0 || depth == 0U ||
        element_size != (uint64_t)sizeof(JadrenBufferDescriptor) ||
        alignment != 8U || leaf_element_size == 0U ||
        leaf_alignment == 0U ||
        (leaf_alignment & (leaf_alignment - 1U)) != 0U) {
        return -15;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity ||
        buffer->capacity > UINT64_MAX / element_size) {
        return -21;
    }
    if (index >= buffer->length) {
        return -20;
    }
    base = (JadrenBufferDescriptor *)buffer->data;
    if (depth == 1U) {
        status = jadren_rt_buffer_destroy(
            &base[index], leaf_element_size, leaf_alignment);
    } else {
        status = jadren_rt_buffer_destroy_nested_buffer_recursive(
            &base[index], element_size, alignment, depth - 1U,
            leaf_element_size, leaf_alignment);
    }
    if (status != 0) {
        return status;
    }
    for (current = index; current + 1U < buffer->length; current += 1U) {
        base[current] = base[current + 1U];
        base[current + 1U].data = 0;
        base[current + 1U].length = 0U;
        base[current + 1U].capacity = 0U;
    }
    buffer->length -= 1U;
    return 0;
}

int32_t jadren_rt_buffer_remove_drop_nested_buffer(
    void *descriptor, uint64_t index, uint64_t element_size,
    uint64_t alignment, uint64_t depth, uint64_t leaf_element_size,
    uint64_t leaf_alignment) {
    return jadren_rt_buffer_remove_drop_nested_buffer_status(
        descriptor, index, element_size, alignment, depth,
        leaf_element_size, leaf_alignment) == 0;
}

int32_t jadren_rt_carrier_destroy_enum_buffer(
    void *carrier, uint64_t element_size, uint64_t payload_offset,
    const JadrenCarrierDropBranch *branches, uint64_t branch_count) {
    unsigned char *bytes = (unsigned char *)carrier;
    uint32_t tag;
    int32_t status;
    int32_t branch_status = jadren_validate_enum_carrier_branches(
        branches, branch_count);
    uint64_t index;
    if (bytes == 0 || element_size == 0U || payload_offset % 8U != 0U ||
        payload_offset > element_size ||
        element_size - payload_offset < (uint64_t)sizeof(JadrenBufferDescriptor) ||
        branch_status != 0) {
        return branch_status != 0 ? branch_status : -15;
    }
    tag = (uint32_t)bytes[0] |
        ((uint32_t)bytes[1] << 8) |
        ((uint32_t)bytes[2] << 16) |
        ((uint32_t)bytes[3] << 24);
    {
        const JadrenCarrierDropBranch *branch =
            jadren_find_enum_carrier_branch(branches, branch_count, (uint64_t)tag);
        if (branch != 0) {
            JadrenBufferDescriptor *nested = (JadrenBufferDescriptor *)
                (bytes + payload_offset);
            if (branch->depth == 1U) {
                status = jadren_rt_buffer_destroy(
                    nested, branch->leaf_element_size, branch->leaf_alignment);
            } else {
                status = jadren_rt_buffer_destroy_nested_buffer_recursive(
                    nested, (uint64_t)sizeof(JadrenBufferDescriptor), 8U,
                    branch->depth - 1U, branch->leaf_element_size,
                    branch->leaf_alignment);
            }
            if (status != 0) {
                return status;
            }
        }
    }
    for (index = 0U; index < element_size; index += 1U) {
        bytes[index] = 0U;
    }
    return 0;
}

static int32_t jadren_validate_enum_carrier_fields(
    const JadrenCarrierDropField *fields, uint64_t field_count,
    uint64_t element_size) {
    uint64_t index;
    uint64_t previous;
    if (fields == 0 || field_count == 0U || field_count > 1024U) {
        return -11;
    }
    for (index = 0U; index < field_count; index += 1U) {
        uint64_t path_byte = fields[index].payload_variant >> 56;
        int is_path = (path_byte & UINT64_C(0xf0)) ==
            (JADREN_ENUM_FIELD_PATH_MARKER_BASE >> 56);
        if (fields[index].depth == 0U ||
            fields[index].leaf_element_size == 0U ||
            fields[index].leaf_alignment == 0U ||
            (fields[index].leaf_alignment &
             (fields[index].leaf_alignment - 1U)) != 0U ||
            fields[index].payload_offset % 8U != 0U ||
            fields[index].payload_offset > element_size ||
            element_size - fields[index].payload_offset <
                (uint64_t)sizeof(JadrenBufferDescriptor)) {
            return -11;
        }
        if (fields[index].depth == JADREN_RECORD_FIELD_OWNED_STRING &&
            (fields[index].leaf_element_size != 24U ||
             fields[index].leaf_alignment != 8U)) {
            return -11;
        }
        if (!is_path && fields[index].payload_variant != UINT64_MAX &&
            (fields[index].payload_variant & JADREN_ENUM_FIELD_MULTI_TAG_MARKER) ==
                JADREN_ENUM_FIELD_MULTI_TAG_MARKER &&
            (fields[index].payload_offset < 24U ||
             (fields[index].payload_variant & UINT64_C(1)) != 0U)) {
            return -12;
        }
        for (previous = 0U; previous < index; previous += 1U) {
            if (fields[previous].payload_variant == fields[index].payload_variant &&
                fields[previous].payload_offset == fields[index].payload_offset) {
                return -21;
            }
        }
    }
    return 0;
}

int32_t jadren_rt_buffer_destroy_enum_carrier_fields(
    void *descriptor, uint64_t element_size, uint64_t alignment,
    const JadrenCarrierDropField *fields, uint64_t field_count) {
    JadrenBufferDescriptor *buffer = (JadrenBufferDescriptor *)descriptor;
    uint64_t index;
    int32_t status;
    int32_t field_status = jadren_validate_enum_carrier_fields(
        fields, field_count, element_size);
    if (buffer == 0 || element_size == 0U || alignment == 0U ||
        (alignment & (alignment - 1U)) != 0U || field_status != 0) {
        return field_status != 0 ? field_status : -15;
    }
    if ((buffer->data == 0 && buffer->capacity != 0U) ||
        buffer->length > buffer->capacity) {
        return -21;
    }
    for (index = 0U; index < buffer->length; index += 1U) {
        unsigned char *carrier = (unsigned char *)buffer->data +
            index * element_size;
        uint64_t field_index;
        for (field_index = 0U; field_index < field_count; field_index += 1U) {
            const JadrenCarrierDropField *field = &fields[field_index];
            int active;
            if (jadren_enum_carrier_field_is_active(carrier, field, &active) != 0) {
                return -15;
            }
            if (!active) {
                continue;
            }
            status = jadren_destroy_record_field(carrier, field);
            if (status != 0) {
                return status;
            }
        }
    }
    if (buffer->data != 0 && !HeapFree(GetProcessHeap(), 0, buffer->data)) {
        return -10;
    }
    buffer->data = 0;
    buffer->length = 0U;
    buffer->capacity = 0U;
    return 0;
}

int32_t jadren_rt_carrier_destroy_enum_fields(
    void *carrier, uint64_t element_size,
    const JadrenCarrierDropField *fields, uint64_t field_count) {
    unsigned char *bytes = (unsigned char *)carrier;
    uint64_t field_index;
    int32_t status;
    int32_t field_status = jadren_validate_enum_carrier_fields(
        fields, field_count, element_size);
    if (bytes == 0 || element_size == 0U || field_status != 0) {
        return field_status != 0 ? field_status : -15;
    }
    {
        for (field_index = 0U; field_index < field_count; field_index += 1U) {
            const JadrenCarrierDropField *field = &fields[field_index];
            int active;
            if (jadren_enum_carrier_field_is_active(bytes, field, &active) != 0) {
                return -15;
            }
            if (!active) {
                continue;
            }
            status = jadren_destroy_record_field(bytes, field);
            if (status != 0) {
                return status;
            }
        }
    }
    for (field_index = 0U; field_index < element_size; field_index += 1U) {
        bytes[field_index] = 0U;
    }
    return 0;
}

int32_t jadren_rt_carrier_destroy_multi_buffer(
    void *carrier, uint64_t element_size, uint64_t payload_offset,
    uint64_t first_variant, uint64_t first_depth,
    uint64_t first_leaf_element_size, uint64_t first_leaf_alignment,
    uint64_t second_variant, uint64_t second_depth,
    uint64_t second_leaf_element_size, uint64_t second_leaf_alignment) {
    unsigned char *bytes = (unsigned char *)carrier;
    uint32_t tag;
    int32_t status;
    uint64_t depth;
    uint64_t leaf_element_size;
    uint64_t leaf_alignment;
    uint64_t index;
    if (bytes == 0 || first_depth == 0U || second_depth == 0U ||
        first_variant > 1U || second_variant > 1U ||
        first_variant == second_variant || element_size == 0U ||
        payload_offset % 8U != 0U || payload_offset > element_size ||
        element_size - payload_offset < (uint64_t)sizeof(JadrenBufferDescriptor) ||
        first_leaf_element_size == 0U || first_leaf_alignment == 0U ||
        (first_leaf_alignment & (first_leaf_alignment - 1U)) != 0U ||
        second_leaf_element_size == 0U || second_leaf_alignment == 0U ||
        (second_leaf_alignment & (second_leaf_alignment - 1U)) != 0U) {
        return -15;
    }
    tag = (uint32_t)bytes[0] |
        ((uint32_t)bytes[1] << 8) |
        ((uint32_t)bytes[2] << 16) |
        ((uint32_t)bytes[3] << 24);
    if ((uint64_t)tag == first_variant) {
        depth = first_depth;
        leaf_element_size = first_leaf_element_size;
        leaf_alignment = first_leaf_alignment;
    } else if ((uint64_t)tag == second_variant) {
        depth = second_depth;
        leaf_element_size = second_leaf_element_size;
        leaf_alignment = second_leaf_alignment;
    } else {
        return -21;
    }
    {
        JadrenBufferDescriptor *nested = (JadrenBufferDescriptor *)
            (bytes + payload_offset);
        if (depth == 1U) {
            status = jadren_rt_buffer_destroy(nested, leaf_element_size,
                                              leaf_alignment);
        } else {
            status = jadren_rt_buffer_destroy_nested_buffer_recursive(
                nested, (uint64_t)sizeof(JadrenBufferDescriptor), 8U,
                depth - 1U, leaf_element_size, leaf_alignment);
        }
        if (status != 0) {
            return status;
        }
    }
    for (index = 0U; index < element_size; index += 1U) {
        bytes[index] = 0U;
    }
    return 0;
}

void jadren_rt_native_region_destroy(void *handle) {
    JadrenRegion *region = (JadrenRegion *)handle;
    JadrenRegionAllocation *entry;
    JadrenRegionAllocation *next;
    if (region == 0) {
        return;
    }
    entry = region->allocations;
    while (entry != 0) {
        next = entry->next;
        jadren_region_free_raw(entry->raw);
        jadren_region_free_raw(entry);
        entry = next;
    }
    jadren_region_free_raw(region);
}
