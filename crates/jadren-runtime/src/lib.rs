//! Versioned initialization boundary for the Jadren native runtime.
//!
//! JADREN-UNSAFE-AUDIT: raw allocation, callback and C-ABI pointer code is
//! isolated in this module. Public pointer entry points document their caller
//! invariants in `# Safety` sections; safe Rust owns validation and status
//! conversion before any dereference or deallocation.

use std::alloc::{Layout, alloc, dealloc, realloc};
use std::ffi::c_void;
use std::mem::{align_of, size_of, transmute};
use std::process;
use std::ptr;
use std::slice;
use std::str;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

/// First incompatible-change generation of the native runtime ABI.
pub const RUNTIME_ABI_MAJOR: u32 = 0;
/// Backward-compatible feature generation of the native runtime ABI.
pub const RUNTIME_ABI_MINOR: u32 = 21;
/// Deterministic identity of this runtime build and ABI contract.
pub const RUNTIME_BUILD_ID: u64 = runtime_build_id();

const STATE_UNINITIALIZED: u8 = 0;
const STATE_INITIALIZED: u8 = 1;
static RUNTIME_STATE: AtomicU8 = AtomicU8::new(STATE_UNINITIALIZED);

// Callback registration is expected before worker threads start. Dispatch
// only performs atomic loads and a typed function-pointer call; it never takes
// a lock or allocates.
static LOG_CALLBACK: AtomicUsize = AtomicUsize::new(0);
static PROFILER_BEGIN_CALLBACK: AtomicUsize = AtomicUsize::new(0);
static PROFILER_END_CALLBACK: AtomicUsize = AtomicUsize::new(0);
static PROFILER_COUNTER_CALLBACK: AtomicUsize = AtomicUsize::new(0);
static CALLBACK_CONTEXT: AtomicUsize = AtomicUsize::new(0);

/// Major/minor version requested or provided at the runtime boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AbiVersion {
    pub major: u32,
    pub minor: u32,
}

impl AbiVersion {
    /// Current ABI implemented by this runtime binary.
    pub const CURRENT: Self = Self {
        major: RUNTIME_ABI_MAJOR,
        minor: RUNTIME_ABI_MINOR,
    };

    /// Packs major/minor into a stable integer for language-neutral hosts.
    #[must_use]
    pub const fn packed(self) -> u64 {
        (self.major as u64) << 32 | self.minor as u64
    }
}

/// Process-global runtime lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeState {
    Uninitialized,
    Initialized,
}

/// Stable result code returned across the runtime C ABI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i32)]
pub enum RuntimeStatus {
    Initialized = 0,
    AlreadyInitialized = 1,
    IncompatibleMajor = -1,
    IncompatibleMinor = -2,
}

/// Stable panic categories reserved for deterministic crash diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PanicCode {
    BoundsCheck = 1,
}

impl PanicCode {
    /// Returns the exact unsigned C ABI value.
    #[must_use]
    pub const fn code(self) -> u32 {
        self as u32
    }
}

/// Fixed-layout panic information for deterministic diagnostics and future
/// crash-report hooks.
///
/// `detail_a` and `detail_b` are operation-specific. For a bounds panic they
/// contain the attempted index and collection length respectively.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct PanicInfo {
    pub code: u32,
    pub detail_a: u64,
    pub detail_b: u64,
}

/// Log severity passed to the host callback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum LogLevel {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
}

impl LogLevel {
    /// Returns the exact unsigned C ABI value.
    #[must_use]
    pub const fn code(self) -> u32 {
        self as u32
    }

    fn from_raw(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Trace),
            1 => Some(Self::Debug),
            2 => Some(Self::Info),
            3 => Some(Self::Warn),
            4 => Some(Self::Error),
            _ => None,
        }
    }
}

/// Result code shared by callback registration and callback dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i32)]
pub enum CallbackStatus {
    Delivered = 0,
    Disabled = 1,
    RuntimeNotInitialized = -10,
    InvalidInput = -30,
}

impl CallbackStatus {
    /// Returns the exact signed C ABI value.
    #[must_use]
    pub const fn code(self) -> i32 {
        self as i32
    }
}

/// Host callback for one length-prefixed UTF-8 or binary log message.
///
/// The message pointer is borrowed only for the duration of the callback; the
/// runtime never allocates or copies it. The host must not unwind across this
/// ABI and should keep the callback short in realtime code.
pub type LogCallback =
    unsafe extern "C" fn(level: u32, message: *const u8, message_length: u64, context: *mut c_void);

/// Host callback for beginning one profiler sample.
pub type ProfilerBeginCallback = unsafe extern "C" fn(name_id: u64, context: *mut c_void);

/// Host callback for ending the current profiler sample.
pub type ProfilerEndCallback = unsafe extern "C" fn(context: *mut c_void);

/// Host callback for recording one profiler counter value.
pub type ProfilerCounterCallback =
    unsafe extern "C" fn(name_id: u64, value: i64, context: *mut c_void);

/// C-compatible owning buffer header used by the runtime core.
///
/// The element size and alignment are supplied to each operation by generated
/// code; keeping them out of the header preserves the JIR `{pointer,length,
/// capacity}` layout for generic `Buffer<T>` values.
#[derive(Debug, Eq, PartialEq)]
#[repr(C)]
pub struct Buffer {
    pub pointer: *mut c_void,
    pub length: u64,
    pub capacity: u64,
}

/// C-compatible branch descriptor used by named enum carrier drop glue.
///
/// Generated code materializes a short, immutable table on the stack and
/// passes it to the runtime. A missing tag entry denotes a copy-only enum
/// variant and therefore needs no destructor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct CarrierDropBranchAbi {
    pub payload_variant: u64,
    pub depth: u64,
    pub leaf_element_size: u64,
    pub leaf_alignment: u64,
}

/// C-compatible descriptor for one owning Buffer or OwnedString field in a named enum
/// carrier. Unlike [`CarrierDropBranchAbi`], multiple entries may share the
/// same variant; their offsets identify the individual descriptors to drop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct CarrierDropFieldAbi {
    pub payload_variant: u64,
    pub payload_offset: u64,
    pub depth: u64,
    pub leaf_element_size: u64,
    pub leaf_alignment: u64,
}

/// Sentinel used by record field tables for an unconditional Buffer field.
/// Other discriminants select an active `Option`/`Result` carrier branch.
const RECORD_FIELD_UNCONDITIONAL: u64 = u64::MAX;
/// Reserved field-table depth marker for a direct OwnedString descriptor.
const RECORD_FIELD_OWNED_STRING: u64 = u32::MAX as u64;

impl Buffer {
    const EMPTY: Self = Self {
        pointer: ptr::null_mut(),
        length: 0,
        capacity: 0,
    };
}

/// Pointer/status pair returned when creating a buffer.
#[derive(Debug, Eq, PartialEq)]
#[repr(C)]
pub struct BufferResult {
    pub buffer: Buffer,
    pub status: i32,
}

impl BufferResult {
    const fn failure(status: BufferStatus) -> Self {
        Self {
            buffer: Buffer::EMPTY,
            status: status.code(),
        }
    }

    const fn success(buffer: Buffer) -> Self {
        Self {
            buffer,
            status: BufferStatus::Ok.code(),
        }
    }
}

/// C-compatible non-owning slice view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct Slice {
    pub pointer: *mut c_void,
    pub length: u64,
}

/// Pointer/status pair returned when creating a slice view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct SliceResult {
    pub slice: Slice,
    pub status: i32,
}

impl SliceResult {
    const fn failure(status: BufferStatus) -> Self {
        Self {
            slice: Slice {
                pointer: ptr::null_mut(),
                length: 0,
            },
            status: status.code(),
        }
    }

    const fn success(slice: Slice) -> Self {
        Self {
            slice,
            status: BufferStatus::Ok.code(),
        }
    }
}

/// C-compatible owning UTF-8 string header used by the runtime core.
///
/// The byte payload is always valid UTF-8 for `length` bytes. Capacity is
/// stored explicitly so append/reserve can remain allocation-aware without
/// hidden metadata. The header is move-only; typed Jadren code owns it and
/// must destroy it exactly once.
#[derive(Debug, Eq, PartialEq)]
#[repr(C)]
pub struct Utf8String {
    pub pointer: *mut u8,
    pub length: u64,
    pub capacity: u64,
}

impl Utf8String {
    const EMPTY: Self = Self {
        pointer: ptr::null_mut(),
        length: 0,
        capacity: 0,
    };
}

/// Result returned by fallible UTF-8 string constructors.
#[derive(Debug, Eq, PartialEq)]
#[repr(C)]
pub struct Utf8StringResult {
    pub string: Utf8String,
    pub status: i32,
}

impl Utf8StringResult {
    const fn failure(status: StringStatus) -> Self {
        Self {
            string: Utf8String::EMPTY,
            status: status.code(),
        }
    }

    const fn success(string: Utf8String) -> Self {
        Self {
            string,
            status: StringStatus::Ok.code(),
        }
    }
}

/// Stable result code returned by UTF-8 string operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i32)]
pub enum StringStatus {
    Ok = 0,
    Disabled = 1,
    RuntimeNotInitialized = -10,
    InvalidSize = -11,
    InvalidAlignment = -12,
    SizeOverflow = -13,
    OutOfMemory = -14,
    NullPointer = -15,
    InvalidUtf8 = -40,
    InvalidString = -41,
}

impl StringStatus {
    /// Returns the exact signed C ABI value.
    #[must_use]
    pub const fn code(self) -> i32 {
        self as i32
    }
}

/// Stable result code for owning buffers and non-owning slices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i32)]
pub enum BufferStatus {
    Ok = 0,
    Disabled = 1,
    RuntimeNotInitialized = -10,
    InvalidSize = -11,
    InvalidAlignment = -12,
    SizeOverflow = -13,
    OutOfMemory = -14,
    NullPointer = -15,
    OutOfBounds = -20,
    InvalidBuffer = -21,
}

impl BufferStatus {
    /// Returns the exact signed C ABI value.
    #[must_use]
    pub const fn code(self) -> i32 {
        self as i32
    }
}

/// Stable system allocator result code returned across the C ABI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i32)]
pub enum AllocatorStatus {
    Ok = 0,
    RuntimeNotInitialized = -10,
    InvalidSize = -11,
    InvalidAlignment = -12,
    SizeOverflow = -13,
    OutOfMemory = -14,
    NullPointer = -15,
}

impl AllocatorStatus {
    /// Returns the exact signed C ABI status value.
    #[must_use]
    pub const fn code(self) -> i32 {
        self as i32
    }
}

/// Pointer/status pair returned by system allocation and reallocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct AllocationResult {
    pub pointer: *mut c_void,
    pub status: i32,
}

impl AllocationResult {
    const fn failure(status: AllocatorStatus) -> Self {
        Self {
            pointer: ptr::null_mut(),
            status: status.code(),
        }
    }

    fn success(pointer: *mut u8) -> Self {
        Self {
            pointer: pointer.cast(),
            status: AllocatorStatus::Ok.code(),
        }
    }
}

/// Opaque region handle/status pair returned across the C ABI.
///
/// A region owns every allocation made through its handle and releases all of
/// them when [`region_destroy`] is called. The pointer must not be inspected or
/// freed by the caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct RegionResult {
    pub pointer: *mut c_void,
    pub status: i32,
}

impl RegionResult {
    const fn failure(status: AllocatorStatus) -> Self {
        Self {
            pointer: ptr::null_mut(),
            status: status.code(),
        }
    }

    fn success(pointer: *mut Region) -> Self {
        Self {
            pointer: pointer.cast(),
            status: AllocatorStatus::Ok.code(),
        }
    }
}

/// A lexical arena that bulk-releases all of its allocations at destruction.
///
/// The language-level region analysis guarantees that the handle and all
/// region-owned values stay within the lexical region. The runtime keeps the
/// allocation layouts so destruction can return every block to the system
/// allocator without requiring individual frees from generated code.
pub struct Region {
    allocations: Vec<RegionAllocation>,
}

struct RegionAllocation {
    pointer: *mut u8,
    layout: Layout,
}

impl Region {
    fn new() -> Self {
        Self {
            allocations: Vec::new(),
        }
    }

    /// Allocates one aligned block owned by this region.
    ///
    /// # Safety
    ///
    /// The region pointer must be a live handle returned by [`region_create`]
    /// and must not be used concurrently with another mutable operation.
    #[must_use]
    #[allow(unsafe_code)]
    unsafe fn allocate(&mut self, size: u64, alignment: u64) -> AllocationResult {
        let layout = match allocator_layout_initialized(size, alignment) {
            Ok(layout) => layout,
            Err(status) => return AllocationResult::failure(status),
        };

        // Reserve metadata first so a successful block allocation can never
        // be leaked if recording it would run out of memory.
        if self.allocations.try_reserve(1).is_err() {
            return AllocationResult::failure(AllocatorStatus::OutOfMemory);
        }

        // SAFETY: `layout` is a validated nonzero allocation layout. Region
        // storage is zero-initialized so a freshly allocated Buffer has a
        // deterministic element value before the Jadren program mutates it.
        let pointer = unsafe { alloc(layout) };
        if pointer.is_null() {
            return AllocationResult::failure(AllocatorStatus::OutOfMemory);
        }
        // SAFETY: `pointer` references the complete allocation described by
        // `layout`; writing bytes does not require a typed Rust value.
        unsafe { ptr::write_bytes(pointer, 0, layout.size()) };
        self.allocations.push(RegionAllocation { pointer, layout });
        AllocationResult::success(pointer)
    }
}

#[allow(unsafe_code)]
impl Drop for Region {
    fn drop(&mut self) {
        for allocation in self.allocations.drain(..) {
            // SAFETY: every entry was created with this exact layout by
            // `Region::allocate` and remains live until region destruction.
            unsafe { dealloc(allocation.pointer, allocation.layout) };
        }
    }
}

impl RuntimeStatus {
    /// Returns the exact signed C ABI status value.
    #[must_use]
    pub const fn code(self) -> i32 {
        self as i32
    }
}

/// Initializes the process-global runtime after an ABI compatibility check.
///
/// Initialization is thread-safe and idempotent. A host compiled against the
/// same major and an equal-or-older minor version is compatible.
pub fn initialize(required: AbiVersion) -> RuntimeStatus {
    if required.major != RUNTIME_ABI_MAJOR {
        return RuntimeStatus::IncompatibleMajor;
    }
    if required.minor > RUNTIME_ABI_MINOR {
        return RuntimeStatus::IncompatibleMinor;
    }
    match RUNTIME_STATE.compare_exchange(
        STATE_UNINITIALIZED,
        STATE_INITIALIZED,
        Ordering::AcqRel,
        Ordering::Acquire,
    ) {
        Ok(_) => RuntimeStatus::Initialized,
        Err(STATE_INITIALIZED) => RuntimeStatus::AlreadyInitialized,
        Err(_) => unreachable!("runtime state contains an unknown value"),
    }
}

/// Returns the current process-global runtime state.
#[must_use]
pub fn runtime_state() -> RuntimeState {
    match RUNTIME_STATE.load(Ordering::Acquire) {
        STATE_UNINITIALIZED => RuntimeState::Uninitialized,
        STATE_INITIALIZED => RuntimeState::Initialized,
        _ => unreachable!("runtime state contains an unknown value"),
    }
}

/// Registers the optional logging and profiler callbacks.
///
/// Registration is process-global and should happen before worker threads
/// start. Replacing a table while callbacks are executing is supported as a
/// best-effort transition; hosts must keep both old and new contexts valid
/// until the transition completes. A table with no callbacks disables all
/// dispatch and clears its context.
#[must_use]
pub fn set_callbacks(
    log: Option<LogCallback>,
    profiler_begin: Option<ProfilerBeginCallback>,
    profiler_end: Option<ProfilerEndCallback>,
    profiler_counter: Option<ProfilerCounterCallback>,
    context: *mut c_void,
) -> CallbackStatus {
    if runtime_state() != RuntimeState::Initialized {
        return CallbackStatus::RuntimeNotInitialized;
    }
    let has_callback = log.is_some()
        || profiler_begin.is_some()
        || profiler_end.is_some()
        || profiler_counter.is_some();
    if !has_callback && !context.is_null() {
        return CallbackStatus::InvalidInput;
    }

    // Disable first, publish the context, then publish callback addresses.
    // Acquire dispatch loads cannot call a newly published function with an
    // uninitialized context. Registration itself remains allocation-free.
    LOG_CALLBACK.store(0, Ordering::Release);
    PROFILER_BEGIN_CALLBACK.store(0, Ordering::Release);
    PROFILER_END_CALLBACK.store(0, Ordering::Release);
    PROFILER_COUNTER_CALLBACK.store(0, Ordering::Release);
    CALLBACK_CONTEXT.store(
        if has_callback { context as usize } else { 0 },
        Ordering::Release,
    );
    LOG_CALLBACK.store(
        log.map_or(0, |callback| callback as usize),
        Ordering::Release,
    );
    PROFILER_BEGIN_CALLBACK.store(
        profiler_begin.map_or(0, |callback| callback as usize),
        Ordering::Release,
    );
    PROFILER_END_CALLBACK.store(
        profiler_end.map_or(0, |callback| callback as usize),
        Ordering::Release,
    );
    PROFILER_COUNTER_CALLBACK.store(
        profiler_counter.map_or(0, |callback| callback as usize),
        Ordering::Release,
    );
    if has_callback {
        CallbackStatus::Delivered
    } else {
        CallbackStatus::Disabled
    }
}

/// Sends one log message to the registered host callback without allocation.
#[allow(unsafe_code)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[must_use]
pub fn log(level: u32, message: *const u8, message_length: u64) -> CallbackStatus {
    if runtime_state() != RuntimeState::Initialized {
        return CallbackStatus::RuntimeNotInitialized;
    }
    if LogLevel::from_raw(level).is_none() || (message_length != 0 && message.is_null()) {
        return CallbackStatus::InvalidInput;
    }
    let address = LOG_CALLBACK.load(Ordering::Acquire);
    if address == 0 {
        return CallbackStatus::Disabled;
    }
    let context = CALLBACK_CONTEXT.load(Ordering::Acquire) as *mut c_void;
    // SAFETY: the address was installed by `set_callbacks` from this exact
    // function-pointer type and remains valid under the registration contract.
    let callback = unsafe { transmute::<usize, LogCallback>(address) };
    // SAFETY: callback ownership and borrowed message lifetime are the C ABI
    // contract documented on `LogCallback`.
    unsafe { callback(level, message, message_length, context) };
    CallbackStatus::Delivered
}

/// Begins one profiler sample through the registered callback.
#[allow(unsafe_code)]
#[must_use]
pub fn profiler_begin_sample(name_id: u64) -> CallbackStatus {
    if runtime_state() != RuntimeState::Initialized {
        return CallbackStatus::RuntimeNotInitialized;
    }
    let address = PROFILER_BEGIN_CALLBACK.load(Ordering::Acquire);
    if address == 0 {
        return CallbackStatus::Disabled;
    }
    let context = CALLBACK_CONTEXT.load(Ordering::Acquire) as *mut c_void;
    // SAFETY: the address was installed by `set_callbacks` from this exact
    // function-pointer type.
    let callback = unsafe { transmute::<usize, ProfilerBeginCallback>(address) };
    // SAFETY: callback ownership and context validity are the C ABI contract.
    unsafe { callback(name_id, context) };
    CallbackStatus::Delivered
}

/// Ends the current profiler sample through the registered callback.
#[allow(unsafe_code)]
#[must_use]
pub fn profiler_end_sample() -> CallbackStatus {
    if runtime_state() != RuntimeState::Initialized {
        return CallbackStatus::RuntimeNotInitialized;
    }
    let address = PROFILER_END_CALLBACK.load(Ordering::Acquire);
    if address == 0 {
        return CallbackStatus::Disabled;
    }
    let context = CALLBACK_CONTEXT.load(Ordering::Acquire) as *mut c_void;
    // SAFETY: the address was installed by `set_callbacks` from this exact
    // function-pointer type.
    let callback = unsafe { transmute::<usize, ProfilerEndCallback>(address) };
    // SAFETY: callback ownership and context validity are the C ABI contract.
    unsafe { callback(context) };
    CallbackStatus::Delivered
}

/// Sends one profiler counter value through the registered callback.
#[allow(unsafe_code)]
#[must_use]
pub fn profiler_counter(name_id: u64, value: i64) -> CallbackStatus {
    if runtime_state() != RuntimeState::Initialized {
        return CallbackStatus::RuntimeNotInitialized;
    }
    let address = PROFILER_COUNTER_CALLBACK.load(Ordering::Acquire);
    if address == 0 {
        return CallbackStatus::Disabled;
    }
    let context = CALLBACK_CONTEXT.load(Ordering::Acquire) as *mut c_void;
    // SAFETY: the address was installed by `set_callbacks` from this exact
    // function-pointer type.
    let callback = unsafe { transmute::<usize, ProfilerCounterCallback>(address) };
    // SAFETY: callback ownership and context validity are the C ABI contract.
    unsafe { callback(name_id, value, context) };
    CallbackStatus::Delivered
}

fn map_allocator_status(status: AllocatorStatus) -> BufferStatus {
    match status {
        AllocatorStatus::Ok => BufferStatus::Ok,
        AllocatorStatus::RuntimeNotInitialized => BufferStatus::RuntimeNotInitialized,
        AllocatorStatus::InvalidSize => BufferStatus::InvalidSize,
        AllocatorStatus::InvalidAlignment => BufferStatus::InvalidAlignment,
        AllocatorStatus::SizeOverflow => BufferStatus::SizeOverflow,
        AllocatorStatus::OutOfMemory => BufferStatus::OutOfMemory,
        AllocatorStatus::NullPointer => BufferStatus::NullPointer,
    }
}

fn map_string_allocator_status(status: AllocatorStatus) -> StringStatus {
    match status {
        AllocatorStatus::Ok => StringStatus::Ok,
        AllocatorStatus::RuntimeNotInitialized => StringStatus::RuntimeNotInitialized,
        AllocatorStatus::InvalidSize => StringStatus::InvalidSize,
        AllocatorStatus::InvalidAlignment => StringStatus::InvalidAlignment,
        AllocatorStatus::SizeOverflow => StringStatus::SizeOverflow,
        AllocatorStatus::OutOfMemory => StringStatus::OutOfMemory,
        AllocatorStatus::NullPointer => StringStatus::NullPointer,
    }
}

fn string_layout(capacity: u64) -> Result<Layout, StringStatus> {
    if capacity == 0 {
        return Err(StringStatus::InvalidSize);
    }
    allocator_layout_initialized(capacity, 1).map_err(map_string_allocator_status)
}

fn validate_string_header(string: &Utf8String) -> Result<(), StringStatus> {
    if string.length > string.capacity {
        return Err(StringStatus::InvalidString);
    }
    if string.capacity == 0 {
        if string.pointer.is_null() && string.length == 0 {
            Ok(())
        } else {
            Err(StringStatus::InvalidString)
        }
    } else if string.pointer.is_null()
        || usize::try_from(string.capacity).is_err()
        || usize::try_from(string.length).is_err()
    {
        Err(StringStatus::InvalidString)
    } else {
        Ok(())
    }
}

fn buffer_layout(element_size: u64, alignment: u64, capacity: u64) -> Result<Layout, BufferStatus> {
    let element_layout = validate_element_layout(element_size, alignment)?;
    let bytes = element_size
        .checked_mul(capacity)
        .ok_or(BufferStatus::SizeOverflow)?;
    if bytes == 0 {
        return Err(BufferStatus::InvalidSize);
    }
    let layout = allocator_layout_initialized(bytes, alignment).map_err(map_allocator_status)?;
    if layout.align() != element_layout.align() {
        return Err(BufferStatus::InvalidAlignment);
    }
    Ok(layout)
}

fn validate_element_layout(element_size: u64, alignment: u64) -> Result<Layout, BufferStatus> {
    if element_size == 0 {
        return Err(BufferStatus::InvalidSize);
    }
    let layout =
        allocator_layout_initialized(element_size, alignment).map_err(map_allocator_status)?;
    if !element_size.is_multiple_of(layout.align() as u64) {
        return Err(BufferStatus::InvalidAlignment);
    }
    Ok(layout)
}

fn validate_buffer(buffer: &Buffer) -> Result<(), BufferStatus> {
    if buffer.length > buffer.capacity {
        return Err(BufferStatus::InvalidBuffer);
    }
    if buffer.capacity == 0 {
        if buffer.pointer.is_null() {
            Ok(())
        } else {
            Err(BufferStatus::InvalidBuffer)
        }
    } else if buffer.pointer.is_null() {
        Err(BufferStatus::InvalidBuffer)
    } else {
        Ok(())
    }
}

const MAX_ENUM_CARRIER_BRANCHES: u64 = 1024;

#[allow(unsafe_code)]
unsafe fn validate_enum_carrier_branches(
    branches: *const CarrierDropBranchAbi,
    branch_count: u64,
) -> Result<(), BufferStatus> {
    if branches.is_null() || branch_count == 0 || branch_count > MAX_ENUM_CARRIER_BRANCHES {
        return Err(BufferStatus::InvalidSize);
    }
    for index in 0..branch_count {
        // SAFETY: callers of the public drop APIs provide a live table with
        // exactly `branch_count` entries; unaligned reads keep the C ABI
        // tolerant of stack packing from foreign hosts.
        let branch = unsafe { branches.add(index as usize).read_unaligned() };
        if branch.depth == 0 {
            return Err(BufferStatus::InvalidSize);
        }
        validate_element_layout(branch.leaf_element_size, branch.leaf_alignment)?;
        for previous in 0..index {
            // SAFETY: `previous < index < branch_count` is inside the table.
            let previous_branch = unsafe { branches.add(previous as usize).read_unaligned() };
            if previous_branch.payload_variant == branch.payload_variant {
                return Err(BufferStatus::InvalidBuffer);
            }
        }
    }
    Ok(())
}

#[allow(unsafe_code)]
unsafe fn enum_carrier_branch_for_tag(
    branches: *const CarrierDropBranchAbi,
    branch_count: u64,
    tag: u64,
) -> Option<CarrierDropBranchAbi> {
    for index in 0..branch_count {
        // SAFETY: validation has established the table bounds.
        let branch = unsafe { branches.add(index as usize).read_unaligned() };
        if branch.payload_variant == tag {
            return Some(branch);
        }
    }
    None
}

#[allow(unsafe_code)]
unsafe fn validate_enum_carrier_fields(
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
    element_size: u64,
) -> Result<(), BufferStatus> {
    if fields.is_null() || field_count == 0 || field_count > MAX_ENUM_CARRIER_BRANCHES {
        return Err(BufferStatus::InvalidSize);
    }
    for index in 0..field_count {
        // SAFETY: callers provide a table with exactly `field_count` entries.
        let field = unsafe { fields.add(index as usize).read_unaligned() };
        if field.depth == 0 {
            return Err(BufferStatus::InvalidSize);
        }
        validate_element_layout(field.leaf_element_size, field.leaf_alignment)?;
        if field.depth == RECORD_FIELD_OWNED_STRING
            && (field.leaf_element_size != size_of::<Utf8String>() as u64
                || field.leaf_alignment != align_of::<Utf8String>() as u64)
        {
            return Err(BufferStatus::InvalidSize);
        }
        let Some(payload_end) = field.payload_offset.checked_add(size_of::<Buffer>() as u64) else {
            return Err(BufferStatus::InvalidAlignment);
        };
        if field.payload_offset % align_of::<Buffer>() as u64 != 0 || payload_end > element_size {
            return Err(BufferStatus::InvalidAlignment);
        }
        for previous in 0..index {
            // SAFETY: `previous < index < field_count` is inside the table.
            let previous_field = unsafe { fields.add(previous as usize).read_unaligned() };
            if previous_field.payload_variant == field.payload_variant
                && previous_field.payload_offset == field.payload_offset
            {
                return Err(BufferStatus::InvalidBuffer);
            }
        }
    }
    Ok(())
}

/// Returns whether one record field is active for the current value.  The
/// compact record field ABI derives the tag location for the supported
/// `Option`/`Result` carriers from the fixed 8-byte Buffer payload offset.
#[allow(unsafe_code)]
unsafe fn record_field_is_active(
    record: *const u8,
    field: CarrierDropFieldAbi,
) -> Result<bool, BufferStatus> {
    if field.payload_variant == RECORD_FIELD_UNCONDITIONAL {
        return Ok(true);
    }
    if field.payload_offset < 8 {
        return Err(BufferStatus::InvalidAlignment);
    }
    // SAFETY: the field validator checked the payload offset; the carrier tag
    // is the four-byte word eight bytes before its aligned Buffer payload.
    let tag = unsafe {
        record
            .add((field.payload_offset - 8) as usize)
            .cast::<u32>()
            .read_unaligned()
    };
    Ok(u64::from(tag) == field.payload_variant)
}

/// Destroys one already-validated record field. The field-table ABI uses the
/// same descriptor-sized slot for Buffer and OwnedString; the depth marker
/// selects the correct destructor without adding per-record hidden metadata.
#[allow(unsafe_code)]
unsafe fn destroy_record_field(record: *mut u8, field: CarrierDropFieldAbi) -> BufferStatus {
    // SAFETY: callers validate the field offset and descriptor layout before
    // invoking this helper.
    let payload = unsafe { record.add(field.payload_offset as usize) };
    if field.depth == RECORD_FIELD_OWNED_STRING {
        // SAFETY: the validator checked the exact Utf8String layout.
        let status = unsafe { string_destroy(payload.cast::<Utf8String>()) };
        return match status {
            StringStatus::Ok => BufferStatus::Ok,
            StringStatus::RuntimeNotInitialized => BufferStatus::RuntimeNotInitialized,
            StringStatus::NullPointer => BufferStatus::NullPointer,
            StringStatus::InvalidSize => BufferStatus::InvalidSize,
            StringStatus::InvalidAlignment => BufferStatus::InvalidAlignment,
            StringStatus::SizeOverflow => BufferStatus::SizeOverflow,
            StringStatus::OutOfMemory => BufferStatus::OutOfMemory,
            _ => BufferStatus::InvalidBuffer,
        };
    }
    let nested = payload.cast::<Buffer>();
    if field.depth == 1 {
        // SAFETY: the field table describes a live Buffer descriptor.
        unsafe { buffer_destroy(nested, field.leaf_element_size, field.leaf_alignment) }
    } else {
        // SAFETY: the field table describes a live nested Buffer chain.
        unsafe {
            buffer_destroy_nested_buffer_recursive(
                nested,
                size_of::<Buffer>() as u64,
                align_of::<Buffer>() as u64,
                field.depth - 1,
                field.leaf_element_size,
                field.leaf_alignment,
            )
        }
    }
}

/// Creates an owning buffer with zero logical length and the requested
/// element capacity.
#[must_use]
#[allow(unsafe_code)]
pub fn buffer_create(element_size: u64, alignment: u64, capacity: u64) -> BufferResult {
    if runtime_state() != RuntimeState::Initialized {
        return BufferResult::failure(BufferStatus::RuntimeNotInitialized);
    }
    if capacity == 0 {
        if let Err(status) = validate_element_layout(element_size, alignment) {
            return BufferResult::failure(status);
        }
        return BufferResult::success(Buffer::EMPTY);
    }
    let layout = match buffer_layout(element_size, alignment, capacity) {
        Ok(layout) => layout,
        Err(status) => return BufferResult::failure(status),
    };
    // SAFETY: `layout` is validated and nonzero.
    let pointer = unsafe { alloc(layout) };
    if pointer.is_null() {
        BufferResult::failure(BufferStatus::OutOfMemory)
    } else {
        BufferResult::success(Buffer {
            pointer: pointer.cast(),
            length: 0,
            capacity,
        })
    }
}

/// Reserves at least `minimum_capacity` elements in an owning buffer.
///
/// # Safety
///
/// `buffer` must be a live header produced by this runtime. Its existing
/// pointer, length and capacity must not be modified concurrently.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_reserve(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    minimum_capacity: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    // SAFETY: caller guarantees the pointer is a live exclusive Buffer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    if let Err(status) = validate_element_layout(element_size, alignment) {
        return status;
    }
    if minimum_capacity <= buffer.capacity {
        return BufferStatus::Ok;
    }
    let new_layout = match buffer_layout(element_size, alignment, minimum_capacity) {
        Ok(layout) => layout,
        Err(status) => return status,
    };
    let resized = if buffer.capacity == 0 {
        // SAFETY: `new_layout` is validated and nonzero.
        unsafe { alloc(new_layout) }
    } else {
        let old_layout = match buffer_layout(element_size, alignment, buffer.capacity) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        // SAFETY: caller guarantees provenance and exact old layout.
        unsafe { realloc(buffer.pointer.cast(), old_layout, new_layout.size()) }
    };
    if resized.is_null() {
        return BufferStatus::OutOfMemory;
    }
    buffer.pointer = resized.cast();
    buffer.capacity = minimum_capacity;
    BufferStatus::Ok
}

/// Changes the logical element length without allocating.
///
/// `new_length` must not exceed the current capacity. Call [`buffer_reserve`]
/// explicitly when growth is required, which keeps `@noalloc` behavior
/// visible to the caller.
///
/// # Safety
///
/// Any elements newly exposed by an increase must already be initialized by
/// typed compiler/runtime code before a safe read or drop observes the length.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_resize(buffer: *mut Buffer, new_length: u64) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    // SAFETY: this function only accesses the caller-owned header. The caller
    // must provide a live exclusive pointer as documented by the ABI wrapper.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    if new_length > buffer.capacity {
        BufferStatus::OutOfBounds
    } else {
        buffer.length = new_length;
        BufferStatus::Ok
    }
}

/// Clears a copy-safe buffer by publishing logical length zero.
///
/// This intentionally does not destroy element storage.  The compiler only
/// exposes the builtin for copy-safe `Buffer<T>` elements; owning nested
/// values require an explicit move-aware resize operation.
///
/// # Safety
///
/// `buffer` must be a live exclusive header accepted by [`buffer_resize`].
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_clear(buffer: *mut Buffer) -> BufferStatus {
    // SAFETY: the caller contract is identical to `buffer_resize` with a
    // constant zero length.
    unsafe { buffer_resize(buffer, 0) }
}

/// Changes the logical length of an owning nested `Buffer<Buffer<U>>`.
///
/// Growth reserves descriptor slots and zero-initializes the newly exposed
/// slots. Shrinking recursively destroys every nested descriptor leaving the
/// logical range before publishing the new length.
///
/// # Safety
///
/// `buffer` must be a live exclusive outer header. `element_size` and
/// `alignment` must describe the native `Buffer` descriptor, while `depth`
/// and the leaf layout must describe every initialized nested chain.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_resize_move(
    buffer: *mut Buffer,
    new_length: u64,
    element_size: u64,
    alignment: u64,
    depth: u64,
    leaf_element_size: u64,
    leaf_alignment: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    if element_size != size_of::<Buffer>() as u64 {
        return BufferStatus::InvalidSize;
    }
    if alignment != align_of::<Buffer>() as u64 {
        return BufferStatus::InvalidAlignment;
    }
    if depth == 0 {
        return BufferStatus::InvalidSize;
    }
    if let Err(status) = validate_element_layout(leaf_element_size, leaf_alignment) {
        return status;
    }
    // SAFETY: caller guarantees a live exclusive Buffer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    let old_length = buffer.length;
    if new_length > old_length {
        let status = unsafe { buffer_reserve(buffer, element_size, alignment, new_length) };
        if status != BufferStatus::Ok {
            return status;
        }
        let byte_offset = (old_length as usize) * size_of::<Buffer>();
        let byte_length = ((new_length - old_length) as usize) * size_of::<Buffer>();
        // SAFETY: reserve established capacity for the full descriptor range;
        // the destination is within that allocation and is intentionally zeroed.
        unsafe { ptr::write_bytes(buffer.pointer.cast::<u8>().add(byte_offset), 0, byte_length) };
    } else if new_length < old_length {
        for index in new_length..old_length {
            // SAFETY: every slot in the initialized range is a native Buffer.
            let nested = unsafe { buffer.pointer.cast::<Buffer>().add(index as usize) };
            let status = if depth == 1 {
                // SAFETY: the nested descriptor contains the final leaf type.
                unsafe { buffer_destroy(nested, leaf_element_size, leaf_alignment) }
            } else {
                // SAFETY: one descriptor edge is consumed by this outer slot.
                unsafe {
                    buffer_destroy_nested_buffer_recursive(
                        nested,
                        size_of::<Buffer>() as u64,
                        align_of::<Buffer>() as u64,
                        depth - 1,
                        leaf_element_size,
                        leaf_alignment,
                    )
                }
            };
            if status != BufferStatus::Ok {
                return status;
            }
        }
    }
    buffer.length = new_length;
    BufferStatus::Ok
}

/// Changes the logical length of an owning nested Buffer chain whose final
/// leaf is OwnedString. Removed chains release UTF-8 payloads recursively.
///
/// # Safety
///
/// `buffer` must be a live exclusive nested-buffer header. `element_size`,
/// `alignment`, `depth`, and the leaf string layout must describe the actual
/// allocation and initialized descriptors.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_resize_move_nested_owned_string(
    buffer: *mut Buffer,
    new_length: u64,
    element_size: u64,
    alignment: u64,
    depth: u64,
    string_element_size: u64,
    string_alignment: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    if depth == 0 {
        return BufferStatus::InvalidSize;
    }
    if element_size != size_of::<Buffer>() as u64 {
        return BufferStatus::InvalidSize;
    }
    if alignment != align_of::<Buffer>() as u64 {
        return BufferStatus::InvalidAlignment;
    }
    if string_element_size != size_of::<Utf8String>() as u64 {
        return BufferStatus::InvalidSize;
    }
    if string_alignment != align_of::<Utf8String>() as u64 {
        return BufferStatus::InvalidAlignment;
    }
    // SAFETY: caller guarantees a live exclusive Buffer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    let old_length = buffer.length;
    if new_length > old_length {
        let status = unsafe { buffer_reserve(buffer, element_size, alignment, new_length) };
        if status != BufferStatus::Ok {
            return status;
        }
        let byte_offset = match old_length.checked_mul(size_of::<Buffer>() as u64) {
            Some(offset) => offset,
            None => return BufferStatus::SizeOverflow,
        };
        let byte_length = match (new_length - old_length).checked_mul(size_of::<Buffer>() as u64) {
            Some(length) => length,
            None => return BufferStatus::SizeOverflow,
        };
        // SAFETY: reserve established the destination descriptor range.
        unsafe {
            ptr::write_bytes(
                buffer.pointer.cast::<u8>().add(byte_offset as usize),
                0,
                byte_length as usize,
            )
        };
    } else if new_length < old_length {
        for index in new_length..old_length {
            // SAFETY: every removed outer slot is an initialized Buffer.
            let nested = unsafe { buffer.pointer.cast::<Buffer>().add(index as usize) };
            let status = if depth == 1 {
                unsafe {
                    buffer_destroy_owned_string(nested, string_element_size, string_alignment)
                }
            } else {
                unsafe {
                    buffer_destroy_nested_owned_string(
                        nested,
                        size_of::<Buffer>() as u64,
                        align_of::<Buffer>() as u64,
                        depth - 1,
                        string_element_size,
                        string_alignment,
                    )
                }
            };
            if status != BufferStatus::Ok {
                return status;
            }
        }
    }
    buffer.length = new_length;
    BufferStatus::Ok
}

/// Clears an owning nested `Buffer<Buffer<U>>` and recursively releases every
/// initialized nested descriptor without releasing the outer allocation.
///
/// This is the move-aware counterpart of [`buffer_clear`].
///
/// # Safety
///
/// `buffer` must be a live exclusive nested-buffer header, and the supplied
/// descriptor/depth/leaf layout must match the initialized allocation.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_clear_move(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    depth: u64,
    leaf_element_size: u64,
    leaf_alignment: u64,
) -> BufferStatus {
    // SAFETY: the caller supplies the same validated nested layout as
    // `buffer_resize_move`; zero is the only requested new length.
    unsafe {
        buffer_resize_move(
            buffer,
            0,
            element_size,
            alignment,
            depth,
            leaf_element_size,
            leaf_alignment,
        )
    }
}

/// Clears an owning nested Buffer chain whose final leaf is OwnedString.
///
/// # Safety
///
/// `buffer` must be a live exclusive nested-buffer header, and the supplied
/// nested depth and string layout must match the initialized allocation.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_clear_move_nested_owned_string(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    depth: u64,
    string_element_size: u64,
    string_alignment: u64,
) -> BufferStatus {
    // SAFETY: the caller supplies the same validated layout as the resize
    // helper; zero is the only requested new length.
    unsafe {
        buffer_resize_move_nested_owned_string(
            buffer,
            0,
            element_size,
            alignment,
            depth,
            string_element_size,
            string_alignment,
        )
    }
}

/// Changes the logical length of an owning `Buffer<OwnedString>`.
///
/// Growth reserves and zero-initializes new string descriptors. Shrinking
/// destroys every removed UTF-8 allocation before publishing the new length.
///
/// # Safety
///
/// `buffer` must be a live exclusive `Buffer<OwnedString>` header and the
/// supplied element layout must match [`Utf8String`].
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_resize_move_owned_string(
    buffer: *mut Buffer,
    new_length: u64,
    element_size: u64,
    alignment: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    if element_size != size_of::<Utf8String>() as u64 {
        return BufferStatus::InvalidSize;
    }
    if alignment != align_of::<Utf8String>() as u64 {
        return BufferStatus::InvalidAlignment;
    }
    // SAFETY: caller guarantees a live exclusive Buffer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    let old_length = buffer.length;
    if new_length > old_length {
        let status = unsafe { buffer_reserve(buffer, element_size, alignment, new_length) };
        if status != BufferStatus::Ok {
            return status;
        }
        let byte_offset = match old_length.checked_mul(element_size) {
            Some(offset) => offset,
            None => return BufferStatus::SizeOverflow,
        };
        let byte_length = match (new_length - old_length).checked_mul(element_size) {
            Some(length) => length,
            None => return BufferStatus::SizeOverflow,
        };
        // SAFETY: reserve established capacity for the descriptor range.
        unsafe {
            ptr::write_bytes(
                buffer.pointer.cast::<u8>().add(byte_offset as usize),
                0,
                byte_length as usize,
            )
        };
    } else if new_length < old_length {
        for index in new_length..old_length {
            // SAFETY: every removed slot is an initialized UTF-8 descriptor.
            let string = unsafe { buffer.pointer.cast::<Utf8String>().add(index as usize) };
            let status = unsafe { string_destroy(string) };
            if status != StringStatus::Ok {
                return match status {
                    StringStatus::RuntimeNotInitialized => BufferStatus::RuntimeNotInitialized,
                    StringStatus::NullPointer => BufferStatus::NullPointer,
                    StringStatus::InvalidSize => BufferStatus::InvalidSize,
                    StringStatus::InvalidAlignment => BufferStatus::InvalidAlignment,
                    StringStatus::SizeOverflow => BufferStatus::SizeOverflow,
                    StringStatus::OutOfMemory => BufferStatus::OutOfMemory,
                    _ => BufferStatus::InvalidBuffer,
                };
            }
        }
    }
    buffer.length = new_length;
    BufferStatus::Ok
}

/// Clears an owning `Buffer<OwnedString>` without releasing its outer
/// allocation.
///
/// # Safety
///
/// `buffer` must be a live exclusive buffer header containing valid owned
/// string descriptors, and `element_size`/`alignment` must match its layout.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_clear_move_owned_string(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
) -> BufferStatus {
    // SAFETY: the caller supplies the same validated layout as the resize
    // helper; zero is the only requested new length.
    unsafe { buffer_resize_move_owned_string(buffer, 0, element_size, alignment) }
}

/// Releases an owning buffer and resets its header to empty.
///
/// # Safety
///
/// `buffer` must be a live exclusive header from this runtime, and
/// `element_size`/`alignment` must match the allocation used by create/reserve.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_destroy(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    // SAFETY: caller guarantees a live exclusive Buffer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    if let Err(status) = validate_element_layout(element_size, alignment) {
        return status;
    }
    if buffer.capacity != 0 {
        let layout = match buffer_layout(element_size, alignment, buffer.capacity) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        // SAFETY: caller guarantees pointer provenance and exact layout.
        unsafe { dealloc(buffer.pointer.cast(), layout) };
    }
    *buffer = Buffer::EMPTY;
    BufferStatus::Ok
}

/// Releases an owning `Buffer<OwnedString>` and destroys each initialized
/// string descriptor before releasing the outer allocation.
///
/// `OwnedString` has the same native three-word descriptor shape as `Buffer`,
/// but its elements own UTF-8 byte allocations and therefore cannot use the
/// ordinary byte-only `buffer_destroy` path.
///
/// # Safety
///
/// `buffer` must be a live exclusive `Buffer<OwnedString>` header and the
/// supplied element layout must match the target descriptor.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_destroy_owned_string(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if element_size != size_of::<Utf8String>() as u64 {
        return BufferStatus::InvalidSize;
    }
    if alignment != align_of::<Utf8String>() as u64 {
        return BufferStatus::InvalidAlignment;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    // SAFETY: caller guarantees a live exclusive Buffer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    for index in 0..buffer.length {
        // SAFETY: the validated allocation contains initialized, aligned
        // OwnedString descriptors for every logical element.
        let string = unsafe { buffer.pointer.cast::<Utf8String>().add(index as usize) };
        // SAFETY: each descriptor is owned by this outer buffer and is
        // destroyed exactly once before the outer allocation is released.
        let status = unsafe { string_destroy(string) };
        if status != StringStatus::Ok {
            return match status {
                StringStatus::RuntimeNotInitialized => BufferStatus::RuntimeNotInitialized,
                StringStatus::NullPointer => BufferStatus::NullPointer,
                StringStatus::InvalidSize => BufferStatus::InvalidSize,
                StringStatus::InvalidAlignment => BufferStatus::InvalidAlignment,
                StringStatus::SizeOverflow => BufferStatus::SizeOverflow,
                StringStatus::OutOfMemory => BufferStatus::OutOfMemory,
                _ => BufferStatus::InvalidBuffer,
            };
        }
    }
    // SAFETY: all element ownership has been released; the remaining
    // allocation is an ordinary Buffer<OwnedString> payload.
    unsafe { buffer_destroy(buffer, element_size, alignment) }
}

/// Releases an owning `Buffer<Buffer<U>>` and all initialized nested buffers.
///
/// The outer element must have the native `Buffer` descriptor layout. Nested
/// descriptors are destroyed before the outer allocation, so moving a buffer
/// into another buffer does not leak or double-free its allocation.
///
/// # Safety
///
/// `buffer` must be a live exclusive outer header from this runtime. Both
/// element layouts must match the generated descriptors and all initialized
/// nested elements must be valid owning buffers.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_destroy_nested_buffer(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    nested_element_size: u64,
    nested_alignment: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    if element_size != size_of::<Buffer>() as u64 {
        return BufferStatus::InvalidSize;
    }
    if alignment != align_of::<Buffer>() as u64 {
        return BufferStatus::InvalidAlignment;
    }
    if let Err(status) = validate_element_layout(nested_element_size, nested_alignment) {
        return status;
    }
    // SAFETY: caller guarantees a live exclusive outer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    for index in 0..buffer.length {
        // SAFETY: the outer layout check makes every initialized element a
        // properly aligned Buffer descriptor within the allocation.
        let nested = unsafe { (buffer.pointer.cast::<Buffer>()).add(index as usize) };
        // SAFETY: nested descriptors are owned by the outer buffer and are
        // destroyed exactly once before the outer allocation is released.
        let status = unsafe { buffer_destroy(nested, nested_element_size, nested_alignment) };
        if status != BufferStatus::Ok {
            return status;
        }
    }
    if buffer.capacity != 0 {
        let layout = match buffer_layout(element_size, alignment, buffer.capacity) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        // SAFETY: caller guarantees pointer provenance and exact outer layout.
        unsafe { dealloc(buffer.pointer.cast(), layout) };
    }
    *buffer = Buffer::EMPTY;
    BufferStatus::Ok
}

/// Releases an owning buffer with an arbitrary nested `Buffer` descriptor
/// depth. `depth` counts nested owning edges below the outer descriptor and
/// `leaf_element_size`/`leaf_alignment` describe the final non-buffer element.
///
/// # Safety
///
/// `buffer` must be a live exclusive outer header. Every initialized nested
/// descriptor must use the native `Buffer` layout and the leaf allocations
/// must use the supplied final element layout.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_destroy_nested_buffer_recursive(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    depth: u64,
    leaf_element_size: u64,
    leaf_alignment: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    if depth == 0 {
        return BufferStatus::InvalidSize;
    }
    if element_size != size_of::<Buffer>() as u64 {
        return BufferStatus::InvalidSize;
    }
    if alignment != align_of::<Buffer>() as u64 {
        return BufferStatus::InvalidAlignment;
    }
    if let Err(status) = validate_element_layout(leaf_element_size, leaf_alignment) {
        return status;
    }
    // SAFETY: caller guarantees a live exclusive outer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    for index in 0..buffer.length {
        // SAFETY: every initialized outer slot is a native Buffer descriptor.
        let nested = unsafe { buffer.pointer.cast::<Buffer>().add(index as usize) };
        let status = if depth == 1 {
            // SAFETY: the nested descriptor is the final Buffer<leaf> level.
            unsafe { buffer_destroy(nested, leaf_element_size, leaf_alignment) }
        } else {
            // SAFETY: the nested descriptor remains an owning Buffer chain.
            unsafe {
                buffer_destroy_nested_buffer_recursive(
                    nested,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    depth - 1,
                    leaf_element_size,
                    leaf_alignment,
                )
            }
        };
        if status != BufferStatus::Ok {
            return status;
        }
    }
    if buffer.capacity != 0 {
        let layout = match buffer_layout(element_size, alignment, buffer.capacity) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        // SAFETY: caller guarantees pointer provenance and exact outer layout.
        unsafe { dealloc(buffer.pointer.cast(), layout) };
    }
    *buffer = Buffer::EMPTY;
    BufferStatus::Ok
}

/// Releases an owning nested Buffer chain whose final leaf is OwnedString.
/// Unlike the copy-safe nested destructor, this path destroys every UTF-8
/// allocation in the leaf descriptors before freeing nested buffers.
///
/// # Safety
///
/// `buffer` must be a live exclusive nested-buffer header. The supplied
/// layout and depth must match the allocation and initialized descriptors.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_destroy_nested_owned_string(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    depth: u64,
    string_element_size: u64,
    string_alignment: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    if depth == 0 {
        return BufferStatus::InvalidSize;
    }
    if element_size != size_of::<Buffer>() as u64 {
        return BufferStatus::InvalidSize;
    }
    if alignment != align_of::<Buffer>() as u64 {
        return BufferStatus::InvalidAlignment;
    }
    if string_element_size != size_of::<Utf8String>() as u64 {
        return BufferStatus::InvalidSize;
    }
    if string_alignment != align_of::<Utf8String>() as u64 {
        return BufferStatus::InvalidAlignment;
    }
    // SAFETY: caller guarantees a live exclusive outer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    for index in 0..buffer.length {
        // SAFETY: every initialized outer slot is a native Buffer descriptor.
        let nested = unsafe { buffer.pointer.cast::<Buffer>().add(index as usize) };
        let status = if depth == 1 {
            // SAFETY: the final nested descriptor is Buffer<OwnedString>.
            unsafe { buffer_destroy_owned_string(nested, string_element_size, string_alignment) }
        } else {
            // SAFETY: one nested Buffer edge is consumed recursively.
            unsafe {
                buffer_destroy_nested_owned_string(
                    nested,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    depth - 1,
                    string_element_size,
                    string_alignment,
                )
            }
        };
        if status != BufferStatus::Ok {
            return status;
        }
    }
    if buffer.capacity != 0 {
        let layout = match buffer_layout(element_size, alignment, buffer.capacity) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        // SAFETY: nested ownership has been released before the outer
        // allocation is deallocated.
        unsafe { dealloc(buffer.pointer.cast(), layout) };
    }
    *buffer = Buffer::EMPTY;
    BufferStatus::Ok
}

/// Releases a nested owning buffer whose final leaf is a record with owning
/// Buffer fields. `depth` counts Buffer descriptors below the outer buffer;
/// when it reaches one, the record field table is applied to the nested
/// `Buffer<Record>` allocation.
///
/// # Safety
///
/// `buffer` must be a live exclusive outer header. Every nested descriptor
/// must use the native Buffer layout, and `fields` must describe initialized
/// owning Buffer fields inside the final record element.
#[must_use]
#[allow(unsafe_code, clippy::manual_is_multiple_of, clippy::too_many_arguments)]
pub unsafe fn buffer_destroy_nested_record_fields(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    depth: u64,
    record_element_size: u64,
    record_alignment: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    if depth == 0 {
        return BufferStatus::InvalidSize;
    }
    if element_size != size_of::<Buffer>() as u64 {
        return BufferStatus::InvalidSize;
    }
    if alignment != align_of::<Buffer>() as u64 {
        return BufferStatus::InvalidAlignment;
    }
    if let Err(status) = validate_element_layout(record_element_size, record_alignment) {
        return status;
    }
    if let Err(status) =
        unsafe { validate_enum_carrier_fields(fields, field_count, record_element_size) }
    {
        return status;
    }
    // SAFETY: caller guarantees a live exclusive outer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    for index in 0..buffer.length {
        // SAFETY: the outer layout check makes every initialized element a
        // properly aligned Buffer descriptor within the allocation.
        let nested = unsafe { buffer.pointer.cast::<Buffer>().add(index as usize) };
        let status = if depth == 1 {
            // SAFETY: the nested descriptor is the final Buffer<Record> level.
            unsafe {
                buffer_destroy_record_fields(
                    nested,
                    record_element_size,
                    record_alignment,
                    fields,
                    field_count,
                )
            }
        } else {
            // SAFETY: the nested descriptor remains an owning Buffer chain.
            unsafe {
                buffer_destroy_nested_record_fields(
                    nested,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    depth - 1,
                    record_element_size,
                    record_alignment,
                    fields,
                    field_count,
                )
            }
        };
        if status != BufferStatus::Ok {
            return status;
        }
    }
    if buffer.capacity != 0 {
        let layout = match buffer_layout(element_size, alignment, buffer.capacity) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        // SAFETY: caller guarantees pointer provenance and exact outer layout.
        unsafe { dealloc(buffer.pointer.cast(), layout) };
    }
    *buffer = Buffer::EMPTY;
    BufferStatus::Ok
}

/// Changes the logical length of an owning nested buffer whose final leaf is
/// a record with owning Buffer fields.
///
/// Growth reserves descriptor slots and zero-initializes newly exposed slots.
/// Shrinking recursively destroys every nested record chain that leaves the
/// logical range before publishing the new length.
///
/// # Safety
///
/// `buffer` must be a live exclusive outer header. The nested descriptor
/// layout, record layout and field table must describe initialized chains
/// accepted by [`buffer_destroy_nested_record_fields`].
#[must_use]
#[allow(unsafe_code, clippy::manual_is_multiple_of, clippy::too_many_arguments)]
pub unsafe fn buffer_resize_move_nested_record_fields(
    buffer: *mut Buffer,
    new_length: u64,
    element_size: u64,
    alignment: u64,
    depth: u64,
    record_element_size: u64,
    record_alignment: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    if depth == 0 {
        return BufferStatus::InvalidSize;
    }
    if element_size != size_of::<Buffer>() as u64 {
        return BufferStatus::InvalidSize;
    }
    if alignment != align_of::<Buffer>() as u64 {
        return BufferStatus::InvalidAlignment;
    }
    if let Err(status) = validate_element_layout(record_element_size, record_alignment) {
        return status;
    }
    if let Err(status) =
        unsafe { validate_enum_carrier_fields(fields, field_count, record_element_size) }
    {
        return status;
    }
    // SAFETY: caller guarantees a live exclusive outer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    let old_length = buffer.length;
    if new_length > old_length {
        let status = unsafe { buffer_reserve(buffer, element_size, alignment, new_length) };
        if status != BufferStatus::Ok {
            return status;
        }
        let byte_offset = match old_length.checked_mul(element_size) {
            Some(offset) => offset,
            None => return BufferStatus::SizeOverflow,
        };
        let byte_length = match (new_length - old_length).checked_mul(element_size) {
            Some(length) => length,
            None => return BufferStatus::SizeOverflow,
        };
        // SAFETY: reserve established capacity for the full descriptor range;
        // the newly exposed slots are intentionally zero-initialized.
        unsafe {
            ptr::write_bytes(
                buffer.pointer.cast::<u8>().add(byte_offset as usize),
                0,
                byte_length as usize,
            )
        };
    } else if new_length < old_length {
        for index in new_length..old_length {
            // SAFETY: every slot in the initialized range is a native Buffer
            // descriptor because the outer element layout is fixed above.
            let nested = unsafe { buffer.pointer.cast::<Buffer>().add(index as usize) };
            let status = if depth == 1 {
                unsafe {
                    buffer_destroy_record_fields(
                        nested,
                        record_element_size,
                        record_alignment,
                        fields,
                        field_count,
                    )
                }
            } else {
                unsafe {
                    buffer_destroy_nested_record_fields(
                        nested,
                        size_of::<Buffer>() as u64,
                        align_of::<Buffer>() as u64,
                        depth - 1,
                        record_element_size,
                        record_alignment,
                        fields,
                        field_count,
                    )
                }
            };
            if status != BufferStatus::Ok {
                return status;
            }
        }
    }
    buffer.length = new_length;
    BufferStatus::Ok
}

/// Changes the logical length of an owning buffer whose elements are direct
/// `@repr(C)` records with owning Buffer or OwnedString fields.
///
/// Growth reserves record slots and zero-initializes newly exposed storage.
/// Shrinking destroys each removed record's field-table Buffers before the
/// new length is published.
///
/// # Safety
///
/// `buffer` must be a live exclusive header. `element_size`/`alignment` and
/// `fields` must describe initialized records accepted by
/// [`carrier_destroy_record_fields`].
#[must_use]
#[allow(unsafe_code, clippy::manual_is_multiple_of)]
pub unsafe fn buffer_resize_move_record_fields(
    buffer: *mut Buffer,
    new_length: u64,
    element_size: u64,
    alignment: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    if let Err(status) = validate_element_layout(element_size, alignment) {
        return status;
    }
    if let Err(status) = unsafe { validate_enum_carrier_fields(fields, field_count, element_size) }
    {
        return status;
    }
    // SAFETY: caller guarantees a live exclusive Buffer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    let old_length = buffer.length;
    if new_length > old_length {
        let status = unsafe { buffer_reserve(buffer, element_size, alignment, new_length) };
        if status != BufferStatus::Ok {
            return status;
        }
        let byte_offset = match old_length.checked_mul(element_size) {
            Some(offset) => offset,
            None => return BufferStatus::SizeOverflow,
        };
        let byte_length = match (new_length - old_length).checked_mul(element_size) {
            Some(length) => length,
            None => return BufferStatus::SizeOverflow,
        };
        // SAFETY: reserve established the full record range.
        unsafe {
            ptr::write_bytes(
                buffer.pointer.cast::<u8>().add(byte_offset as usize),
                0,
                byte_length as usize,
            )
        };
    } else if new_length < old_length {
        for index in new_length..old_length {
            // SAFETY: every initialized element is a record with the declared
            // native layout and field table.
            let record = unsafe {
                buffer
                    .pointer
                    .cast::<u8>()
                    .add((index * element_size) as usize)
            };
            let status =
                unsafe { carrier_destroy_record_fields(record, element_size, fields, field_count) };
            if status != BufferStatus::Ok {
                return status;
            }
        }
    }
    buffer.length = new_length;
    BufferStatus::Ok
}

/// Releases an owning buffer whose element is an inline `Option`/`Result`
/// carrier with exactly one `Buffer` payload. Only the selected tag owns a
/// descriptor; the other variant is copy-safe and needs no cleanup.
///
/// # Safety
///
/// `buffer` must be a live exclusive header. `payload_offset` must point to a
/// properly aligned Buffer descriptor inside every initialized carrier slot,
/// and the supplied leaf layout must match the descriptor chain.
#[must_use]
#[allow(
    unsafe_code,
    clippy::too_many_arguments,
    clippy::manual_is_multiple_of,
    clippy::unnecessary_cast
)]
pub unsafe fn buffer_destroy_carrier_buffer(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    payload_offset: u64,
    payload_variant: u64,
    depth: u64,
    leaf_element_size: u64,
    leaf_alignment: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    if depth == 0 {
        return BufferStatus::InvalidSize;
    }
    if let Err(status) = validate_element_layout(element_size, alignment) {
        return status;
    }
    let Some(payload_end) = payload_offset.checked_add(size_of::<Buffer>() as u64) else {
        return BufferStatus::InvalidAlignment;
    };
    if payload_offset % align_of::<Buffer>() as u64 != 0 || payload_end > element_size {
        return BufferStatus::InvalidAlignment;
    }
    if let Err(status) = validate_element_layout(leaf_element_size, leaf_alignment) {
        return status;
    }
    // SAFETY: caller guarantees a live exclusive outer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    for index in 0..buffer.length {
        let slot_offset = match (index as u64).checked_mul(element_size) {
            Some(offset) => offset,
            None => return BufferStatus::SizeOverflow,
        };
        // SAFETY: descriptor validity and carrier layout checks keep this
        // offset within the initialized carrier allocation.
        let carrier = unsafe { buffer.pointer.cast::<u8>().add(slot_offset as usize) };
        // SAFETY: the carrier tag is the first four bytes of the lowered enum.
        let tag = unsafe { carrier.cast::<u32>().read_unaligned() } as u64;
        if tag == payload_variant {
            // SAFETY: payload_offset was validated against the carrier size
            // and Buffer alignment above.
            let nested = unsafe { carrier.add(payload_offset as usize).cast::<Buffer>() };
            let status = if depth == 1 {
                // SAFETY: selected carrier payload is a live owning Buffer.
                unsafe { buffer_destroy(nested, leaf_element_size, leaf_alignment) }
            } else {
                // SAFETY: selected payload is a nested descriptor chain.
                unsafe {
                    buffer_destroy_nested_buffer_recursive(
                        nested,
                        size_of::<Buffer>() as u64,
                        align_of::<Buffer>() as u64,
                        depth - 1,
                        leaf_element_size,
                        leaf_alignment,
                    )
                }
            };
            if status != BufferStatus::Ok {
                return status;
            }
        }
    }
    if buffer.capacity != 0 {
        let layout = match buffer_layout(element_size, alignment, buffer.capacity) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        // SAFETY: caller guarantees pointer provenance and exact outer layout.
        unsafe { dealloc(buffer.pointer.cast(), layout) };
    }
    *buffer = Buffer::EMPTY;
    BufferStatus::Ok
}

/// Releases a standalone Option/Result carrier or named `@repr(C)` enum with
/// one owning Buffer payload. The carrier itself is caller-owned storage and
/// is not freed.
///
/// # Safety
///
/// `carrier` must point to initialized carrier storage whose descriptor and
/// leaf layouts match the supplied arguments.
#[must_use]
#[allow(unsafe_code, clippy::manual_is_multiple_of)]
pub unsafe fn carrier_destroy_buffer(
    carrier: *mut u8,
    element_size: u64,
    payload_offset: u64,
    payload_variant: u64,
    depth: u64,
    leaf_element_size: u64,
    leaf_alignment: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if carrier.is_null() || depth == 0 {
        return BufferStatus::NullPointer;
    }
    if let Err(status) = validate_element_layout(element_size, align_of::<Buffer>() as u64) {
        return status;
    }
    let Some(payload_end) = payload_offset.checked_add(size_of::<Buffer>() as u64) else {
        return BufferStatus::InvalidAlignment;
    };
    if payload_offset % align_of::<Buffer>() as u64 != 0 || payload_end > element_size {
        return BufferStatus::InvalidAlignment;
    }
    if let Err(status) = validate_element_layout(leaf_element_size, leaf_alignment) {
        return status;
    }
    // SAFETY: caller guarantees a live carrier with the lowered tag layout.
    let tag = unsafe { carrier.cast::<u32>().read_unaligned() } as u64;
    if tag == payload_variant {
        // SAFETY: payload offset was validated against the carrier layout.
        let nested = unsafe { carrier.add(payload_offset as usize).cast::<Buffer>() };
        let status = if depth == 1 {
            // SAFETY: selected payload is a live owning Buffer.
            unsafe { buffer_destroy(nested, leaf_element_size, leaf_alignment) }
        } else {
            // SAFETY: selected payload is a nested descriptor chain.
            unsafe {
                buffer_destroy_nested_buffer_recursive(
                    nested,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    depth - 1,
                    leaf_element_size,
                    leaf_alignment,
                )
            }
        };
        if status != BufferStatus::Ok {
            return status;
        }
    }
    // SAFETY: carrier storage is caller-owned and exactly element_size bytes.
    unsafe { ptr::write_bytes(carrier, 0, element_size as usize) };
    BufferStatus::Ok
}

/// Releases a buffer whose `Result` carrier has two owning `Buffer` branches.
/// The tag selects exactly one branch; the other descriptor is not touched.
///
/// # Safety
///
/// `buffer` must be a live exclusive header. Both branch layouts must describe
/// the initialized descriptor chains at the common carrier payload offset.
#[must_use]
#[allow(
    unsafe_code,
    clippy::too_many_arguments,
    clippy::manual_is_multiple_of,
    clippy::unnecessary_cast
)]
pub unsafe fn buffer_destroy_multi_carrier_buffer(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    payload_offset: u64,
    first_variant: u64,
    first_depth: u64,
    first_leaf_element_size: u64,
    first_leaf_alignment: u64,
    second_variant: u64,
    second_depth: u64,
    second_leaf_element_size: u64,
    second_leaf_alignment: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null()
        || first_depth == 0
        || second_depth == 0
        || first_variant > 1
        || second_variant > 1
        || first_variant == second_variant
    {
        return BufferStatus::InvalidSize;
    }
    if let Err(status) = validate_element_layout(element_size, alignment) {
        return status;
    }
    let Some(payload_end) = payload_offset.checked_add(size_of::<Buffer>() as u64) else {
        return BufferStatus::InvalidAlignment;
    };
    if payload_offset % align_of::<Buffer>() as u64 != 0 || payload_end > element_size {
        return BufferStatus::InvalidAlignment;
    }
    if let Err(status) = validate_element_layout(first_leaf_element_size, first_leaf_alignment) {
        return status;
    }
    if let Err(status) = validate_element_layout(second_leaf_element_size, second_leaf_alignment) {
        return status;
    }
    // SAFETY: caller guarantees a live exclusive outer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    for index in 0..buffer.length {
        let slot_offset = match (index as u64).checked_mul(element_size) {
            Some(offset) => offset,
            None => return BufferStatus::SizeOverflow,
        };
        // SAFETY: descriptor validity and carrier layout checks keep this
        // offset within the initialized carrier allocation.
        let carrier = unsafe { buffer.pointer.cast::<u8>().add(slot_offset as usize) };
        // SAFETY: the carrier tag is the first four bytes of the lowered enum.
        let tag = unsafe { carrier.cast::<u32>().read_unaligned() } as u64;
        let (variant, depth, leaf_size, leaf_alignment) = if tag == first_variant {
            (
                first_variant,
                first_depth,
                first_leaf_element_size,
                first_leaf_alignment,
            )
        } else if tag == second_variant {
            (
                second_variant,
                second_depth,
                second_leaf_element_size,
                second_leaf_alignment,
            )
        } else {
            return BufferStatus::InvalidBuffer;
        };
        if tag == variant {
            // SAFETY: payload_offset was validated against the carrier size
            // and Buffer alignment above.
            let nested = unsafe { carrier.add(payload_offset as usize).cast::<Buffer>() };
            let status = if depth == 1 {
                // SAFETY: selected carrier payload is a live owning Buffer.
                unsafe { buffer_destroy(nested, leaf_size, leaf_alignment) }
            } else {
                // SAFETY: selected payload is a nested descriptor chain.
                unsafe {
                    buffer_destroy_nested_buffer_recursive(
                        nested,
                        size_of::<Buffer>() as u64,
                        align_of::<Buffer>() as u64,
                        depth - 1,
                        leaf_size,
                        leaf_alignment,
                    )
                }
            };
            if status != BufferStatus::Ok {
                return status;
            }
        }
    }
    if buffer.capacity != 0 {
        let layout = match buffer_layout(element_size, alignment, buffer.capacity) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        // SAFETY: caller guarantees pointer provenance and exact outer layout.
        unsafe { dealloc(buffer.pointer.cast(), layout) };
    }
    *buffer = Buffer::EMPTY;
    BufferStatus::Ok
}

/// Releases a standalone `Result` carrier with two owning Buffer branches.
/// The caller-owned carrier storage is reset but not deallocated.
///
/// # Safety
///
/// `carrier` must point to initialized carrier storage whose descriptor and
/// leaf layouts match the supplied arguments.
#[must_use]
#[allow(unsafe_code, clippy::too_many_arguments, clippy::manual_is_multiple_of)]
pub unsafe fn carrier_destroy_multi_buffer(
    carrier: *mut u8,
    element_size: u64,
    payload_offset: u64,
    first_variant: u64,
    first_depth: u64,
    first_leaf_element_size: u64,
    first_leaf_alignment: u64,
    second_variant: u64,
    second_depth: u64,
    second_leaf_element_size: u64,
    second_leaf_alignment: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if carrier.is_null()
        || first_depth == 0
        || second_depth == 0
        || first_variant > 1
        || second_variant > 1
        || first_variant == second_variant
    {
        return BufferStatus::NullPointer;
    }
    if let Err(status) = validate_element_layout(element_size, align_of::<Buffer>() as u64) {
        return status;
    }
    let Some(payload_end) = payload_offset.checked_add(size_of::<Buffer>() as u64) else {
        return BufferStatus::InvalidAlignment;
    };
    if payload_offset % align_of::<Buffer>() as u64 != 0 || payload_end > element_size {
        return BufferStatus::InvalidAlignment;
    }
    if let Err(status) = validate_element_layout(first_leaf_element_size, first_leaf_alignment) {
        return status;
    }
    if let Err(status) = validate_element_layout(second_leaf_element_size, second_leaf_alignment) {
        return status;
    }
    // SAFETY: caller guarantees a live carrier with the lowered tag layout.
    let tag = unsafe { carrier.cast::<u32>().read_unaligned() } as u64;
    let (variant, depth, leaf_size, leaf_alignment) = if tag == first_variant {
        (
            first_variant,
            first_depth,
            first_leaf_element_size,
            first_leaf_alignment,
        )
    } else if tag == second_variant {
        (
            second_variant,
            second_depth,
            second_leaf_element_size,
            second_leaf_alignment,
        )
    } else {
        return BufferStatus::InvalidBuffer;
    };
    if tag == variant {
        // SAFETY: payload offset was validated against the carrier layout.
        let nested = unsafe { carrier.add(payload_offset as usize).cast::<Buffer>() };
        let status = if depth == 1 {
            // SAFETY: selected payload is a live owning Buffer.
            unsafe { buffer_destroy(nested, leaf_size, leaf_alignment) }
        } else {
            // SAFETY: selected payload is a nested descriptor chain.
            unsafe {
                buffer_destroy_nested_buffer_recursive(
                    nested,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    depth - 1,
                    leaf_size,
                    leaf_alignment,
                )
            }
        };
        if status != BufferStatus::Ok {
            return status;
        }
    }
    // SAFETY: carrier storage is caller-owned and exactly element_size bytes.
    unsafe { ptr::write_bytes(carrier, 0, element_size as usize) };
    BufferStatus::Ok
}

/// Releases a buffer whose named `@repr(C)` enum has multiple owning Buffer
/// variants. The branch table maps tags to descriptor depth and leaf layout;
/// tags not present in the table are copy-only variants.
///
/// # Safety
///
/// `buffer` must be a live exclusive header. `branches` must point to a valid
/// immutable table of `branch_count` descriptors for the duration of this
/// call, and each selected payload must be initialized at `payload_offset`.
#[must_use]
#[allow(
    unsafe_code,
    clippy::too_many_arguments,
    clippy::manual_is_multiple_of,
    clippy::unnecessary_cast
)]
pub unsafe fn buffer_destroy_enum_carrier_buffer(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    payload_offset: u64,
    branches: *const CarrierDropBranchAbi,
    branch_count: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    if let Err(status) = unsafe { validate_enum_carrier_branches(branches, branch_count) } {
        return status;
    }
    if let Err(status) = validate_element_layout(element_size, alignment) {
        return status;
    }
    let Some(payload_end) = payload_offset.checked_add(size_of::<Buffer>() as u64) else {
        return BufferStatus::InvalidAlignment;
    };
    if payload_offset % align_of::<Buffer>() as u64 != 0 || payload_end > element_size {
        return BufferStatus::InvalidAlignment;
    }
    // SAFETY: caller guarantees a live exclusive outer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    for index in 0..buffer.length {
        let slot_offset = match (index as u64).checked_mul(element_size) {
            Some(offset) => offset,
            None => return BufferStatus::SizeOverflow,
        };
        // SAFETY: descriptor validity and carrier layout checks keep this
        // offset within the initialized carrier allocation.
        let carrier = unsafe { buffer.pointer.cast::<u8>().add(slot_offset as usize) };
        // SAFETY: the carrier tag is the first four bytes of the lowered enum.
        let tag = unsafe { carrier.cast::<u32>().read_unaligned() } as u64;
        let Some(branch) = (unsafe { enum_carrier_branch_for_tag(branches, branch_count, tag) })
        else {
            continue;
        };
        // SAFETY: payload_offset was validated against the carrier size and
        // the branch table validated the selected leaf layout.
        let nested = unsafe { carrier.add(payload_offset as usize).cast::<Buffer>() };
        let status = if branch.depth == 1 {
            // SAFETY: selected carrier payload is a live owning Buffer.
            unsafe { buffer_destroy(nested, branch.leaf_element_size, branch.leaf_alignment) }
        } else {
            // SAFETY: selected payload is a nested descriptor chain.
            unsafe {
                buffer_destroy_nested_buffer_recursive(
                    nested,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    branch.depth - 1,
                    branch.leaf_element_size,
                    branch.leaf_alignment,
                )
            }
        };
        if status != BufferStatus::Ok {
            return status;
        }
    }
    if buffer.capacity != 0 {
        let layout = match buffer_layout(element_size, alignment, buffer.capacity) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        // SAFETY: caller guarantees pointer provenance and exact outer layout.
        unsafe { dealloc(buffer.pointer.cast(), layout) };
    }
    *buffer = Buffer::EMPTY;
    BufferStatus::Ok
}

/// Releases a standalone named enum carrier using a tag-to-layout branch
/// table. Copy-only variants are left untouched before the carrier storage is
/// reset.
///
/// # Safety
///
/// `carrier` must point to initialized storage of `element_size` bytes and
/// `branches` must remain valid for the duration of this call.
#[must_use]
#[allow(unsafe_code, clippy::manual_is_multiple_of)]
pub unsafe fn carrier_destroy_enum_buffer(
    carrier: *mut u8,
    element_size: u64,
    payload_offset: u64,
    branches: *const CarrierDropBranchAbi,
    branch_count: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if carrier.is_null() {
        return BufferStatus::NullPointer;
    }
    if let Err(status) = unsafe { validate_enum_carrier_branches(branches, branch_count) } {
        return status;
    }
    if let Err(status) = validate_element_layout(element_size, align_of::<Buffer>() as u64) {
        return status;
    }
    let Some(payload_end) = payload_offset.checked_add(size_of::<Buffer>() as u64) else {
        return BufferStatus::InvalidAlignment;
    };
    if payload_offset % align_of::<Buffer>() as u64 != 0 || payload_end > element_size {
        return BufferStatus::InvalidAlignment;
    }
    // SAFETY: caller guarantees a live carrier with the lowered tag layout.
    let tag = unsafe { carrier.cast::<u32>().read_unaligned() } as u64;
    if let Some(branch) = unsafe { enum_carrier_branch_for_tag(branches, branch_count, tag) } {
        // SAFETY: payload offset was validated against the carrier layout.
        let nested = unsafe { carrier.add(payload_offset as usize).cast::<Buffer>() };
        let status = if branch.depth == 1 {
            // SAFETY: selected payload is a live owning Buffer.
            unsafe { buffer_destroy(nested, branch.leaf_element_size, branch.leaf_alignment) }
        } else {
            // SAFETY: selected payload is a nested descriptor chain.
            unsafe {
                buffer_destroy_nested_buffer_recursive(
                    nested,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    branch.depth - 1,
                    branch.leaf_element_size,
                    branch.leaf_alignment,
                )
            }
        };
        if status != BufferStatus::Ok {
            return status;
        }
    }
    // SAFETY: carrier storage is caller-owned and exactly element_size bytes.
    unsafe { ptr::write_bytes(carrier, 0, element_size as usize) };
    BufferStatus::Ok
}

/// Releases every owning Buffer field selected by the tag of each named enum
/// carrier element. Multiple table entries may share a variant and therefore
/// describe a variant with more than one owning field.
///
/// # Safety
///
/// `buffer` must be a live exclusive header. `fields` must point to a valid
/// immutable table for the duration of this call, and each selected Buffer
/// descriptor must be initialized at its declared offset.
#[must_use]
#[allow(unsafe_code, clippy::manual_is_multiple_of)]
pub unsafe fn buffer_destroy_enum_carrier_fields(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    if let Err(status) = validate_element_layout(element_size, alignment) {
        return status;
    }
    if let Err(status) = unsafe { validate_enum_carrier_fields(fields, field_count, element_size) }
    {
        return status;
    }
    // SAFETY: caller guarantees a live exclusive outer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    for index in 0..buffer.length {
        let slot_offset = match index.checked_mul(element_size) {
            Some(offset) => offset,
            None => return BufferStatus::SizeOverflow,
        };
        // SAFETY: descriptor validity and carrier layout checks keep this
        // offset within the initialized carrier allocation.
        let carrier = unsafe { buffer.pointer.cast::<u8>().add(slot_offset as usize) };
        let tag = unsafe { carrier.cast::<u32>().read_unaligned() } as u64;
        for field_index in 0..field_count {
            // SAFETY: validation established the table bounds.
            let field = unsafe { fields.add(field_index as usize).read_unaligned() };
            if field.payload_variant != tag {
                continue;
            }
            // SAFETY: field offset and descriptor layout were validated
            // against the carrier element size.
            let status = unsafe { destroy_record_field(carrier, field) };
            if status != BufferStatus::Ok {
                return status;
            }
        }
    }
    if buffer.capacity != 0 {
        let layout = match buffer_layout(element_size, alignment, buffer.capacity) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        // SAFETY: caller guarantees pointer provenance and exact outer layout.
        unsafe { dealloc(buffer.pointer.cast(), layout) };
    }
    *buffer = Buffer::EMPTY;
    BufferStatus::Ok
}

/// Releases every owning Buffer field selected by a standalone named enum
/// carrier tag.
///
/// # Safety
///
/// `carrier` must point to initialized storage of `element_size` bytes and
/// `fields` must remain valid for the duration of this call.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn carrier_destroy_enum_fields(
    carrier: *mut u8,
    element_size: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if carrier.is_null() {
        return BufferStatus::NullPointer;
    }
    if let Err(status) = validate_element_layout(element_size, align_of::<Buffer>() as u64) {
        return status;
    }
    if let Err(status) = unsafe { validate_enum_carrier_fields(fields, field_count, element_size) }
    {
        return status;
    }
    let tag = unsafe { carrier.cast::<u32>().read_unaligned() } as u64;
    for field_index in 0..field_count {
        // SAFETY: validation established the table bounds.
        let field = unsafe { fields.add(field_index as usize).read_unaligned() };
        if field.payload_variant != tag {
            continue;
        }
        // SAFETY: field offset and descriptor layout were validated against
        // the carrier storage size.
        let status = unsafe { destroy_record_field(carrier, field) };
        if status != BufferStatus::Ok {
            return status;
        }
    }
    unsafe { ptr::write_bytes(carrier, 0, element_size as usize) };
    BufferStatus::Ok
}

/// Releases every owning Buffer or OwnedString field of each record element in an owning
/// buffer. The field table uses the same C ABI shape as enum field metadata,
/// with `u64::MAX` marking unconditional fields and other variants selecting
/// the active `Option`/`Result` carrier branch.
///
/// # Safety
///
/// `buffer` must be a live exclusive header. The element layout and field
/// table must describe initialized record storage and owning Buffer or
/// OwnedString fields.
#[must_use]
#[allow(unsafe_code, clippy::manual_is_multiple_of)]
pub unsafe fn buffer_destroy_record_fields(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    if let Err(status) = validate_element_layout(element_size, alignment) {
        return status;
    }
    if let Err(status) = unsafe { validate_enum_carrier_fields(fields, field_count, element_size) }
    {
        return status;
    }
    // SAFETY: caller guarantees a live exclusive outer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    for index in 0..buffer.length {
        let slot_offset = match index.checked_mul(element_size) {
            Some(offset) => offset,
            None => return BufferStatus::SizeOverflow,
        };
        // SAFETY: descriptor validity and field validation keep this record
        // pointer inside the initialized outer allocation.
        let record = unsafe { buffer.pointer.cast::<u8>().add(slot_offset as usize) };
        for field_index in 0..field_count {
            // SAFETY: validation established the table bounds.
            let field = unsafe { fields.add(field_index as usize).read_unaligned() };
            let active = match unsafe { record_field_is_active(record, field) } {
                Ok(active) => active,
                Err(status) => return status,
            };
            if !active {
                continue;
            }
            // SAFETY: field offset and descriptor layout were validated.
            let status = unsafe { destroy_record_field(record, field) };
            if status != BufferStatus::Ok {
                return status;
            }
        }
    }
    if buffer.capacity != 0 {
        let layout = match buffer_layout(element_size, alignment, buffer.capacity) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        // SAFETY: caller guarantees pointer provenance and exact outer layout.
        unsafe { dealloc(buffer.pointer.cast(), layout) };
    }
    *buffer = Buffer::EMPTY;
    BufferStatus::Ok
}

/// Releases every owning Buffer or OwnedString field of one standalone record value.
///
/// # Safety
///
/// `record` must point to initialized caller-owned record storage. The
/// element size and field table must describe its owning Buffer or OwnedString fields.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn carrier_destroy_record_fields(
    record: *mut u8,
    element_size: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if record.is_null() {
        return BufferStatus::NullPointer;
    }
    if let Err(status) = validate_element_layout(element_size, align_of::<Buffer>() as u64) {
        return status;
    }
    if let Err(status) = unsafe { validate_enum_carrier_fields(fields, field_count, element_size) }
    {
        return status;
    }
    for field_index in 0..field_count {
        // SAFETY: validation established the table bounds.
        let field = unsafe { fields.add(field_index as usize).read_unaligned() };
        let active = match unsafe { record_field_is_active(record, field) } {
            Ok(active) => active,
            Err(status) => return status,
        };
        if !active {
            continue;
        }
        // SAFETY: field offset and descriptor layout were validated.
        let status = unsafe { destroy_record_field(record, field) };
        if status != BufferStatus::Ok {
            return status;
        }
    }
    // SAFETY: caller owns initialized record storage for the declared size.
    unsafe { ptr::write_bytes(record, 0, element_size as usize) };
    BufferStatus::Ok
}

/// Removes one record from an owning buffer, destroys its owning Buffer
/// fields, and closes the gap by moving later record bytes left.  No record
/// value is returned, so ownership never passes through a temporary result.
///
/// # Safety
///
/// `buffer` must be a live exclusive header, `element_size`/`alignment` must
/// match the record allocation, and `fields` must describe initialized owning
/// Buffer descriptors inside each record.
#[must_use]
#[allow(unsafe_code, clippy::manual_is_multiple_of)]
pub unsafe fn buffer_remove_drop_record_fields(
    buffer: *mut Buffer,
    index: u64,
    element_size: u64,
    alignment: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    if let Err(status) = validate_element_layout(element_size, alignment) {
        return status;
    }
    if let Err(status) = unsafe { validate_enum_carrier_fields(fields, field_count, element_size) }
    {
        return status;
    }
    // SAFETY: caller guarantees a live exclusive Buffer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    if index >= buffer.length {
        return BufferStatus::OutOfBounds;
    }
    let record_offset = match index.checked_mul(element_size) {
        Some(offset) => offset,
        None => return BufferStatus::SizeOverflow,
    };
    // SAFETY: the validated indexed offset lies inside initialized record
    // storage and every field table entry was checked against element_size.
    let record = unsafe { buffer.pointer.cast::<u8>().add(record_offset as usize) };
    for field_index in 0..field_count {
        // SAFETY: validation established the table bounds.
        let field = unsafe { fields.add(field_index as usize).read_unaligned() };
        let active = match unsafe { record_field_is_active(record, field) } {
            Ok(active) => active,
            Err(status) => return status,
        };
        if !active {
            continue;
        }
        // SAFETY: the field offset identifies an initialized owning descriptor.
        let status = unsafe { destroy_record_field(record, field) };
        if status != BufferStatus::Ok {
            return status;
        }
    }
    let trailing_count = buffer.length - index - 1;
    if trailing_count != 0 {
        let move_bytes = match trailing_count.checked_mul(element_size) {
            Some(bytes) => bytes,
            None => return BufferStatus::SizeOverflow,
        };
        let source_offset = match index
            .checked_add(1)
            .and_then(|next| next.checked_mul(element_size))
        {
            Some(offset) => offset,
            None => return BufferStatus::SizeOverflow,
        };
        // SAFETY: source and destination are initialized record ranges in the
        // same allocation; ptr::copy provides memmove semantics for overlap.
        unsafe {
            ptr::copy(
                buffer.pointer.cast::<u8>().add(source_offset as usize),
                buffer.pointer.cast::<u8>().add(record_offset as usize),
                move_bytes as usize,
            )
        };
    }
    let tail_offset = match (buffer.length - 1).checked_mul(element_size) {
        Some(offset) => offset,
        None => return BufferStatus::SizeOverflow,
    };
    // SAFETY: the trailing slot is now the only stale copy of any moved
    // record descriptors; clear it before publishing the shorter length.
    unsafe {
        ptr::write_bytes(
            buffer.pointer.cast::<u8>().add(tail_offset as usize),
            0,
            element_size as usize,
        )
    };
    buffer.length -= 1;
    BufferStatus::Ok
}

/// Removes one nested owning Buffer element whose final leaf is a record with
/// owning Buffer fields. The selected nested chain is destroyed before later
/// outer descriptors are moved left, preserving exactly one owner per chain.
///
/// # Safety
///
/// `buffer` must be a live exclusive outer header. `element_size`/`alignment`
/// describe the nested Buffer descriptor layout, while the record layout and
/// field table describe initialized owning fields at the final leaf.
#[must_use]
#[allow(unsafe_code, clippy::manual_is_multiple_of, clippy::too_many_arguments)]
pub unsafe fn buffer_remove_drop_nested_record_fields(
    buffer: *mut Buffer,
    index: u64,
    element_size: u64,
    alignment: u64,
    depth: u64,
    record_element_size: u64,
    record_alignment: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    if depth == 0 {
        return BufferStatus::InvalidSize;
    }
    if element_size != size_of::<Buffer>() as u64 {
        return BufferStatus::InvalidSize;
    }
    if alignment != align_of::<Buffer>() as u64 {
        return BufferStatus::InvalidAlignment;
    }
    if let Err(status) = validate_element_layout(record_element_size, record_alignment) {
        return status;
    }
    if let Err(status) =
        unsafe { validate_enum_carrier_fields(fields, field_count, record_element_size) }
    {
        return status;
    }
    // SAFETY: caller guarantees a live exclusive Buffer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    if index >= buffer.length {
        return BufferStatus::OutOfBounds;
    }
    // SAFETY: the outer layout makes the selected slot a valid Buffer
    // descriptor and the nested destroy routine consumes its ownership.
    let base = buffer.pointer.cast::<Buffer>();
    let selected = unsafe { base.add(index as usize) };
    let status = if depth == 1 {
        unsafe {
            buffer_destroy_record_fields(
                selected,
                record_element_size,
                record_alignment,
                fields,
                field_count,
            )
        }
    } else {
        unsafe {
            buffer_destroy_nested_record_fields(
                selected,
                element_size,
                alignment,
                depth - 1,
                record_element_size,
                record_alignment,
                fields,
                field_count,
            )
        }
    };
    if status != BufferStatus::Ok {
        return status;
    }
    for current_index in index..(buffer.length - 1) {
        let current = unsafe { base.add(current_index as usize) };
        let next = unsafe { base.add((current_index + 1) as usize) };
        // SAFETY: selected was reset to EMPTY and every later descriptor is
        // moved byte-for-byte exactly once, so ownership is not duplicated.
        let moved = unsafe { ptr::read(next) };
        unsafe { ptr::write(current, moved) };
        unsafe { ptr::write(next, Buffer::EMPTY) };
    }
    buffer.length -= 1;
    BufferStatus::Ok
}

/// Removes one copy-safe element without returning it.  This is the generic
/// byte-compaction path used when no owning field table is required.
///
/// # Safety
///
/// The descriptor and element layout must describe a live initialized buffer.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_remove_drop(
    buffer: *mut Buffer,
    index: u64,
    element_size: u64,
    alignment: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    if let Err(status) = validate_element_layout(element_size, alignment) {
        return status;
    }
    // SAFETY: caller guarantees a live exclusive Buffer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    if index >= buffer.length {
        return BufferStatus::OutOfBounds;
    }
    let trailing_count = buffer.length - index - 1;
    if trailing_count != 0 {
        let move_bytes = match trailing_count.checked_mul(element_size) {
            Some(bytes) => bytes,
            None => return BufferStatus::SizeOverflow,
        };
        let source_offset = match index
            .checked_add(1)
            .and_then(|next| next.checked_mul(element_size))
        {
            Some(offset) => offset,
            None => return BufferStatus::SizeOverflow,
        };
        let destination_offset = match index.checked_mul(element_size) {
            Some(offset) => offset,
            None => return BufferStatus::SizeOverflow,
        };
        unsafe {
            ptr::copy(
                buffer.pointer.cast::<u8>().add(source_offset as usize),
                buffer.pointer.cast::<u8>().add(destination_offset as usize),
                move_bytes as usize,
            )
        };
    }
    let tail_offset = match (buffer.length - 1).checked_mul(element_size) {
        Some(offset) => offset,
        None => return BufferStatus::SizeOverflow,
    };
    unsafe {
        ptr::write_bytes(
            buffer.pointer.cast::<u8>().add(tail_offset as usize),
            0,
            element_size as usize,
        )
    };
    buffer.length -= 1;
    BufferStatus::Ok
}

/// Removes and destroys one `OwnedString` element, then move-compacts the
/// remaining descriptors without duplicating their owned byte allocations.
///
/// # Safety
///
/// `buffer` must be a live exclusive `Buffer<OwnedString>` header and the
/// supplied element layout must match [`Utf8String`].
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_remove_drop_owned_string(
    buffer: *mut Buffer,
    index: u64,
    element_size: u64,
    alignment: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    if element_size != size_of::<Utf8String>() as u64 {
        return BufferStatus::InvalidSize;
    }
    if alignment != align_of::<Utf8String>() as u64 {
        return BufferStatus::InvalidAlignment;
    }
    // SAFETY: caller guarantees a live exclusive Buffer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    if index >= buffer.length {
        return BufferStatus::OutOfBounds;
    }
    // SAFETY: index is inside the initialized descriptor range.
    let selected = unsafe { buffer.pointer.cast::<Utf8String>().add(index as usize) };
    let status = unsafe { string_destroy(selected) };
    if status != StringStatus::Ok {
        return match status {
            StringStatus::RuntimeNotInitialized => BufferStatus::RuntimeNotInitialized,
            StringStatus::NullPointer => BufferStatus::NullPointer,
            StringStatus::InvalidSize => BufferStatus::InvalidSize,
            StringStatus::InvalidAlignment => BufferStatus::InvalidAlignment,
            StringStatus::SizeOverflow => BufferStatus::SizeOverflow,
            StringStatus::OutOfMemory => BufferStatus::OutOfMemory,
            _ => BufferStatus::InvalidBuffer,
        };
    }
    for current_index in index..(buffer.length - 1) {
        // SAFETY: both descriptors are initialized and the move uses
        // ptr::read/write so ownership is transferred exactly once.
        let current = unsafe {
            buffer
                .pointer
                .cast::<Utf8String>()
                .add(current_index as usize)
        };
        let next = unsafe {
            buffer
                .pointer
                .cast::<Utf8String>()
                .add((current_index + 1) as usize)
        };
        let moved = unsafe { ptr::read(next) };
        unsafe {
            ptr::write(current, moved);
            ptr::write(next, Utf8String::EMPTY);
        }
    }
    buffer.length -= 1;
    BufferStatus::Ok
}

/// Moves one owning `@repr(C)` record out of a generic buffer into caller-owned
/// storage and closes the gap without duplicating descriptor ownership.
///
/// The output record is not dropped by this function; ownership of every
/// nested Buffer field is transferred to `output`. The caller must provide a
/// distinct, writable, correctly aligned record slot.
///
/// # Safety
///
/// `buffer` must be a live exclusive Buffer descriptor, `output` must point to
/// writable storage for one record, and the element layout must match both the
/// initialized buffer elements and the output record.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_remove_move_into(
    buffer: *mut Buffer,
    index: u64,
    output: *mut u8,
    element_size: u64,
    alignment: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() || output.is_null() {
        return BufferStatus::NullPointer;
    }
    if let Err(status) = validate_element_layout(element_size, alignment) {
        return status;
    }
    let alignment = match usize::try_from(alignment) {
        Ok(value) => value,
        Err(_) => return BufferStatus::InvalidAlignment,
    };
    if !(output as usize).is_multiple_of(alignment) {
        return BufferStatus::InvalidAlignment;
    }
    // SAFETY: caller guarantees a live exclusive Buffer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    if buffer.capacity > u64::MAX / element_size {
        return BufferStatus::SizeOverflow;
    }
    if index >= buffer.length {
        return BufferStatus::OutOfBounds;
    }
    let source_offset = match index.checked_mul(element_size) {
        Some(offset) => offset,
        None => return BufferStatus::SizeOverflow,
    };
    let source = unsafe { buffer.pointer.cast::<u8>().add(source_offset as usize) };
    // `ptr::copy` keeps this boundary memory-safe even if a caller violates
    // the distinct-output contract; correct ownership still requires a
    // separate output slot.
    unsafe { ptr::copy(source, output, element_size as usize) };
    let trailing_count = buffer.length - index - 1;
    if trailing_count != 0 {
        let move_bytes = match trailing_count.checked_mul(element_size) {
            Some(bytes) => bytes,
            None => return BufferStatus::SizeOverflow,
        };
        let trailing_source_offset = match index
            .checked_add(1)
            .and_then(|next| next.checked_mul(element_size))
        {
            Some(offset) => offset,
            None => return BufferStatus::SizeOverflow,
        };
        unsafe {
            ptr::copy(
                buffer
                    .pointer
                    .cast::<u8>()
                    .add(trailing_source_offset as usize),
                source,
                move_bytes as usize,
            )
        };
    }
    let tail_offset = match (buffer.length - 1).checked_mul(element_size) {
        Some(offset) => offset,
        None => return BufferStatus::SizeOverflow,
    };
    unsafe {
        ptr::write_bytes(
            buffer.pointer.cast::<u8>().add(tail_offset as usize),
            0,
            element_size as usize,
        )
    };
    buffer.length -= 1;
    BufferStatus::Ok
}

/// Inserts one move-safe element from caller-owned value storage and closes
/// the gap by shifting initialized elements to the right.
///
/// The source bytes are cleared only after the insertion succeeds, so a
/// failed bounds, layout, allocation, or overflow check leaves the source
/// value unchanged. This raw-pointer form avoids returning or passing a large
/// owning aggregate through the platform C ABI.
///
/// # Safety
///
/// `buffer` must be a live exclusive Buffer descriptor and `source` must point
/// to one initialized, distinct, correctly aligned element whose ownership may
/// be transferred. The element layout must match the initialized buffer.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_insert_move_from(
    buffer: *mut Buffer,
    index: u64,
    source: *mut u8,
    element_size: u64,
    alignment: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() || source.is_null() {
        return BufferStatus::NullPointer;
    }
    if let Err(status) = validate_element_layout(element_size, alignment) {
        return status;
    }
    let alignment = match usize::try_from(alignment) {
        Ok(value) => value,
        Err(_) => return BufferStatus::InvalidAlignment,
    };
    if !(source as usize).is_multiple_of(alignment) {
        return BufferStatus::InvalidAlignment;
    }
    // SAFETY: caller guarantees a live exclusive Buffer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return status;
    }
    if index > buffer.length {
        return BufferStatus::OutOfBounds;
    }
    if buffer.length == u64::MAX {
        return BufferStatus::SizeOverflow;
    }
    let move_count = buffer.length - index;
    let move_bytes = match move_count.checked_mul(element_size) {
        Some(bytes) => bytes,
        None => return BufferStatus::SizeOverflow,
    };
    // Reserve before touching either ownership-bearing value. A failed
    // allocation therefore preserves both the source and destination.
    let status =
        unsafe { buffer_reserve(buffer, element_size, alignment as u64, buffer.length + 1) };
    if status != BufferStatus::Ok {
        return status;
    }
    let destination_offset = match index.checked_mul(element_size) {
        Some(offset) => offset,
        None => return BufferStatus::SizeOverflow,
    };
    let destination = unsafe { buffer.pointer.cast::<u8>().add(destination_offset as usize) };
    if move_bytes != 0 {
        // `ptr::copy` is overlap-safe for the right shift. The type checker
        // enforces a distinct source place, while this operation remains
        // memory-safe even if an external caller violates that contract.
        unsafe {
            ptr::copy(
                destination,
                destination.add(element_size as usize),
                move_bytes as usize,
            )
        };
    }
    unsafe { ptr::copy(source, destination, element_size as usize) };
    unsafe { ptr::write_bytes(source, 0, element_size as usize) };
    buffer.length += 1;
    BufferStatus::Ok
}

/// Moves the last initialized element out of an owning buffer into
/// caller-owned storage and closes the gap without returning a large
/// aggregate through the platform C ABI.
///
/// The output record is not dropped by this function; ownership of its
/// initialized bytes is transferred to `output`. The element size/alignment
/// describe both the buffer slot and the output slot, including tagged
/// carriers and inline owning records.
///
/// # Safety
///
/// `buffer` must be a live exclusive Buffer descriptor, `output` must point to
/// writable storage for one element, and the element layout must match both
/// the initialized buffer elements and the output value.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_pop_move_into(
    buffer: *mut Buffer,
    output: *mut u8,
    element_size: u64,
    alignment: u64,
) -> BufferStatus {
    if runtime_state() != RuntimeState::Initialized {
        return BufferStatus::RuntimeNotInitialized;
    }
    if buffer.is_null() {
        return BufferStatus::NullPointer;
    }
    // SAFETY: the caller contract requires a live descriptor. The delegated
    // move routine performs the authoritative structural validation.
    let length = unsafe { (*buffer).length };
    if length == 0 {
        return BufferStatus::OutOfBounds;
    }
    // SAFETY: the descriptor and output pointer are forwarded unchanged to
    // the already validated caller-owned move path.
    unsafe { buffer_remove_move_into(buffer, length - 1, output, element_size, alignment) }
}

/// Moves the last initialized element out of an owning `Buffer<Buffer<U>>`.
///
/// The outer slot is reset to an empty descriptor before the outer length is
/// decremented, transferring ownership of the nested allocation to the
/// returned result without copying or dropping it.
///
/// # Safety
///
/// `buffer` must be a live exclusive outer header from this runtime. The
/// outer element layout must be the native `Buffer` descriptor layout, and
/// `nested_element_size`/`nested_alignment` must match the nested allocation.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_pop(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    nested_element_size: u64,
    nested_alignment: u64,
) -> BufferResult {
    if runtime_state() != RuntimeState::Initialized {
        return BufferResult::failure(BufferStatus::RuntimeNotInitialized);
    }
    if buffer.is_null() {
        return BufferResult::failure(BufferStatus::NullPointer);
    }
    if element_size != size_of::<Buffer>() as u64 {
        return BufferResult::failure(BufferStatus::InvalidSize);
    }
    if alignment != align_of::<Buffer>() as u64 {
        return BufferResult::failure(BufferStatus::InvalidAlignment);
    }
    if let Err(status) = validate_element_layout(nested_element_size, nested_alignment) {
        return BufferResult::failure(status);
    }
    // SAFETY: caller guarantees a live exclusive Buffer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return BufferResult::failure(status);
    }
    if buffer.length == 0 {
        return BufferResult::failure(BufferStatus::OutOfBounds);
    }
    let index = buffer.length - 1;
    // SAFETY: the outer layout check makes the final initialized slot a
    // properly aligned Buffer descriptor within the allocation.
    let nested = unsafe { buffer.pointer.cast::<Buffer>().add(index as usize) };
    // SAFETY: the slot is initialized and ownership is transferred by move.
    let value = unsafe { ptr::read(nested) };
    // SAFETY: leave the consumed slot in a non-owning empty state so a later
    // nested destroy cannot double-free the transferred allocation.
    unsafe { ptr::write(nested, Buffer::EMPTY) };
    buffer.length = index;
    BufferResult::success(value)
}

/// Moves an initialized element out of an owning `Buffer<Buffer<U>>` at an
/// arbitrary index and closes the gap by moving later descriptors left.
///
/// Every consumed slot is reset to an empty descriptor, so the outer drop
/// glue retains exactly one owner for each remaining nested allocation.
///
/// # Safety
///
/// `buffer` must be a live exclusive outer header from this runtime. The
/// outer element layout must be the native `Buffer` descriptor layout, and
/// `nested_element_size`/`nested_alignment` must match each nested allocation.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_remove_move(
    buffer: *mut Buffer,
    index: u64,
    element_size: u64,
    alignment: u64,
    nested_element_size: u64,
    nested_alignment: u64,
) -> BufferResult {
    if runtime_state() != RuntimeState::Initialized {
        return BufferResult::failure(BufferStatus::RuntimeNotInitialized);
    }
    if buffer.is_null() {
        return BufferResult::failure(BufferStatus::NullPointer);
    }
    if element_size != size_of::<Buffer>() as u64 {
        return BufferResult::failure(BufferStatus::InvalidSize);
    }
    if alignment != align_of::<Buffer>() as u64 {
        return BufferResult::failure(BufferStatus::InvalidAlignment);
    }
    if let Err(status) = validate_element_layout(nested_element_size, nested_alignment) {
        return BufferResult::failure(status);
    }
    // SAFETY: caller guarantees a live exclusive Buffer header.
    let buffer = unsafe { &mut *buffer };
    if let Err(status) = validate_buffer(buffer) {
        return BufferResult::failure(status);
    }
    if index >= buffer.length {
        return BufferResult::failure(BufferStatus::OutOfBounds);
    }
    // SAFETY: the outer layout check makes the indexed slot aligned and
    // initialized, and the caller guarantees ownership of every descriptor.
    let base = buffer.pointer.cast::<Buffer>();
    let removed = unsafe { ptr::read(base.add(index as usize)) };
    for current_index in index..(buffer.length - 1) {
        // SAFETY: both slots are within the initialized outer range.
        let current = unsafe { base.add(current_index as usize) };
        let next = unsafe { base.add((current_index + 1) as usize) };
        let moved = unsafe { ptr::read(next) };
        unsafe { ptr::write(current, moved) };
        unsafe { ptr::write(next, Buffer::EMPTY) };
    }
    buffer.length -= 1;
    BufferResult::success(removed)
}

/// Creates a checked non-owning subslice from a buffer header.
///
/// The returned pointer is borrowed from the buffer and becomes invalid when
/// the buffer is resized or destroyed. No allocation or copying occurs.
///
/// # Safety
///
/// `buffer` must be a live immutable descriptor whose element layout matches
/// `element_size`/`alignment` and whose pointer remains valid for the call.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn buffer_slice(
    buffer: *const Buffer,
    element_size: u64,
    alignment: u64,
    start: u64,
    count: u64,
) -> SliceResult {
    if runtime_state() != RuntimeState::Initialized {
        return SliceResult::failure(BufferStatus::RuntimeNotInitialized);
    }
    if buffer.is_null() {
        return SliceResult::failure(BufferStatus::NullPointer);
    }
    // SAFETY: caller guarantees the pointer is a live immutable Buffer header.
    let buffer = unsafe { &*buffer };
    if let Err(status) = validate_buffer(buffer) {
        return SliceResult::failure(status);
    }
    if let Err(status) = validate_element_layout(element_size, alignment) {
        return SliceResult::failure(status);
    }
    if start > buffer.length || count > buffer.length - start {
        return SliceResult::failure(BufferStatus::OutOfBounds);
    }
    if count == 0 {
        return SliceResult::success(Slice {
            pointer: ptr::null_mut(),
            length: 0,
        });
    }
    if let Err(status) = buffer_layout(element_size, alignment, buffer.capacity) {
        return SliceResult::failure(status);
    }
    let offset = match element_size.checked_mul(start) {
        Some(offset) => offset,
        None => return SliceResult::failure(BufferStatus::SizeOverflow),
    };
    let offset = match usize::try_from(offset) {
        Ok(offset) => offset,
        Err(_) => return SliceResult::failure(BufferStatus::SizeOverflow),
    };
    // SAFETY: buffer validation and bounds proof establish an in-allocation
    // byte offset; caller guarantees the original pointer provenance.
    let pointer = unsafe { buffer.pointer.cast::<u8>().add(offset) };
    let alignment = usize::try_from(alignment).expect("validated alignment fits target usize");
    if pointer.addr() % alignment != 0 {
        return SliceResult::failure(BufferStatus::InvalidAlignment);
    }
    SliceResult::success(Slice {
        pointer: pointer.cast(),
        length: count,
    })
}

/// Creates a checked subslice from an existing non-owning slice descriptor.
///
/// # Safety
///
/// `slice` must be a live descriptor whose pointer and length came from a
/// compatible buffer, and the element layout must match its source storage.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn slice_subslice(
    slice: *const Slice,
    element_size: u64,
    alignment: u64,
    start: u64,
    count: u64,
) -> SliceResult {
    if runtime_state() != RuntimeState::Initialized {
        return SliceResult::failure(BufferStatus::RuntimeNotInitialized);
    }
    if slice.is_null() {
        return SliceResult::failure(BufferStatus::NullPointer);
    }
    // SAFETY: caller guarantees a live immutable Slice descriptor.
    let slice = unsafe { &*slice };
    let buffer = Buffer {
        pointer: slice.pointer,
        length: slice.length,
        capacity: slice.length,
    };
    // SAFETY: the temporary header borrows the same source pointer and the
    // helper performs the checked range/layout validation.
    unsafe { buffer_slice(&buffer, element_size, alignment, start, count) }
}

/// Creates an empty owning UTF-8 string with the requested byte capacity.
#[must_use]
#[allow(unsafe_code)]
pub fn string_create(capacity: u64) -> Utf8StringResult {
    if runtime_state() != RuntimeState::Initialized {
        return Utf8StringResult::failure(StringStatus::RuntimeNotInitialized);
    }
    if capacity == 0 {
        return Utf8StringResult::success(Utf8String::EMPTY);
    }
    let layout = match string_layout(capacity) {
        Ok(layout) => layout,
        Err(status) => return Utf8StringResult::failure(status),
    };
    // SAFETY: `layout` is validated and nonzero.
    let pointer = unsafe { alloc(layout) };
    if pointer.is_null() {
        Utf8StringResult::failure(StringStatus::OutOfMemory)
    } else {
        Utf8StringResult::success(Utf8String {
            pointer,
            length: 0,
            capacity,
        })
    }
}

/// Copies a borrowed byte range into a validated owning UTF-8 string.
///
/// # Safety
///
/// When `length` is nonzero, `bytes` must point to a live readable byte range
/// for the duration of this call. The range may be retained only by the
/// caller; the returned string owns its copied bytes.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn string_from_utf8(bytes: *const u8, length: u64) -> Utf8StringResult {
    if runtime_state() != RuntimeState::Initialized {
        return Utf8StringResult::failure(StringStatus::RuntimeNotInitialized);
    }
    let length = match usize::try_from(length) {
        Ok(length) => length,
        Err(_) => return Utf8StringResult::failure(StringStatus::SizeOverflow),
    };
    if length == 0 {
        return Utf8StringResult::success(Utf8String::EMPTY);
    }
    if bytes.is_null() {
        return Utf8StringResult::failure(StringStatus::NullPointer);
    }
    // SAFETY: the caller supplies a live immutable byte range for the call.
    let source = unsafe { slice::from_raw_parts(bytes, length) };
    if str::from_utf8(source).is_err() {
        return Utf8StringResult::failure(StringStatus::InvalidUtf8);
    }
    let capacity = u64::try_from(length).expect("usize fits u64 on supported targets");
    let layout = match string_layout(capacity) {
        Ok(layout) => layout,
        Err(status) => return Utf8StringResult::failure(status),
    };
    // SAFETY: `layout` is validated and nonzero.
    let pointer = unsafe { alloc(layout) };
    if pointer.is_null() {
        return Utf8StringResult::failure(StringStatus::OutOfMemory);
    }
    // SAFETY: destination owns `length` writable bytes and source is a
    // separate borrowed range supplied by the caller.
    unsafe { ptr::copy_nonoverlapping(bytes, pointer, length) };
    Utf8StringResult::success(Utf8String {
        pointer,
        length: capacity,
        capacity,
    })
}

/// Reserves at least `minimum_capacity` bytes in an owning UTF-8 string.
///
/// # Safety
///
/// `string` must be a live exclusive header produced by this runtime. Its
/// pointer, length and capacity must not be modified concurrently.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn string_reserve(string: *mut Utf8String, minimum_capacity: u64) -> StringStatus {
    if runtime_state() != RuntimeState::Initialized {
        return StringStatus::RuntimeNotInitialized;
    }
    if string.is_null() {
        return StringStatus::NullPointer;
    }
    // SAFETY: caller guarantees a live exclusive string header.
    let string = unsafe { &mut *string };
    if let Err(status) = validate_string_header(string) {
        return status;
    }
    if minimum_capacity <= string.capacity {
        return StringStatus::Ok;
    }
    let new_layout = match string_layout(minimum_capacity) {
        Ok(layout) => layout,
        Err(status) => return status,
    };
    let resized = if string.capacity == 0 {
        // SAFETY: `new_layout` is validated and nonzero.
        unsafe { alloc(new_layout) }
    } else {
        let old_layout = match string_layout(string.capacity) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        // SAFETY: caller guarantees pointer provenance and exact old layout.
        unsafe { realloc(string.pointer, old_layout, new_layout.size()) }
    };
    if resized.is_null() {
        return StringStatus::OutOfMemory;
    }
    string.pointer = resized;
    string.capacity = minimum_capacity;
    StringStatus::Ok
}

/// Appends one borrowed valid UTF-8 range without copying or allocating when
/// the existing capacity is sufficient.
///
/// # Safety
///
/// `string` must be a live exclusive header. The source byte range must remain
/// valid for the call and must not overlap the string allocation because a
/// reserve may move that allocation.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn string_append_utf8(
    string: *mut Utf8String,
    bytes: *const u8,
    length: u64,
) -> StringStatus {
    if runtime_state() != RuntimeState::Initialized {
        return StringStatus::RuntimeNotInitialized;
    }
    if string.is_null() {
        return StringStatus::NullPointer;
    }
    let length = match usize::try_from(length) {
        Ok(length) => length,
        Err(_) => return StringStatus::SizeOverflow,
    };
    if length != 0 && bytes.is_null() {
        return StringStatus::NullPointer;
    }
    // SAFETY: caller guarantees the borrowed source range for the call. A
    // zero-length append deliberately uses a non-null static empty slice.
    let source: &[u8] = if length == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(bytes, length) }
    };
    if str::from_utf8(source).is_err() {
        return StringStatus::InvalidUtf8;
    }
    // SAFETY: caller guarantees a live exclusive string header.
    let string = unsafe { &mut *string };
    if let Err(status) = validate_string_header(string) {
        return status;
    }
    if string.length != 0 {
        // SAFETY: the header proves the payload is live for `length` bytes.
        let existing = unsafe { slice::from_raw_parts(string.pointer, string.length as usize) };
        if str::from_utf8(existing).is_err() {
            return StringStatus::InvalidString;
        }
    }
    let append_length = u64::try_from(length).expect("usize fits u64 on supported targets");
    let new_length = match string.length.checked_add(append_length) {
        Some(length) => length,
        None => return StringStatus::SizeOverflow,
    };
    if new_length > string.capacity {
        let doubled = string.capacity.checked_mul(2).unwrap_or(new_length);
        let minimum_capacity = doubled.max(new_length);
        // SAFETY: this function owns the exclusive header and preserves the
        // documented non-overlap precondition for the borrowed source.
        let status = unsafe { string_reserve(string, minimum_capacity) };
        if status != StringStatus::Ok {
            return status;
        }
    }
    if length != 0 {
        // SAFETY: capacity proof establishes writable tail bytes.
        unsafe {
            ptr::copy_nonoverlapping(bytes, string.pointer.add(string.length as usize), length)
        };
    }
    string.length = new_length;
    StringStatus::Ok
}

/// Clears the logical contents without releasing the reserved capacity.
///
/// # Safety
///
/// `string` must be a live exclusive header produced by this runtime.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn string_clear(string: *mut Utf8String) -> StringStatus {
    if runtime_state() != RuntimeState::Initialized {
        return StringStatus::RuntimeNotInitialized;
    }
    if string.is_null() {
        return StringStatus::NullPointer;
    }
    // SAFETY: caller guarantees a live exclusive string header.
    let string = unsafe { &mut *string };
    if let Err(status) = validate_string_header(string) {
        return status;
    }
    string.length = 0;
    StringStatus::Ok
}

/// Releases an owning UTF-8 string and resets its header to empty.
///
/// # Safety
///
/// `string` must be a live exclusive header returned by this runtime.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn string_destroy(string: *mut Utf8String) -> StringStatus {
    if runtime_state() != RuntimeState::Initialized {
        return StringStatus::RuntimeNotInitialized;
    }
    if string.is_null() {
        return StringStatus::NullPointer;
    }
    // SAFETY: caller guarantees a live exclusive string header.
    let string = unsafe { &mut *string };
    if let Err(status) = validate_string_header(string) {
        return status;
    }
    if string.capacity != 0 {
        let layout = match string_layout(string.capacity) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        // SAFETY: caller guarantees pointer provenance and exact layout.
        unsafe { dealloc(string.pointer, layout) };
    }
    *string = Utf8String::EMPTY;
    StringStatus::Ok
}

/// Allocation-free IEEE scalar absolute value for `Float32`.
#[must_use]
#[inline]
pub fn math_abs_f32(value: f32) -> f32 {
    value.abs()
}

/// Allocation-free IEEE scalar absolute value for `Float64`.
#[must_use]
#[inline]
pub fn math_abs_f64(value: f64) -> f64 {
    value.abs()
}

/// Allocation-free IEEE square root for `Float32`.
#[must_use]
#[inline]
pub fn math_sqrt_f32(value: f32) -> f32 {
    value.sqrt()
}

/// Allocation-free IEEE square root for `Float64`.
#[must_use]
#[inline]
pub fn math_sqrt_f64(value: f64) -> f64 {
    value.sqrt()
}

/// Allocation-free IEEE arc-cosine for `Float32`.
///
/// This is kept in the runtime rather than reimplemented in each native
/// kernel so animation backends share one target-provided math contract.
#[must_use]
#[inline]
pub fn math_acos_f32(value: f32) -> f32 {
    value.acos()
}

/// Allocation-free IEEE sine for `Float32`.
#[must_use]
#[inline]
pub fn math_sin_f32(value: f32) -> f32 {
    value.sin()
}

/// Allocation-free IEEE cosine for `Float32`.
#[must_use]
#[inline]
pub fn math_cos_f32(value: f32) -> f32 {
    value.cos()
}

/// Allocation-free IEEE floor for `Float32`.
#[must_use]
#[inline]
pub fn math_floor_f32(value: f32) -> f32 {
    value.floor()
}

/// Allocation-free IEEE floor for `Float64`.
#[must_use]
#[inline]
pub fn math_floor_f64(value: f64) -> f64 {
    value.floor()
}

/// Allocation-free IEEE ceil for `Float32`.
#[must_use]
#[inline]
pub fn math_ceil_f32(value: f32) -> f32 {
    value.ceil()
}

/// Allocation-free IEEE ceil for `Float64`.
#[must_use]
#[inline]
pub fn math_ceil_f64(value: f64) -> f64 {
    value.ceil()
}

/// C-compatible two-lane single-precision value type.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Float2 {
    pub x: f32,
    pub y: f32,
}

/// C-compatible three-lane single-precision value type.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Float3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// C-compatible four-lane single-precision value type.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Float4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

/// Target-neutral eight-lane single-precision value type.
///
/// The lane array is an internal runtime value representation. It is not an
/// exported C ABI signature; the `Float8` external calling convention remains
/// a separate specification gate.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Float8 {
    pub lanes: [f32; 8],
}

/// C-compatible quaternion stored as `(x, y, z, w)`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Quaternion {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

/// C-compatible row-major 4×4 single-precision matrix.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Matrix4 {
    pub values: [f32; 16],
}

/// Adds two `Float2` values without allocation.
#[must_use]
#[inline]
pub fn float2_add(left: Float2, right: Float2) -> Float2 {
    Float2 {
        x: left.x + right.x,
        y: left.y + right.y,
    }
}

/// Adds two `Float3` values without allocation.
#[must_use]
#[inline]
pub fn float3_add(left: Float3, right: Float3) -> Float3 {
    Float3 {
        x: left.x + right.x,
        y: left.y + right.y,
        z: left.z + right.z,
    }
}

/// Adds two `Float4` values without allocation.
#[must_use]
#[inline]
pub fn float4_add(left: Float4, right: Float4) -> Float4 {
    Float4 {
        x: left.x + right.x,
        y: left.y + right.y,
        z: left.z + right.z,
        w: left.w + right.w,
    }
}

/// Adds two target-neutral `Float8` values without allocation.
#[must_use]
#[inline]
pub fn float8_add(left: Float8, right: Float8) -> Float8 {
    let mut lanes = [0.0; 8];
    let mut index = 0;
    while index < lanes.len() {
        lanes[index] = left.lanes[index] + right.lanes[index];
        index += 1;
    }
    Float8 { lanes }
}

/// Returns the three-dimensional dot product without allocation.
#[must_use]
#[inline]
pub fn float3_dot(left: Float3, right: Float3) -> f32 {
    left.x * right.x + left.y * right.y + left.z * right.z
}

/// Returns the identity quaternion `(0, 0, 0, 1)`.
#[must_use]
#[inline]
pub const fn quaternion_identity() -> Quaternion {
    Quaternion {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    }
}

fn quaternion_lerp_unclamped(left: Quaternion, right: Quaternion, weight: f32) -> Quaternion {
    Quaternion {
        x: left.x + (right.x - left.x) * weight,
        y: left.y + (right.y - left.y) * weight,
        z: left.z + (right.z - left.z) * weight,
        w: left.w + (right.w - left.w) * weight,
    }
}

fn quaternion_normalize(value: Quaternion) -> Quaternion {
    let magnitude = math_sqrt_f32(
        value.x * value.x + value.y * value.y + value.z * value.z + value.w * value.w,
    );
    if magnitude > 0.000001 {
        let inverse = 1.0 / magnitude;
        Quaternion {
            x: value.x * inverse,
            y: value.y * inverse,
            z: value.z * inverse,
            w: value.w * inverse,
        }
    } else {
        quaternion_identity()
    }
}

/// Computes Unity-compatible shortest-arc spherical interpolation without
/// clamping the interpolation weight. The near-linear path is normalized so
/// extrapolation and nearly parallel quaternions preserve a valid rotation.
#[must_use]
#[inline]
pub fn quaternion_slerp_unclamped(
    left: Quaternion,
    mut right: Quaternion,
    weight: f32,
) -> Quaternion {
    let mut dot = left.x * right.x + left.y * right.y + left.z * right.z + left.w * right.w;
    if dot < 0.0 {
        right = Quaternion {
            x: -right.x,
            y: -right.y,
            z: -right.z,
            w: -right.w,
        };
        dot = -dot;
    }
    let dot = dot.clamp(-1.0, 1.0);
    if dot > 0.9995 {
        return quaternion_normalize(quaternion_lerp_unclamped(left, right, weight));
    }

    let theta_zero = math_acos_f32(dot);
    let sin_theta_zero = math_sin_f32(theta_zero);
    if sin_theta_zero.abs() < 0.000001 {
        return quaternion_normalize(quaternion_lerp_unclamped(left, right, weight));
    }

    let theta = theta_zero * weight;
    let sin_theta = math_sin_f32(theta);
    let left_scale = math_cos_f32(theta) - dot * sin_theta / sin_theta_zero;
    let right_scale = sin_theta / sin_theta_zero;
    Quaternion {
        x: left.x * left_scale + right.x * right_scale,
        y: left.y * left_scale + right.y * right_scale,
        z: left.z * left_scale + right.z * right_scale,
        w: left.w * left_scale + right.w * right_scale,
    }
}

/// Returns the row-major 4×4 identity matrix.
#[must_use]
#[inline]
pub const fn matrix4_identity() -> Matrix4 {
    Matrix4 {
        values: [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ],
    }
}

/// Builds the fixed-layout record used by a bounds failure.
#[must_use]
pub const fn bounds_panic_info(index: u64, length: u64) -> PanicInfo {
    PanicInfo {
        code: PanicCode::BoundsCheck.code(),
        detail_a: index,
        detail_b: length,
    }
}

/// Reports a bounds failure from generated native code and terminates.
///
/// The function deliberately performs no allocation, locking, callback or
/// unwind. Generated LLVM places it on a failure edge followed by `unreachable`;
/// continuing would make the memory-safety proof invalid.
#[cold]
#[inline(never)]
pub fn bounds_panic(index: u64, length: u64) -> ! {
    let _info = bounds_panic_info(index, length);
    process::abort()
}

/// Allocates uninitialized system memory with an explicit size and alignment.
#[must_use]
#[allow(unsafe_code)]
pub fn system_allocate(size: u64, alignment: u64) -> AllocationResult {
    let layout = match allocator_layout(size, alignment) {
        Ok(layout) => layout,
        Err(status) => return AllocationResult::failure(status),
    };
    // SAFETY: allocator_layout produced a nonzero valid Layout.
    let pointer = unsafe { alloc(layout) };
    if pointer.is_null() {
        AllocationResult::failure(AllocatorStatus::OutOfMemory)
    } else {
        AllocationResult::success(pointer)
    }
}

/// Resizes a system allocation while preserving `min(old_size, new_size)` bytes.
///
/// # Safety
///
/// `pointer` must come from this runtime allocator, must still be live, and
/// `old_size`/`alignment` must exactly match its original/current layout.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn system_reallocate(
    pointer: *mut c_void,
    old_size: u64,
    new_size: u64,
    alignment: u64,
) -> AllocationResult {
    if runtime_state() != RuntimeState::Initialized {
        return AllocationResult::failure(AllocatorStatus::RuntimeNotInitialized);
    }
    if pointer.is_null() {
        return AllocationResult::failure(AllocatorStatus::NullPointer);
    }
    let old_layout = match allocator_layout_initialized(old_size, alignment) {
        Ok(layout) => layout,
        Err(status) => return AllocationResult::failure(status),
    };
    let new_size = match validated_size(new_size) {
        Ok(size) => size,
        Err(status) => return AllocationResult::failure(status),
    };
    if Layout::from_size_align(new_size, old_layout.align()).is_err() {
        return AllocationResult::failure(AllocatorStatus::SizeOverflow);
    }
    // SAFETY: caller guarantees pointer/old_layout provenance and liveness;
    // new_size is nonzero and valid for the unchanged alignment.
    let resized = unsafe { realloc(pointer.cast(), old_layout, new_size) };
    if resized.is_null() {
        AllocationResult::failure(AllocatorStatus::OutOfMemory)
    } else {
        AllocationResult::success(resized)
    }
}

/// Releases one live system allocation.
///
/// # Safety
///
/// `pointer` must come from this runtime allocator, must still be live, and
/// `size`/`alignment` must exactly match its current layout.
#[allow(unsafe_code)]
pub unsafe fn system_deallocate(
    pointer: *mut c_void,
    size: u64,
    alignment: u64,
) -> AllocatorStatus {
    if runtime_state() != RuntimeState::Initialized {
        return AllocatorStatus::RuntimeNotInitialized;
    }
    if pointer.is_null() {
        return AllocatorStatus::NullPointer;
    }
    let layout = match allocator_layout_initialized(size, alignment) {
        Ok(layout) => layout,
        Err(status) => return status,
    };
    // SAFETY: caller guarantees pointer/layout provenance and liveness.
    unsafe { dealloc(pointer.cast(), layout) };
    AllocatorStatus::Ok
}

/// Creates an empty region allocator handle.
///
/// The handle is opaque and must eventually be released with
/// [`region_destroy`]. Creating a region does not allocate its backing arena;
/// blocks are obtained lazily by [`region_allocate`].
#[must_use]
#[allow(unsafe_code)]
pub fn region_create() -> RegionResult {
    if runtime_state() != RuntimeState::Initialized {
        return RegionResult::failure(AllocatorStatus::RuntimeNotInitialized);
    }
    let layout = Layout::new::<Region>();
    // SAFETY: `layout` is the valid layout of the region metadata object.
    let pointer = unsafe { alloc(layout).cast::<Region>() };
    if pointer.is_null() {
        return RegionResult::failure(AllocatorStatus::OutOfMemory);
    }
    // SAFETY: the allocation has the exact size/alignment required by Region.
    unsafe { pointer.write(Region::new()) };
    RegionResult::success(pointer)
}

/// Allocates one block owned by a region.
///
/// Region blocks cannot be individually reallocated or deallocated; the
/// entire region is released by [`region_destroy`].
///
/// # Safety
///
/// `region` must be a live pointer returned by [`region_create`], and no other
/// mutable operation may access that region concurrently.
#[must_use]
#[allow(unsafe_code)]
pub unsafe fn region_allocate(region: *mut c_void, size: u64, alignment: u64) -> AllocationResult {
    if runtime_state() != RuntimeState::Initialized {
        return AllocationResult::failure(AllocatorStatus::RuntimeNotInitialized);
    }
    if region.is_null() {
        return AllocationResult::failure(AllocatorStatus::NullPointer);
    }
    // SAFETY: the caller guarantees that `region` is a live `Region` handle.
    let region =
        unsafe { region.cast::<Region>().as_mut() }.expect("region pointer was checked for null");
    // SAFETY: the caller also guarantees exclusive access to the handle.
    unsafe { region.allocate(size, alignment) }
}

/// Destroys a region and releases every block it owns.
///
/// # Safety
///
/// `region` must be a live pointer returned by [`region_create`] that has not
/// already been destroyed. Any pointer into one of its allocations becomes
/// invalid when this function returns.
#[allow(unsafe_code)]
pub unsafe fn region_destroy(region: *mut c_void) -> AllocatorStatus {
    if runtime_state() != RuntimeState::Initialized {
        return AllocatorStatus::RuntimeNotInitialized;
    }
    if region.is_null() {
        return AllocatorStatus::NullPointer;
    }
    // SAFETY: `region` was allocated with the global allocator using this
    // exact metadata layout and is still live by the caller contract.
    let pointer = region.cast::<Region>();
    unsafe {
        ptr::drop_in_place(pointer);
        dealloc(pointer.cast(), Layout::new::<Region>());
    }
    AllocatorStatus::Ok
}

fn allocator_layout(size: u64, alignment: u64) -> Result<Layout, AllocatorStatus> {
    if runtime_state() != RuntimeState::Initialized {
        return Err(AllocatorStatus::RuntimeNotInitialized);
    }
    allocator_layout_initialized(size, alignment)
}

fn allocator_layout_initialized(size: u64, alignment: u64) -> Result<Layout, AllocatorStatus> {
    let size = validated_size(size)?;
    let alignment = usize::try_from(alignment).map_err(|_| AllocatorStatus::InvalidAlignment)?;
    if !alignment.is_power_of_two() {
        return Err(AllocatorStatus::InvalidAlignment);
    }
    Layout::from_size_align(size, alignment).map_err(|_| AllocatorStatus::SizeOverflow)
}

fn validated_size(size: u64) -> Result<usize, AllocatorStatus> {
    if size == 0 {
        Err(AllocatorStatus::InvalidSize)
    } else {
        usize::try_from(size).map_err(|_| AllocatorStatus::SizeOverflow)
    }
}

// SAFETY: this stable, argument-free export has no caller invariants.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_abi_version() -> u64 {
    AbiVersion::CURRENT.packed()
}

// SAFETY: this stable, argument-free export has no caller invariants.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_build_id() -> u64 {
    RUNTIME_BUILD_ID
}

// SAFETY: all arguments are fixed-width integers and no pointer crosses the ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_initialize(required_major: u32, required_minor: u32) -> i32 {
    initialize(AbiVersion {
        major: required_major,
        minor: required_minor,
    })
    .code()
}

// SAFETY: this stable, argument-free export returns a fixed-width integer.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_is_initialized() -> u32 {
    u32::from(runtime_state() == RuntimeState::Initialized)
}

// SAFETY: the export returns an owned pointer/status pair; validation happens before allocation.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_system_allocate(size: u64, alignment: u64) -> AllocationResult {
    system_allocate(size, alignment)
}

/// Resizes a runtime allocation across the C ABI.
///
/// # Safety
///
/// The pointer and old layout must satisfy [`system_reallocate`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_system_reallocate(
    pointer: *mut c_void,
    old_size: u64,
    new_size: u64,
    alignment: u64,
) -> AllocationResult {
    // SAFETY: this ABI boundary exposes the same documented caller contract.
    unsafe { system_reallocate(pointer, old_size, new_size, alignment) }
}

/// Releases a runtime allocation across the C ABI.
///
/// # Safety
///
/// The pointer and layout must satisfy [`system_deallocate`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_system_deallocate(
    pointer: *mut c_void,
    size: u64,
    alignment: u64,
) -> i32 {
    // SAFETY: this ABI boundary exposes the same documented caller contract.
    unsafe { system_deallocate(pointer, size, alignment) }.code()
}

/// Creates one opaque region allocator handle across the C ABI.
// SAFETY: the export returns a C-layout pointer/status pair and performs no
// caller memory access.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_region_create() -> RegionResult {
    region_create()
}

/// Allocates one block owned by a region across the C ABI.
///
/// # Safety
///
/// The region pointer must satisfy [`region_allocate`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_region_allocate(
    region: *mut c_void,
    size: u64,
    alignment: u64,
) -> AllocationResult {
    // SAFETY: this ABI boundary exposes the same documented caller contract.
    unsafe { region_allocate(region, size, alignment) }
}

/// Destroys a region and all of its allocations across the C ABI.
///
/// # Safety
///
/// The region pointer must satisfy [`region_destroy`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_region_destroy(region: *mut c_void) -> i32 {
    // SAFETY: this ABI boundary exposes the same documented caller contract.
    unsafe { region_destroy(region) }.code()
}

/// Non-returning bounds panic entry point used by generated LLVM code.
///
/// # Safety
///
/// This function never returns. The generated caller must place it on a
/// failure edge and must not execute subsequent instructions on that edge.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_bounds_panic_u64(index: u64, length: u64) -> ! {
    bounds_panic(index, length)
}

/// Registers or clears logging/profiler callbacks across the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_set_callbacks(
    log: Option<LogCallback>,
    profiler_begin: Option<ProfilerBeginCallback>,
    profiler_end: Option<ProfilerEndCallback>,
    profiler_counter: Option<ProfilerCounterCallback>,
    context: *mut c_void,
) -> i32 {
    set_callbacks(log, profiler_begin, profiler_end, profiler_counter, context).code()
}

/// Dispatches one length-prefixed log message to the host callback.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_log(level: u32, message: *const u8, message_length: u64) -> i32 {
    log(level, message, message_length).code()
}

/// Dispatches one profiler begin event.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_profiler_begin_sample(name_id: u64) -> i32 {
    profiler_begin_sample(name_id).code()
}

/// Dispatches one profiler end event.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_profiler_end_sample() -> i32 {
    profiler_end_sample().code()
}

/// Dispatches one profiler counter event.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_profiler_counter(name_id: u64, value: i64) -> i32 {
    profiler_counter(name_id, value).code()
}

/// Creates an owning generic buffer across the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_buffer_create(
    element_size: u64,
    alignment: u64,
    capacity: u64,
) -> BufferResult {
    buffer_create(element_size, alignment, capacity)
}

/// Reserves at least `minimum_capacity` elements in a buffer across the C ABI.
///
/// # Safety
///
/// The descriptor and element layout must satisfy [`buffer_reserve`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_reserve(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    minimum_capacity: u64,
) -> i32 {
    // SAFETY: this ABI boundary exposes the same documented caller contract.
    unsafe { buffer_reserve(buffer, element_size, alignment, minimum_capacity) }.code()
}

/// Changes a buffer's logical length without allocation across the C ABI.
///
/// # Safety
///
/// The descriptor must satisfy [`buffer_resize`], including initialization of
/// any newly exposed elements.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_resize(buffer: *mut Buffer, new_length: u64) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller-owned header.
    unsafe { buffer_resize(buffer, new_length) }.code()
}

/// Clears a copy-safe generic buffer across the C ABI.
///
/// # Safety
///
/// The descriptor must satisfy [`buffer_clear`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_clear(buffer: *mut Buffer) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller-owned header.
    unsafe { buffer_clear(buffer) }.code()
}

/// Changes the logical length of an owning nested buffer across the C ABI.
///
/// # Safety
///
/// The descriptor, nested depth, and leaf layout must satisfy
/// [`buffer_resize_move`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_resize_move_status(
    buffer: *mut Buffer,
    new_length: u64,
    element_size: u64,
    alignment: u64,
    depth: u64,
    leaf_element_size: u64,
    leaf_alignment: u64,
) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller-owned header.
    unsafe {
        buffer_resize_move(
            buffer,
            new_length,
            element_size,
            alignment,
            depth,
            leaf_element_size,
            leaf_alignment,
        )
    }
    .code()
}

/// Clears an owning nested buffer across the C ABI.
///
/// # Safety
///
/// The descriptor, nested depth, and leaf layout must satisfy
/// [`buffer_clear_move`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_clear_move_status(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    depth: u64,
    leaf_element_size: u64,
    leaf_alignment: u64,
) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller-owned header.
    unsafe {
        buffer_clear_move(
            buffer,
            element_size,
            alignment,
            depth,
            leaf_element_size,
            leaf_alignment,
        )
    }
    .code()
}

/// Resizes an owning nested Buffer chain with an OwnedString leaf across the
/// C ABI.
///
/// # Safety
///
/// `buffer` must point to a live exclusive descriptor, and all element,
/// alignment, depth, and string-layout arguments must match its allocation.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_resize_move_nested_owned_string_status(
    buffer: *mut Buffer,
    new_length: u64,
    element_size: u64,
    alignment: u64,
    depth: u64,
    string_element_size: u64,
    string_alignment: u64,
) -> i32 {
    unsafe {
        buffer_resize_move_nested_owned_string(
            buffer,
            new_length,
            element_size,
            alignment,
            depth,
            string_element_size,
            string_alignment,
        )
    }
    .code()
}

/// Boolean convenience wrapper for nested OwnedString resize.
///
/// # Safety
///
/// The arguments must satisfy the same live-descriptor and layout contract as
/// [`jadren_rt_buffer_resize_move_nested_owned_string_status`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_resize_move_nested_owned_string(
    buffer: *mut Buffer,
    new_length: u64,
    element_size: u64,
    alignment: u64,
    depth: u64,
    string_element_size: u64,
    string_alignment: u64,
) -> i32 {
    let status = unsafe {
        jadren_rt_buffer_resize_move_nested_owned_string_status(
            buffer,
            new_length,
            element_size,
            alignment,
            depth,
            string_element_size,
            string_alignment,
        )
    };
    i32::from(status == BufferStatus::Ok.code())
}

/// Clears an owning nested Buffer chain with an OwnedString leaf across the
/// C ABI.
///
/// # Safety
///
/// `buffer` must point to a live exclusive descriptor, and all element,
/// alignment, depth, and string-layout arguments must match its allocation.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_clear_move_nested_owned_string_status(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    depth: u64,
    string_element_size: u64,
    string_alignment: u64,
) -> i32 {
    unsafe {
        buffer_clear_move_nested_owned_string(
            buffer,
            element_size,
            alignment,
            depth,
            string_element_size,
            string_alignment,
        )
    }
    .code()
}

/// Boolean convenience wrapper for nested OwnedString clear.
///
/// # Safety
///
/// The arguments must satisfy the same live-descriptor and layout contract as
/// [`jadren_rt_buffer_clear_move_nested_owned_string_status`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_clear_move_nested_owned_string(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    depth: u64,
    string_element_size: u64,
    string_alignment: u64,
) -> i32 {
    let status = unsafe {
        jadren_rt_buffer_clear_move_nested_owned_string_status(
            buffer,
            element_size,
            alignment,
            depth,
            string_element_size,
            string_alignment,
        )
    };
    i32::from(status == BufferStatus::Ok.code())
}

/// Resizes an owning `Buffer<OwnedString>` across the C ABI.
///
/// # Safety
///
/// `buffer` must be a live exclusive buffer header containing valid owned
/// string descriptors, and the supplied layout must match the allocation.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_resize_move_owned_string_status(
    buffer: *mut Buffer,
    new_length: u64,
    element_size: u64,
    alignment: u64,
) -> i32 {
    unsafe { buffer_resize_move_owned_string(buffer, new_length, element_size, alignment) }.code()
}

/// Boolean convenience wrapper for
/// [`jadren_rt_buffer_resize_move_owned_string_status`].
///
/// # Safety
///
/// The caller must uphold the same buffer and layout contract as the status
/// function.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_resize_move_owned_string(
    buffer: *mut Buffer,
    new_length: u64,
    element_size: u64,
    alignment: u64,
) -> i32 {
    let status = unsafe {
        jadren_rt_buffer_resize_move_owned_string_status(
            buffer,
            new_length,
            element_size,
            alignment,
        )
    };
    i32::from(status == BufferStatus::Ok.code())
}

/// Clears an owning `Buffer<OwnedString>` across the C ABI.
///
/// # Safety
///
/// `buffer` must be a live exclusive buffer header containing valid owned
/// string descriptors, and the supplied layout must match the allocation.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_clear_move_owned_string_status(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
) -> i32 {
    unsafe { buffer_clear_move_owned_string(buffer, element_size, alignment) }.code()
}

/// Boolean convenience wrapper for
/// [`jadren_rt_buffer_clear_move_owned_string_status`].
///
/// # Safety
///
/// The caller must uphold the same buffer and layout contract as the status
/// function.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_clear_move_owned_string(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
) -> i32 {
    let status =
        unsafe { jadren_rt_buffer_clear_move_owned_string_status(buffer, element_size, alignment) };
    i32::from(status == BufferStatus::Ok.code())
}

/// Resizes an owning nested Buffer whose final leaf is an owning-field record
/// across the C ABI.
///
/// # Safety
///
/// The descriptor, nested record layouts and field table must describe a live
/// caller-owned nested Buffer chain accepted by the runtime contract.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_resize_move_nested_record_fields_status(
    buffer: *mut Buffer,
    new_length: u64,
    element_size: u64,
    alignment: u64,
    depth: u64,
    record_element_size: u64,
    record_alignment: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller-owned header
    // and field-table contract.
    unsafe {
        buffer_resize_move_nested_record_fields(
            buffer,
            new_length,
            element_size,
            alignment,
            depth,
            record_element_size,
            record_alignment,
            fields,
            field_count,
        )
    }
    .code()
}

/// Boolean convenience wrapper for
/// [`jadren_rt_buffer_resize_move_nested_record_fields_status`].
///
/// # Safety
///
/// Uses the same caller-owned descriptor, layout and field-table contract as
/// the status wrapper.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_resize_move_nested_record_fields(
    buffer: *mut Buffer,
    new_length: u64,
    element_size: u64,
    alignment: u64,
    depth: u64,
    record_element_size: u64,
    record_alignment: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> i32 {
    // SAFETY: this ABI boundary shares the status wrapper's documented
    // caller-owned header and field-table contract.
    let status = unsafe {
        jadren_rt_buffer_resize_move_nested_record_fields_status(
            buffer,
            new_length,
            element_size,
            alignment,
            depth,
            record_element_size,
            record_alignment,
            fields,
            field_count,
        )
    };
    if status == BufferStatus::Ok.code() {
        1
    } else {
        0
    }
}

/// Status wrapper for direct owning record-buffer resize.
///
/// # Safety
///
/// The descriptor, record layout and field table must satisfy
/// [`buffer_resize_move_record_fields`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_resize_move_record_fields_status(
    buffer: *mut Buffer,
    new_length: u64,
    element_size: u64,
    alignment: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller-owned header
    // and field-table contract.
    unsafe {
        buffer_resize_move_record_fields(
            buffer,
            new_length,
            element_size,
            alignment,
            fields,
            field_count,
        )
    }
    .code()
}

/// Boolean convenience wrapper for direct owning record-buffer resize.
///
/// # Safety
///
/// Uses the same caller-owned descriptor, layout and field-table contract as
/// the status wrapper.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_resize_move_record_fields(
    buffer: *mut Buffer,
    new_length: u64,
    element_size: u64,
    alignment: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> i32 {
    let status = unsafe {
        jadren_rt_buffer_resize_move_record_fields_status(
            buffer,
            new_length,
            element_size,
            alignment,
            fields,
            field_count,
        )
    };
    if status == BufferStatus::Ok.code() {
        1
    } else {
        0
    }
}

/// Destroys an owning buffer across the C ABI.
///
/// # Safety
///
/// The descriptor and element layout must satisfy [`buffer_destroy`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_destroy(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
) -> i32 {
    // SAFETY: this ABI boundary exposes the same documented caller contract.
    unsafe { buffer_destroy(buffer, element_size, alignment) }.code()
}

/// Destroys an owning `Buffer<OwnedString>` across the C ABI.
///
/// # Safety
///
/// `buffer` must be a live exclusive buffer header containing valid owned
/// string descriptors, and the supplied layout must match the allocation.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_destroy_owned_string(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller contract.
    unsafe { buffer_destroy_owned_string(buffer, element_size, alignment) }.code()
}

/// Destroys an owning `Buffer<Buffer<U>>` across the C ABI.
///
/// # Safety
///
/// The descriptor and nested element layouts must satisfy
/// [`buffer_destroy_nested_buffer`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_destroy_nested_buffer(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    nested_element_size: u64,
    nested_alignment: u64,
) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller contract.
    unsafe {
        buffer_destroy_nested_buffer(
            buffer,
            element_size,
            alignment,
            nested_element_size,
            nested_alignment,
        )
    }
    .code()
}

/// Destroys an owning recursively nested buffer across the C ABI.
///
/// # Safety
///
/// The descriptor, depth, and leaf layout must describe a live recursive
/// buffer chain accepted by [`buffer_destroy_nested_buffer_recursive`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_destroy_nested_buffer_recursive(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    depth: u64,
    leaf_element_size: u64,
    leaf_alignment: u64,
) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller contract.
    unsafe {
        buffer_destroy_nested_buffer_recursive(
            buffer,
            element_size,
            alignment,
            depth,
            leaf_element_size,
            leaf_alignment,
        )
    }
    .code()
}

/// Destroys an owning recursively nested Buffer chain with an OwnedString
/// leaf across the C ABI.
///
/// # Safety
///
/// `buffer` must point to a live exclusive descriptor, and all element,
/// alignment, depth, and string-layout arguments must match its allocation.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_destroy_nested_owned_string(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    depth: u64,
    string_element_size: u64,
    string_alignment: u64,
) -> i32 {
    unsafe {
        buffer_destroy_nested_owned_string(
            buffer,
            element_size,
            alignment,
            depth,
            string_element_size,
            string_alignment,
        )
    }
    .code()
}

/// Destroys a nested Buffer whose final elements are owning record values
/// across the C ABI.
///
/// # Safety
///
/// The descriptor, depth, record layout and field table must describe a live
/// nested record buffer accepted by [`buffer_destroy_nested_record_fields`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_destroy_nested_record_fields(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    depth: u64,
    record_element_size: u64,
    record_alignment: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller contract.
    unsafe {
        buffer_destroy_nested_record_fields(
            buffer,
            element_size,
            alignment,
            depth,
            record_element_size,
            record_alignment,
            fields,
            field_count,
        )
    }
    .code()
}

/// Destroys carrier elements that own a selected Buffer payload across C ABI.
///
/// # Safety
///
/// The descriptor, carrier layout, tag, and selected payload layout must
/// describe initialized caller-owned storage.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_destroy_carrier_buffer(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    payload_offset: u64,
    payload_variant: u64,
    depth: u64,
    leaf_element_size: u64,
    leaf_alignment: u64,
) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller contract.
    unsafe {
        buffer_destroy_carrier_buffer(
            buffer,
            element_size,
            alignment,
            payload_offset,
            payload_variant,
            depth,
            leaf_element_size,
            leaf_alignment,
        )
    }
    .code()
}

/// Destroys a standalone owning carrier Buffer payload across the C ABI.
///
/// # Safety
///
/// The carrier pointer and payload layout must describe initialized
/// caller-owned storage.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_carrier_destroy_buffer(
    carrier: *mut u8,
    element_size: u64,
    payload_offset: u64,
    payload_variant: u64,
    depth: u64,
    leaf_element_size: u64,
    leaf_alignment: u64,
) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller contract.
    unsafe {
        carrier_destroy_buffer(
            carrier,
            element_size,
            payload_offset,
            payload_variant,
            depth,
            leaf_element_size,
            leaf_alignment,
        )
    }
    .code()
}

/// Destroys a two-branch owning carrier inside a Buffer across the C ABI.
///
/// # Safety
///
/// The pointer and all layout arguments must describe a live caller-owned
/// carrier buffer and its initialized branch descriptors.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_destroy_multi_carrier_buffer(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    payload_offset: u64,
    first_variant: u64,
    first_depth: u64,
    first_leaf_element_size: u64,
    first_leaf_alignment: u64,
    second_variant: u64,
    second_depth: u64,
    second_leaf_element_size: u64,
    second_leaf_alignment: u64,
) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller contract.
    unsafe {
        buffer_destroy_multi_carrier_buffer(
            buffer,
            element_size,
            alignment,
            payload_offset,
            first_variant,
            first_depth,
            first_leaf_element_size,
            first_leaf_alignment,
            second_variant,
            second_depth,
            second_leaf_element_size,
            second_leaf_alignment,
        )
    }
    .code()
}

/// Destroys a standalone two-branch owning carrier across the C ABI.
///
/// # Safety
///
/// The pointer and all layout arguments must describe initialized caller-owned
/// carrier storage and its branch descriptors.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_carrier_destroy_multi_buffer(
    carrier: *mut u8,
    element_size: u64,
    payload_offset: u64,
    first_variant: u64,
    first_depth: u64,
    first_leaf_element_size: u64,
    first_leaf_alignment: u64,
    second_variant: u64,
    second_depth: u64,
    second_leaf_element_size: u64,
    second_leaf_alignment: u64,
) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller contract.
    unsafe {
        carrier_destroy_multi_buffer(
            carrier,
            element_size,
            payload_offset,
            first_variant,
            first_depth,
            first_leaf_element_size,
            first_leaf_alignment,
            second_variant,
            second_depth,
            second_leaf_element_size,
            second_leaf_alignment,
        )
    }
    .code()
}

/// Destroys a multi-branch named enum carrier inside a Buffer across C ABI.
///
/// # Safety
///
/// The pointer, branch table, and layout arguments must describe initialized
/// caller-owned storage for the duration of this call.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_destroy_enum_carrier_buffer(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    payload_offset: u64,
    branches: *const CarrierDropBranchAbi,
    branch_count: u64,
) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller contract.
    unsafe {
        buffer_destroy_enum_carrier_buffer(
            buffer,
            element_size,
            alignment,
            payload_offset,
            branches,
            branch_count,
        )
    }
    .code()
}

/// Destroys a standalone multi-branch named enum carrier across C ABI.
///
/// # Safety
///
/// The pointer, branch table, and layout arguments must describe initialized
/// caller-owned storage for the duration of this call.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_carrier_destroy_enum_buffer(
    carrier: *mut u8,
    element_size: u64,
    payload_offset: u64,
    branches: *const CarrierDropBranchAbi,
    branch_count: u64,
) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller contract.
    unsafe {
        carrier_destroy_enum_buffer(
            carrier,
            element_size,
            payload_offset,
            branches,
            branch_count,
        )
    }
    .code()
}

/// Destroys multi-field named enum carriers inside a Buffer across the C ABI.
///
/// # Safety
///
/// The pointer, field table, and layout arguments must describe initialized
/// caller-owned storage for the duration of this call.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_destroy_enum_carrier_fields(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> i32 {
    unsafe {
        buffer_destroy_enum_carrier_fields(buffer, element_size, alignment, fields, field_count)
    }
    .code()
}

/// Destroys a standalone multi-field named enum carrier across the C ABI.
///
/// # Safety
///
/// The pointer, field table, and layout arguments must describe initialized
/// caller-owned storage for the duration of this call.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_carrier_destroy_enum_fields(
    carrier: *mut u8,
    element_size: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> i32 {
    unsafe { carrier_destroy_enum_fields(carrier, element_size, fields, field_count) }.code()
}

/// Destroys owning Buffer fields of record elements inside a Buffer across C
/// ABI.
///
/// # Safety
///
/// The descriptor, element layout and field table must describe live
/// caller-owned record storage.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_destroy_record_fields(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> i32 {
    unsafe { buffer_destroy_record_fields(buffer, element_size, alignment, fields, field_count) }
        .code()
}

/// Destroys owning Buffer fields of one standalone record across C ABI.
///
/// # Safety
///
/// The record pointer, element size and field table must describe initialized
/// caller-owned storage.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_carrier_destroy_record_fields(
    record: *mut u8,
    element_size: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> i32 {
    unsafe { carrier_destroy_record_fields(record, element_size, fields, field_count) }.code()
}

/// Removes one owning record from a Buffer, disposing its owning fields and
/// compacting later elements leftward across the C ABI.
///
/// # Safety
///
/// `buffer`, `fields` and all layout values must describe initialized,
/// caller-owned storage with valid owning-field descriptors.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_remove_drop_record_fields_status(
    buffer: *mut Buffer,
    index: u64,
    element_size: u64,
    alignment: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> i32 {
    unsafe {
        buffer_remove_drop_record_fields(
            buffer,
            index,
            element_size,
            alignment,
            fields,
            field_count,
        )
    }
    .code()
}

/// Boolean convenience wrapper for
/// [`jadren_rt_buffer_remove_drop_record_fields_status`].
///
/// # Safety
///
/// The arguments must satisfy the safety contract of the status wrapper.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_remove_drop_record_fields(
    buffer: *mut Buffer,
    index: u64,
    element_size: u64,
    alignment: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> i32 {
    let status = unsafe {
        jadren_rt_buffer_remove_drop_record_fields_status(
            buffer,
            index,
            element_size,
            alignment,
            fields,
            field_count,
        )
    };
    i32::from(status == BufferStatus::Ok.code())
}

/// Removes one nested owning record chain across the C ABI.
///
/// # Safety
///
/// `buffer`, `fields`, `depth` and all layout values must describe initialized,
/// caller-owned nested storage with valid owning-field descriptors.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_remove_drop_nested_record_fields_status(
    buffer: *mut Buffer,
    index: u64,
    element_size: u64,
    alignment: u64,
    depth: u64,
    record_element_size: u64,
    record_alignment: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> i32 {
    unsafe {
        buffer_remove_drop_nested_record_fields(
            buffer,
            index,
            element_size,
            alignment,
            depth,
            record_element_size,
            record_alignment,
            fields,
            field_count,
        )
    }
    .code()
}

/// Boolean convenience wrapper for nested owning record removal.
///
/// # Safety
///
/// The arguments must satisfy the safety contract of the nested status wrapper.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_remove_drop_nested_record_fields(
    buffer: *mut Buffer,
    index: u64,
    element_size: u64,
    alignment: u64,
    depth: u64,
    record_element_size: u64,
    record_alignment: u64,
    fields: *const CarrierDropFieldAbi,
    field_count: u64,
) -> i32 {
    let status = unsafe {
        jadren_rt_buffer_remove_drop_nested_record_fields_status(
            buffer,
            index,
            element_size,
            alignment,
            depth,
            record_element_size,
            record_alignment,
            fields,
            field_count,
        )
    };
    i32::from(status == BufferStatus::Ok.code())
}

/// Removes one copy-safe element from a Buffer without returning it across
/// the C ABI.
///
/// # Safety
///
/// `buffer` must point to a valid initialized Buffer and the element layout
/// must match its initialized elements.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_remove_drop_status(
    buffer: *mut Buffer,
    index: u64,
    element_size: u64,
    alignment: u64,
) -> i32 {
    unsafe { buffer_remove_drop(buffer, index, element_size, alignment) }.code()
}

/// Boolean convenience wrapper for [`jadren_rt_buffer_remove_drop_status`].
///
/// # Safety
///
/// The arguments must satisfy the safety contract of the status wrapper.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_remove_drop(
    buffer: *mut Buffer,
    index: u64,
    element_size: u64,
    alignment: u64,
) -> i32 {
    let status =
        unsafe { jadren_rt_buffer_remove_drop_status(buffer, index, element_size, alignment) };
    i32::from(status == BufferStatus::Ok.code())
}

/// Removes and destroys one `OwnedString` element across the C ABI.
///
/// # Safety
///
/// `buffer` must be a live exclusive buffer header containing valid owned
/// string descriptors, and `index` and the supplied layout must be valid for
/// that buffer.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_remove_drop_owned_string_status(
    buffer: *mut Buffer,
    index: u64,
    element_size: u64,
    alignment: u64,
) -> i32 {
    unsafe { buffer_remove_drop_owned_string(buffer, index, element_size, alignment) }.code()
}

/// Boolean convenience wrapper for
/// [`jadren_rt_buffer_remove_drop_owned_string_status`].
///
/// # Safety
///
/// The caller must uphold the same buffer, index and layout contract as the
/// status function.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_remove_drop_owned_string(
    buffer: *mut Buffer,
    index: u64,
    element_size: u64,
    alignment: u64,
) -> i32 {
    let status = unsafe {
        jadren_rt_buffer_remove_drop_owned_string_status(buffer, index, element_size, alignment)
    };
    i32::from(status == BufferStatus::Ok.code())
}

/// Moves one owning record into caller-owned output storage across the C ABI.
///
/// # Safety
///
/// `buffer` and `output` must be valid exclusive storage for the supplied
/// element layout, and `index` must identify an initialized element.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_remove_move_into_status(
    buffer: *mut Buffer,
    index: u64,
    output: *mut u8,
    element_size: u64,
    alignment: u64,
) -> i32 {
    unsafe { buffer_remove_move_into(buffer, index, output, element_size, alignment) }.code()
}

/// Boolean convenience wrapper for
/// [`jadren_rt_buffer_remove_move_into_status`].
///
/// # Safety
///
/// The caller must uphold the same buffer, output, index and layout contract
/// as the status function.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_remove_move_into(
    buffer: *mut Buffer,
    index: u64,
    output: *mut u8,
    element_size: u64,
    alignment: u64,
) -> i32 {
    let status = unsafe {
        jadren_rt_buffer_remove_move_into_status(buffer, index, output, element_size, alignment)
    };
    i32::from(status == BufferStatus::Ok.code())
}

/// Moves one caller-owned value into a generic Buffer at an index across the
/// C ABI without passing the aggregate by value.
///
/// # Safety
///
/// `buffer` and `source` must be valid exclusive storage for the supplied
/// element layout, and `index` must be within the insertion range.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_insert_move_from_status(
    buffer: *mut Buffer,
    index: u64,
    source: *mut u8,
    element_size: u64,
    alignment: u64,
) -> i32 {
    unsafe { buffer_insert_move_from(buffer, index, source, element_size, alignment) }.code()
}

/// Boolean convenience wrapper for
/// [`jadren_rt_buffer_insert_move_from_status`].
///
/// # Safety
///
/// The caller must uphold the same buffer, source, index and layout contract
/// as the status function.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_insert_move_from(
    buffer: *mut Buffer,
    index: u64,
    source: *mut u8,
    element_size: u64,
    alignment: u64,
) -> i32 {
    let status = unsafe {
        jadren_rt_buffer_insert_move_from_status(buffer, index, source, element_size, alignment)
    };
    i32::from(status == BufferStatus::Ok.code())
}

/// Moves the last owning element into caller-owned output storage across the
/// C ABI. The raw output pointer keeps the contract valid for arbitrary
/// already-supported element layouts.
///
/// # Safety
///
/// `buffer` and `output` must be valid exclusive storage for the supplied
/// element layout, and the buffer must contain at least one initialized item.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_pop_move_into_status(
    buffer: *mut Buffer,
    output: *mut u8,
    element_size: u64,
    alignment: u64,
) -> i32 {
    unsafe { buffer_pop_move_into(buffer, output, element_size, alignment) }.code()
}

/// Boolean convenience wrapper for
/// [`jadren_rt_buffer_pop_move_into_status`].
///
/// # Safety
///
/// The caller must uphold the same buffer, output and layout contract as the
/// status function.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_pop_move_into(
    buffer: *mut Buffer,
    output: *mut u8,
    element_size: u64,
    alignment: u64,
) -> i32 {
    let status =
        unsafe { jadren_rt_buffer_pop_move_into_status(buffer, output, element_size, alignment) };
    i32::from(status == BufferStatus::Ok.code())
}

/// Moves the last initialized nested buffer across the C ABI.
///
/// # Safety
///
/// The descriptor and nested element layouts must describe a live owning
/// nested buffer.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_pop(
    buffer: *mut Buffer,
    element_size: u64,
    alignment: u64,
    nested_element_size: u64,
    nested_alignment: u64,
) -> BufferResult {
    // SAFETY: this ABI boundary exposes the documented caller contract.
    unsafe {
        buffer_pop(
            buffer,
            element_size,
            alignment,
            nested_element_size,
            nested_alignment,
        )
    }
}

/// Moves an arbitrary nested buffer element across the C ABI.
///
/// # Safety
///
/// The descriptor, index, and nested element layouts must describe a live
/// owning nested buffer.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_remove_move(
    buffer: *mut Buffer,
    index: u64,
    element_size: u64,
    alignment: u64,
    nested_element_size: u64,
    nested_alignment: u64,
) -> BufferResult {
    // SAFETY: this ABI boundary exposes the documented caller contract.
    unsafe {
        buffer_remove_move(
            buffer,
            index,
            element_size,
            alignment,
            nested_element_size,
            nested_alignment,
        )
    }
}

/// Creates a checked non-owning slice from a buffer across the C ABI.
///
/// # Safety
///
/// The descriptor and element layout must satisfy [`buffer_slice`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_buffer_slice(
    buffer: *const Buffer,
    element_size: u64,
    alignment: u64,
    start: u64,
    count: u64,
) -> SliceResult {
    // SAFETY: this ABI boundary exposes the same documented caller contract.
    unsafe { buffer_slice(buffer, element_size, alignment, start, count) }
}

/// Creates a checked non-owning subslice across the C ABI.
///
/// # Safety
///
/// The descriptor and element layout must satisfy [`slice_subslice`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_slice_subslice(
    slice: *const Slice,
    element_size: u64,
    alignment: u64,
    start: u64,
    count: u64,
) -> SliceResult {
    // SAFETY: this ABI boundary exposes the same documented caller contract.
    unsafe { slice_subslice(slice, element_size, alignment, start, count) }
}

/// Creates an empty owning UTF-8 string across the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_string_create(capacity: u64) -> Utf8StringResult {
    string_create(capacity)
}

/// Copies and validates a borrowed UTF-8 byte range across the C ABI.
///
/// # Safety
///
/// When `length` is nonzero, `bytes` must point to a live readable range for
/// the duration of this call.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_string_from_utf8(
    bytes: *const u8,
    length: u64,
) -> Utf8StringResult {
    // SAFETY: the C caller owns the documented borrowed byte-range contract.
    unsafe { string_from_utf8(bytes, length) }
}

/// Reserves UTF-8 string capacity across the C ABI.
///
/// # Safety
///
/// The descriptor must satisfy [`string_reserve`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_string_reserve(
    string: *mut Utf8String,
    minimum_capacity: u64,
) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller contract.
    unsafe { string_reserve(string, minimum_capacity) }.code()
}

/// Appends a validated UTF-8 byte range across the C ABI.
///
/// # Safety
///
/// The descriptor and borrowed source must satisfy [`string_append_utf8`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_string_append_utf8(
    string: *mut Utf8String,
    bytes: *const u8,
    length: u64,
) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller contract.
    unsafe { string_append_utf8(string, bytes, length) }.code()
}

/// Clears the logical UTF-8 string contents across the C ABI.
///
/// # Safety
///
/// The descriptor must satisfy [`string_clear`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_string_clear(string: *mut Utf8String) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller contract.
    unsafe { string_clear(string) }.code()
}

/// Destroys an owning UTF-8 string across the C ABI.
///
/// # Safety
///
/// The descriptor must satisfy [`string_destroy`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_string_destroy(string: *mut Utf8String) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller contract.
    unsafe { string_destroy(string) }.code()
}

/// Destroys an `OwnedString` value emitted by the language lowering.
///
/// The language backend emits this symbol for automatic cleanup instead of
/// relying on a host-side destructor. It intentionally shares the validated
/// UTF-8 runtime header with the existing explicit string API.
///
/// # Safety
///
/// The descriptor must satisfy [`string_destroy`].
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jadren_rt_owned_string_destroy(string: *mut Utf8String) -> i32 {
    // SAFETY: this ABI boundary exposes the documented caller contract.
    unsafe { string_destroy(string) }.code()
}

/// Exposes allocation-free `Float32` absolute value through the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_math_abs_f32(value: f32) -> f32 {
    math_abs_f32(value)
}

/// Exposes allocation-free `Float64` absolute value through the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_math_abs_f64(value: f64) -> f64 {
    math_abs_f64(value)
}

/// Exposes allocation-free `Float32` square root through the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_math_sqrt_f32(value: f32) -> f32 {
    math_sqrt_f32(value)
}

/// Exposes allocation-free `Float64` square root through the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_math_sqrt_f64(value: f64) -> f64 {
    math_sqrt_f64(value)
}

/// Exposes allocation-free `Float32` arc-cosine through the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_math_acos_f32(value: f32) -> f32 {
    math_acos_f32(value)
}

/// Exposes allocation-free `Float32` sine through the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_math_sin_f32(value: f32) -> f32 {
    math_sin_f32(value)
}

/// Exposes allocation-free `Float32` cosine through the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_math_cos_f32(value: f32) -> f32 {
    math_cos_f32(value)
}

/// Exposes allocation-free `Float32` floor through the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_math_floor_f32(value: f32) -> f32 {
    math_floor_f32(value)
}

/// Exposes allocation-free `Float64` floor through the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_math_floor_f64(value: f64) -> f64 {
    math_floor_f64(value)
}

/// Exposes allocation-free `Float32` ceil through the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_math_ceil_f32(value: f32) -> f32 {
    math_ceil_f32(value)
}

/// Exposes allocation-free `Float64` ceil through the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_math_ceil_f64(value: f64) -> f64 {
    math_ceil_f64(value)
}

/// Adds two `Float2` values across the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_float2_add(left: Float2, right: Float2) -> Float2 {
    float2_add(left, right)
}

/// Adds two `Float3` values across the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_float3_add(left: Float3, right: Float3) -> Float3 {
    float3_add(left, right)
}

/// Adds two `Float4` values across the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_float4_add(left: Float4, right: Float4) -> Float4 {
    float4_add(left, right)
}

/// Returns a `Float3` dot product across the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_float3_dot(left: Float3, right: Float3) -> f32 {
    float3_dot(left, right)
}

/// Returns the identity quaternion across the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_quaternion_identity() -> Quaternion {
    quaternion_identity()
}

/// Computes shortest-arc spherical interpolation across the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_quaternion_slerp_unclamped(
    left: Quaternion,
    right: Quaternion,
    weight: f32,
) -> Quaternion {
    quaternion_slerp_unclamped(left, right, weight)
}

/// Returns the row-major 4×4 identity matrix across the C ABI.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn jadren_rt_matrix4_identity() -> Matrix4 {
    matrix4_identity()
}

const fn runtime_build_id() -> u64 {
    let hash = fnv1a64(
        concat!("jadren-runtime-v1;package=", env!("CARGO_PKG_VERSION")).as_bytes(),
        0xcbf2_9ce4_8422_2325_u64,
    );
    let hash = fnv1a64(&RUNTIME_ABI_MAJOR.to_le_bytes(), hash);
    fnv1a64(&RUNTIME_ABI_MINOR.to_le_bytes(), hash)
}

const fn fnv1a64(bytes: &[u8], mut hash: u64) -> u64 {
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        index += 1;
    }
    hash
}

#[cfg(test)]
mod tests {
    use std::ffi::c_void;
    use std::mem::{align_of, size_of};
    use std::ptr;
    use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Mutex};
    use std::thread;

    use super::{
        AbiVersion, AllocatorStatus, Buffer, BufferStatus, CallbackStatus, CarrierDropBranchAbi,
        CarrierDropFieldAbi, Float2, Float3, Float4, Float8, LogLevel, Matrix4, PanicCode,
        Quaternion, RUNTIME_ABI_MAJOR, RUNTIME_ABI_MINOR, RUNTIME_BUILD_ID, RUNTIME_STATE,
        RuntimeState, RuntimeStatus, STATE_UNINITIALIZED, StringStatus, Utf8String,
        bounds_panic_info, buffer_clear_move_nested_owned_string, buffer_clear_move_owned_string,
        buffer_create, buffer_destroy, buffer_destroy_carrier_buffer,
        buffer_destroy_enum_carrier_buffer, buffer_destroy_enum_carrier_fields,
        buffer_destroy_multi_carrier_buffer, buffer_destroy_nested_buffer,
        buffer_destroy_nested_buffer_recursive, buffer_destroy_nested_owned_string,
        buffer_destroy_nested_record_fields, buffer_destroy_owned_string,
        buffer_destroy_record_fields, buffer_insert_move_from, buffer_pop, buffer_pop_move_into,
        buffer_remove_drop_nested_record_fields, buffer_remove_drop_owned_string,
        buffer_remove_drop_record_fields, buffer_remove_move, buffer_remove_move_into,
        buffer_reserve, buffer_resize, buffer_resize_move, buffer_resize_move_nested_owned_string,
        buffer_resize_move_nested_record_fields, buffer_resize_move_owned_string,
        buffer_resize_move_record_fields, buffer_slice, carrier_destroy_buffer,
        carrier_destroy_enum_buffer, carrier_destroy_enum_fields, carrier_destroy_multi_buffer,
        carrier_destroy_record_fields, float2_add, float3_add, float3_dot, float4_add, float8_add,
        initialize, jadren_rt_abi_version, jadren_rt_build_id, jadren_rt_initialize,
        jadren_rt_is_initialized, jadren_rt_region_allocate, jadren_rt_region_create,
        jadren_rt_region_destroy, log, math_abs_f32, math_abs_f64, math_acos_f32, math_ceil_f32,
        math_ceil_f64, math_cos_f32, math_floor_f32, math_floor_f64, math_sin_f32, math_sqrt_f32,
        math_sqrt_f64, matrix4_identity, profiler_begin_sample, profiler_counter,
        profiler_end_sample, quaternion_identity, quaternion_slerp_unclamped, region_allocate,
        region_create, region_destroy, runtime_state, set_callbacks, slice_subslice,
        string_append_utf8, string_clear, string_create, string_destroy, string_from_utf8,
        string_reserve, system_allocate, system_deallocate, system_reallocate,
    };

    static TEST_RUNTIME_LOCK: Mutex<()> = Mutex::new(());
    static TEST_LOG_COUNT: AtomicUsize = AtomicUsize::new(0);
    static TEST_BEGIN_COUNT: AtomicUsize = AtomicUsize::new(0);
    static TEST_END_COUNT: AtomicUsize = AtomicUsize::new(0);
    static TEST_COUNTER_COUNT: AtomicUsize = AtomicUsize::new(0);
    static TEST_LAST_LEVEL: AtomicU32 = AtomicU32::new(u32::MAX);
    static TEST_LAST_NAME: AtomicU64 = AtomicU64::new(0);
    static TEST_LAST_VALUE: AtomicI64 = AtomicI64::new(0);
    static TEST_LAST_CONTEXT: AtomicUsize = AtomicUsize::new(0);
    static TEST_FIRST_BYTE: AtomicU32 = AtomicU32::new(0);

    #[test]
    #[allow(unsafe_code)]
    fn record_field_activity_follows_tag_and_unconditional_sentinel() {
        let mut record = [0u8; 16];
        record[0] = 1;
        let active = CarrierDropFieldAbi {
            payload_variant: 1,
            payload_offset: 8,
            depth: 1,
            leaf_element_size: 4,
            leaf_alignment: 4,
        };
        let inactive = CarrierDropFieldAbi {
            payload_variant: 0,
            ..active
        };
        let unconditional = CarrierDropFieldAbi {
            payload_variant: u64::MAX,
            ..active
        };
        // SAFETY: each field points at an aligned payload inside `record` and
        // the helper only reads the four-byte carrier tag preceding it.
        unsafe {
            assert!(super::record_field_is_active(record.as_ptr(), active).unwrap());
            assert!(!super::record_field_is_active(record.as_ptr(), inactive).unwrap());
            assert!(super::record_field_is_active(record.as_ptr(), unconditional).unwrap());
        }
    }

    #[allow(unsafe_code)]
    unsafe extern "C" fn test_log_callback(
        level: u32,
        message: *const u8,
        message_length: u64,
        context: *mut c_void,
    ) {
        TEST_LOG_COUNT.fetch_add(1, Ordering::Relaxed);
        TEST_LAST_LEVEL.store(level, Ordering::Relaxed);
        TEST_LAST_CONTEXT.store(context as usize, Ordering::Relaxed);
        if message_length != 0 && !message.is_null() {
            // SAFETY: the runtime callback contract keeps the borrowed message
            // live for this synchronous invocation.
            TEST_FIRST_BYTE.store(unsafe { u32::from(*message) }, Ordering::Relaxed);
        }
    }

    #[allow(unsafe_code)]
    unsafe extern "C" fn test_begin_callback(name_id: u64, context: *mut c_void) {
        TEST_BEGIN_COUNT.fetch_add(1, Ordering::Relaxed);
        TEST_LAST_NAME.store(name_id, Ordering::Relaxed);
        TEST_LAST_CONTEXT.store(context as usize, Ordering::Relaxed);
    }

    #[allow(unsafe_code)]
    unsafe extern "C" fn test_end_callback(context: *mut c_void) {
        TEST_END_COUNT.fetch_add(1, Ordering::Relaxed);
        TEST_LAST_CONTEXT.store(context as usize, Ordering::Relaxed);
    }

    #[allow(unsafe_code)]
    unsafe extern "C" fn test_counter_callback(name_id: u64, value: i64, context: *mut c_void) {
        TEST_COUNTER_COUNT.fetch_add(1, Ordering::Relaxed);
        TEST_LAST_NAME.store(name_id, Ordering::Relaxed);
        TEST_LAST_VALUE.store(value, Ordering::Relaxed);
        TEST_LAST_CONTEXT.store(context as usize, Ordering::Relaxed);
    }

    #[test]
    fn exports_stable_abi_and_build_identity() {
        assert_eq!(AbiVersion::CURRENT.major, RUNTIME_ABI_MAJOR);
        assert_eq!(AbiVersion::CURRENT.minor, RUNTIME_ABI_MINOR);
        assert_eq!(jadren_rt_abi_version(), AbiVersion::CURRENT.packed());
        assert_eq!(jadren_rt_build_id(), RUNTIME_BUILD_ID);
        assert_ne!(RUNTIME_BUILD_ID, 0);
    }

    #[test]
    fn bounds_panic_info_has_stable_fixed_width_payload() {
        assert_eq!(PanicCode::BoundsCheck.code(), 1);
        assert_eq!(size_of::<super::PanicInfo>(), 24);
        assert_eq!(align_of::<super::PanicInfo>(), 8);
        assert_eq!(
            bounds_panic_info(17, 12),
            super::PanicInfo {
                code: 1,
                detail_a: 17,
                detail_b: 12,
            }
        );
    }

    #[test]
    fn status_codes_are_stable_across_ffi_boundaries() {
        assert_eq!(RuntimeStatus::Initialized as i32, 0);
        assert_eq!(RuntimeStatus::AlreadyInitialized as i32, 1);
        assert_eq!(RuntimeStatus::IncompatibleMajor as i32, -1);
        assert_eq!(RuntimeStatus::IncompatibleMinor as i32, -2);

        assert_eq!(CallbackStatus::Delivered.code(), 0);
        assert_eq!(CallbackStatus::Disabled.code(), 1);
        assert_eq!(CallbackStatus::RuntimeNotInitialized.code(), -10);
        assert_eq!(CallbackStatus::InvalidInput.code(), -30);

        assert_eq!(AllocatorStatus::Ok.code(), 0);
        assert_eq!(AllocatorStatus::RuntimeNotInitialized.code(), -10);
        assert_eq!(AllocatorStatus::InvalidSize.code(), -11);
        assert_eq!(AllocatorStatus::InvalidAlignment.code(), -12);
        assert_eq!(AllocatorStatus::SizeOverflow.code(), -13);
        assert_eq!(AllocatorStatus::OutOfMemory.code(), -14);
        assert_eq!(AllocatorStatus::NullPointer.code(), -15);

        assert_eq!(BufferStatus::Ok.code(), 0);
        assert_eq!(BufferStatus::Disabled.code(), 1);
        assert_eq!(BufferStatus::RuntimeNotInitialized.code(), -10);
        assert_eq!(BufferStatus::InvalidSize.code(), -11);
        assert_eq!(BufferStatus::InvalidAlignment.code(), -12);
        assert_eq!(BufferStatus::SizeOverflow.code(), -13);
        assert_eq!(BufferStatus::OutOfMemory.code(), -14);
        assert_eq!(BufferStatus::NullPointer.code(), -15);
        assert_eq!(BufferStatus::OutOfBounds.code(), -20);
        assert_eq!(BufferStatus::InvalidBuffer.code(), -21);

        assert_eq!(StringStatus::Ok.code(), 0);
        assert_eq!(StringStatus::Disabled.code(), 1);
        assert_eq!(StringStatus::RuntimeNotInitialized.code(), -10);
        assert_eq!(StringStatus::InvalidSize.code(), -11);
        assert_eq!(StringStatus::InvalidAlignment.code(), -12);
        assert_eq!(StringStatus::SizeOverflow.code(), -13);
        assert_eq!(StringStatus::OutOfMemory.code(), -14);
        assert_eq!(StringStatus::NullPointer.code(), -15);
        assert_eq!(StringStatus::InvalidUtf8.code(), -40);
        assert_eq!(StringStatus::InvalidString.code(), -41);

        assert_eq!(size_of::<super::AllocationResult>(), 16);
        assert_eq!(size_of::<super::BufferResult>(), 32);
        assert_eq!(size_of::<super::Utf8StringResult>(), 32);
    }

    #[test]
    #[allow(unsafe_code)]
    fn callbacks_dispatch_without_copying_or_allocating_payloads() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(
            set_callbacks(
                Some(test_log_callback),
                Some(test_begin_callback),
                Some(test_end_callback),
                Some(test_counter_callback),
                ptr::null_mut(),
            ),
            CallbackStatus::RuntimeNotInitialized
        );
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);
        assert_eq!(profiler_begin_sample(1), CallbackStatus::Disabled);
        assert_eq!(log(99, ptr::null(), 0), CallbackStatus::InvalidInput);
        assert_eq!(log(2, ptr::null(), 1), CallbackStatus::InvalidInput);

        let context = 0x1234usize as *mut c_void;
        assert_eq!(
            set_callbacks(
                Some(test_log_callback),
                Some(test_begin_callback),
                Some(test_end_callback),
                Some(test_counter_callback),
                context,
            ),
            CallbackStatus::Delivered
        );
        let message = b"hello";
        assert_eq!(
            log(2, message.as_ptr(), message.len() as u64),
            CallbackStatus::Delivered
        );
        assert_eq!(profiler_begin_sample(41), CallbackStatus::Delivered);
        assert_eq!(profiler_counter(41, -7), CallbackStatus::Delivered);
        assert_eq!(profiler_end_sample(), CallbackStatus::Delivered);
        assert_eq!(TEST_LOG_COUNT.load(Ordering::Relaxed), 1);
        assert_eq!(TEST_BEGIN_COUNT.load(Ordering::Relaxed), 1);
        assert_eq!(TEST_COUNTER_COUNT.load(Ordering::Relaxed), 1);
        assert_eq!(TEST_END_COUNT.load(Ordering::Relaxed), 1);
        assert_eq!(
            TEST_LAST_LEVEL.load(Ordering::Relaxed),
            LogLevel::Info.code()
        );
        assert_eq!(TEST_FIRST_BYTE.load(Ordering::Relaxed), u32::from(b'h'));
        assert_eq!(TEST_LAST_NAME.load(Ordering::Relaxed), 41);
        assert_eq!(TEST_LAST_VALUE.load(Ordering::Relaxed), -7);
        assert_eq!(TEST_LAST_CONTEXT.load(Ordering::Relaxed), context as usize);

        assert_eq!(
            set_callbacks(None, None, None, None, context),
            CallbackStatus::InvalidInput
        );
        assert_eq!(
            set_callbacks(None, None, None, None, ptr::null_mut()),
            CallbackStatus::Disabled
        );
        assert_eq!(
            log(2, message.as_ptr(), message.len() as u64),
            CallbackStatus::Disabled
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn buffers_reserve_resize_slice_and_destroy_preserve_layout() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(
            buffer_create(4, 4, 4).status,
            BufferStatus::RuntimeNotInitialized.code()
        );
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);
        assert_eq!(size_of::<Buffer>(), 24);
        assert_eq!(align_of::<Buffer>(), 8);
        assert_eq!(size_of::<super::Slice>(), 16);
        assert_eq!(align_of::<super::Slice>(), 8);
        assert_eq!(
            buffer_create(0, 4, 4).status,
            BufferStatus::InvalidSize.code()
        );
        assert_eq!(
            buffer_create(4, 3, 4).status,
            BufferStatus::InvalidAlignment.code()
        );
        assert_eq!(
            buffer_create(u64::MAX, 8, 2).status,
            BufferStatus::SizeOverflow.code()
        );

        let result = buffer_create(4, 4, 4);
        assert_eq!(result.status, BufferStatus::Ok.code());
        let mut buffer = result.buffer;
        assert!(!buffer.pointer.is_null());
        assert_eq!(buffer.length, 0);
        assert_eq!(buffer.capacity, 4);
        let bytes = buffer.pointer.cast::<u8>();
        for index in 0..16 {
            // SAFETY: the buffer owns four four-byte elements.
            unsafe { bytes.add(index).write((index ^ 0xa5) as u8) };
        }
        assert_eq!(
            unsafe { buffer_reserve(&mut buffer, 4, 4, 4) },
            BufferStatus::Ok
        );
        assert_eq!(
            unsafe { buffer_reserve(&mut buffer, 4, 4, 8) },
            BufferStatus::Ok
        );
        assert_eq!(buffer.capacity, 8);
        let resized_bytes = buffer.pointer.cast::<u8>();
        for index in 0..16 {
            // SAFETY: reserve preserves the first old allocation bytes.
            assert_eq!(
                unsafe { resized_bytes.add(index).read() },
                (index ^ 0xa5) as u8
            );
        }
        assert_eq!(unsafe { buffer_resize(&mut buffer, 6) }, BufferStatus::Ok);
        assert_eq!(buffer.length, 6);
        assert_eq!(
            unsafe { buffer_resize(&mut buffer, 9) },
            BufferStatus::OutOfBounds
        );
        let view = unsafe { buffer_slice(&buffer, 4, 4, 2, 3) };
        assert_eq!(view.status, BufferStatus::Ok.code());
        assert_eq!(view.slice.length, 3);
        assert_eq!(view.slice.pointer.addr(), buffer.pointer.addr() + 8);
        let nested = unsafe { slice_subslice(&view.slice, 4, 4, 1, 1) };
        assert_eq!(nested.status, BufferStatus::Ok.code());
        assert_eq!(nested.slice.pointer.addr(), view.slice.pointer.addr() + 4);
        assert_eq!(nested.slice.length, 1);
        assert_eq!(
            unsafe { buffer_slice(&buffer, 4, 4, 5, 2) }.status,
            BufferStatus::OutOfBounds.code()
        );
        assert_eq!(
            unsafe { buffer_destroy(&mut buffer, 4, 4) },
            BufferStatus::Ok
        );
        assert_eq!(buffer, Buffer::EMPTY);
        assert_eq!(
            unsafe { buffer_destroy(&mut buffer, 4, 4) },
            BufferStatus::Ok
        );

        let empty = buffer_create(4, 4, 0);
        assert_eq!(empty.status, BufferStatus::Ok.code());
        let mut empty = empty.buffer;
        assert_eq!(
            unsafe { buffer_destroy(&mut empty, 4, 4) },
            BufferStatus::Ok
        );
        let mut malformed = Buffer {
            pointer: ptr::null_mut(),
            length: 1,
            capacity: 1,
        };
        assert_eq!(
            unsafe { buffer_resize(&mut malformed, 0) },
            BufferStatus::InvalidBuffer
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn nested_buffer_destroy_releases_initialized_inner_descriptors() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let inner_result = buffer_create(4, 4, 2);
        assert_eq!(inner_result.status, BufferStatus::Ok.code());
        let mut inner = inner_result.buffer;
        assert_eq!(unsafe { buffer_resize(&mut inner, 1) }, BufferStatus::Ok);

        let outer_result =
            buffer_create(size_of::<Buffer>() as u64, align_of::<Buffer>() as u64, 1);
        assert_eq!(outer_result.status, BufferStatus::Ok.code());
        let mut outer = outer_result.buffer;
        // SAFETY: the outer allocation has one aligned Buffer descriptor.
        unsafe { outer.pointer.cast::<Buffer>().write(inner) };
        outer.length = 1;
        assert_eq!(
            unsafe {
                buffer_destroy_nested_buffer(
                    &mut outer,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    4,
                    4,
                )
            },
            BufferStatus::Ok
        );
        assert_eq!(outer, Buffer::EMPTY);
    }

    #[test]
    #[allow(unsafe_code)]
    fn owned_string_buffer_destroy_releases_initialized_string_descriptors() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let first = unsafe { string_from_utf8(b"Jadren".as_ptr(), 6) };
        let second = unsafe { string_from_utf8(b"UI".as_ptr(), 2) };
        assert_eq!(first.status, StringStatus::Ok.code());
        assert_eq!(second.status, StringStatus::Ok.code());

        let outer_result = buffer_create(
            size_of::<Utf8String>() as u64,
            align_of::<Utf8String>() as u64,
            2,
        );
        assert_eq!(outer_result.status, BufferStatus::Ok.code());
        let mut outer = outer_result.buffer;
        // SAFETY: the outer allocation has two aligned initialized string
        // descriptors, each transferred into the owning buffer.
        unsafe {
            let slots = outer.pointer.cast::<Utf8String>();
            slots.write(first.string);
            slots.add(1).write(second.string);
        }
        outer.length = 2;

        assert_eq!(
            unsafe {
                buffer_destroy_owned_string(
                    &mut outer,
                    size_of::<Utf8String>() as u64,
                    align_of::<Utf8String>() as u64,
                )
            },
            BufferStatus::Ok
        );
        assert_eq!(outer, Buffer::EMPTY);
    }

    #[test]
    #[allow(unsafe_code)]
    fn owned_string_buffer_resize_clear_and_remove_drop_preserve_ownership() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let first = unsafe { string_from_utf8(b"first".as_ptr(), 5) };
        let second = unsafe { string_from_utf8(b"second".as_ptr(), 6) };
        assert_eq!(first.status, StringStatus::Ok.code());
        assert_eq!(second.status, StringStatus::Ok.code());

        let result = buffer_create(
            size_of::<Utf8String>() as u64,
            align_of::<Utf8String>() as u64,
            0,
        );
        assert_eq!(result.status, BufferStatus::Ok.code());
        let mut outer = result.buffer;
        assert_eq!(
            unsafe {
                buffer_resize_move_owned_string(
                    &mut outer,
                    2,
                    size_of::<Utf8String>() as u64,
                    align_of::<Utf8String>() as u64,
                )
            },
            BufferStatus::Ok
        );
        unsafe {
            let slots = outer.pointer.cast::<Utf8String>();
            slots.write(first.string);
            slots.add(1).write(second.string);
        }
        assert_eq!(
            unsafe {
                buffer_resize_move_owned_string(
                    &mut outer,
                    1,
                    size_of::<Utf8String>() as u64,
                    align_of::<Utf8String>() as u64,
                )
            },
            BufferStatus::Ok
        );
        assert_eq!(outer.length, 1);
        assert_eq!(
            unsafe {
                buffer_clear_move_owned_string(
                    &mut outer,
                    size_of::<Utf8String>() as u64,
                    align_of::<Utf8String>() as u64,
                )
            },
            BufferStatus::Ok
        );
        assert_eq!(outer.length, 0);

        let third = unsafe { string_from_utf8(b"third".as_ptr(), 5) };
        let fourth = unsafe { string_from_utf8(b"fourth".as_ptr(), 6) };
        assert_eq!(third.status, StringStatus::Ok.code());
        assert_eq!(fourth.status, StringStatus::Ok.code());
        unsafe {
            let slots = outer.pointer.cast::<Utf8String>();
            slots.write(third.string);
            slots.add(1).write(fourth.string);
        }
        outer.length = 2;
        assert_eq!(
            unsafe {
                buffer_remove_drop_owned_string(
                    &mut outer,
                    0,
                    size_of::<Utf8String>() as u64,
                    align_of::<Utf8String>() as u64,
                )
            },
            BufferStatus::Ok
        );
        assert_eq!(outer.length, 1);
        assert_eq!(
            unsafe {
                buffer_destroy_owned_string(
                    &mut outer,
                    size_of::<Utf8String>() as u64,
                    align_of::<Utf8String>() as u64,
                )
            },
            BufferStatus::Ok
        );
        assert_eq!(outer, Buffer::EMPTY);
    }

    #[test]
    #[allow(unsafe_code)]
    fn recursive_nested_buffer_destroy_releases_multiple_descriptor_levels() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let leaf_result = buffer_create(4, 4, 1);
        assert_eq!(leaf_result.status, BufferStatus::Ok.code());
        let mut leaf = leaf_result.buffer;
        assert_eq!(unsafe { buffer_resize(&mut leaf, 1) }, BufferStatus::Ok);

        let middle_result =
            buffer_create(size_of::<Buffer>() as u64, align_of::<Buffer>() as u64, 1);
        assert_eq!(middle_result.status, BufferStatus::Ok.code());
        let mut middle = middle_result.buffer;
        // SAFETY: the middle allocation has one aligned Buffer descriptor.
        unsafe { middle.pointer.cast::<Buffer>().write(leaf) };
        middle.length = 1;

        let outer_result =
            buffer_create(size_of::<Buffer>() as u64, align_of::<Buffer>() as u64, 1);
        assert_eq!(outer_result.status, BufferStatus::Ok.code());
        let mut outer = outer_result.buffer;
        // SAFETY: the outer allocation has one aligned Buffer descriptor.
        unsafe { outer.pointer.cast::<Buffer>().write(middle) };
        outer.length = 1;

        assert_eq!(
            unsafe {
                buffer_destroy_nested_buffer_recursive(
                    &mut outer,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    2,
                    4,
                    4,
                )
            },
            BufferStatus::Ok
        );
        assert_eq!(outer, Buffer::EMPTY);
    }

    #[test]
    #[allow(unsafe_code)]
    fn nested_owned_string_resize_clear_and_destroy_releases_leaf_payloads() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let string_result = unsafe { string_from_utf8(b"nested".as_ptr(), 6) };
        assert_eq!(string_result.status, StringStatus::Ok.code());

        let inner_result = buffer_create(
            size_of::<Utf8String>() as u64,
            align_of::<Utf8String>() as u64,
            0,
        );
        assert_eq!(inner_result.status, BufferStatus::Ok.code());
        let mut inner = inner_result.buffer;
        assert_eq!(
            unsafe {
                buffer_resize_move_owned_string(
                    &mut inner,
                    1,
                    size_of::<Utf8String>() as u64,
                    align_of::<Utf8String>() as u64,
                )
            },
            BufferStatus::Ok
        );
        // SAFETY: the resized inner allocation owns one initialized string
        // descriptor and receives the result's owned payload exactly once.
        unsafe {
            inner
                .pointer
                .cast::<Utf8String>()
                .write(string_result.string)
        };

        let outer_result =
            buffer_create(size_of::<Buffer>() as u64, align_of::<Buffer>() as u64, 0);
        assert_eq!(outer_result.status, BufferStatus::Ok.code());
        let mut outer = outer_result.buffer;
        assert_eq!(
            unsafe {
                buffer_resize_move_nested_owned_string(
                    &mut outer,
                    1,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    1,
                    size_of::<Utf8String>() as u64,
                    align_of::<Utf8String>() as u64,
                )
            },
            BufferStatus::Ok
        );
        // SAFETY: the outer slot is an aligned Buffer descriptor transferred
        // from the inner owner; the nested cleanup now owns it.
        unsafe { outer.pointer.cast::<Buffer>().write(inner) };

        assert_eq!(
            unsafe {
                buffer_clear_move_nested_owned_string(
                    &mut outer,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    1,
                    size_of::<Utf8String>() as u64,
                    align_of::<Utf8String>() as u64,
                )
            },
            BufferStatus::Ok
        );
        assert_eq!(outer.length, 0);
        assert_eq!(
            unsafe {
                buffer_destroy_nested_owned_string(
                    &mut outer,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    1,
                    size_of::<Utf8String>() as u64,
                    align_of::<Utf8String>() as u64,
                )
            },
            BufferStatus::Ok
        );
        assert_eq!(outer, Buffer::EMPTY);
    }

    #[test]
    #[allow(unsafe_code)]
    fn carrier_buffer_destroy_releases_only_selected_option_payloads() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let leaf_result = buffer_create(4, 4, 1);
        assert_eq!(leaf_result.status, BufferStatus::Ok.code());
        let mut leaf = leaf_result.buffer;
        assert_eq!(unsafe { buffer_resize(&mut leaf, 1) }, BufferStatus::Ok);

        let outer_result = buffer_create(32, 8, 2);
        assert_eq!(outer_result.status, BufferStatus::Ok.code());
        let mut outer = outer_result.buffer;
        // SAFETY: each slot is a 32-byte Option<Buffer<Int32>> carrier with
        // tag at offset zero and the Buffer descriptor at offset eight.
        unsafe {
            let bytes = outer.pointer.cast::<u8>();
            bytes.cast::<u32>().write(1);
            bytes.add(8).cast::<Buffer>().write(leaf);
            bytes.add(32).cast::<u32>().write(0);
        }
        outer.length = 2;
        assert_eq!(
            unsafe { buffer_destroy_carrier_buffer(&mut outer, 32, 8, 8, 1, 1, 4, 4) },
            BufferStatus::Ok
        );
        assert_eq!(outer, Buffer::EMPTY);
    }

    #[test]
    #[allow(unsafe_code)]
    fn standalone_carrier_destroy_releases_selected_payload() {
        #[repr(C, align(8))]
        struct CarrierBytes([u8; 32]);

        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let leaf_result = buffer_create(4, 4, 1);
        assert_eq!(leaf_result.status, BufferStatus::Ok.code());
        let mut leaf = leaf_result.buffer;
        assert_eq!(unsafe { buffer_resize(&mut leaf, 1) }, BufferStatus::Ok);

        let mut carrier = CarrierBytes([0; 32]);
        // SAFETY: carrier is 8-byte aligned and models a named enum whose
        // owning payload is variant two.
        unsafe {
            carrier.0.as_mut_ptr().cast::<u32>().write(2);
            carrier.0.as_mut_ptr().add(8).cast::<Buffer>().write(leaf);
        }
        assert_eq!(
            unsafe { carrier_destroy_buffer(carrier.0.as_mut_ptr(), 32, 8, 2, 1, 4, 4) },
            BufferStatus::Ok
        );
        assert!(carrier.0.iter().all(|byte| *byte == 0));
    }

    #[test]
    #[allow(unsafe_code)]
    fn multi_carrier_buffer_destroy_selects_each_result_branch() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let first_result = buffer_create(4, 4, 1);
        assert_eq!(first_result.status, BufferStatus::Ok.code());
        let mut first = first_result.buffer;
        assert_eq!(unsafe { buffer_resize(&mut first, 1) }, BufferStatus::Ok);

        let second_leaf_result = buffer_create(4, 4, 1);
        assert_eq!(second_leaf_result.status, BufferStatus::Ok.code());
        let mut second_leaf = second_leaf_result.buffer;
        assert_eq!(
            unsafe { buffer_resize(&mut second_leaf, 1) },
            BufferStatus::Ok
        );
        let second_result =
            buffer_create(size_of::<Buffer>() as u64, align_of::<Buffer>() as u64, 1);
        assert_eq!(second_result.status, BufferStatus::Ok.code());
        let mut second = second_result.buffer;
        // SAFETY: the nested branch allocation has one aligned descriptor.
        unsafe { second.pointer.cast::<Buffer>().write(second_leaf) };
        second.length = 1;

        let outer_result = buffer_create(32, 8, 2);
        assert_eq!(outer_result.status, BufferStatus::Ok.code());
        let mut outer = outer_result.buffer;
        // SAFETY: each slot models Result<Buffer<Int32>, Buffer<Buffer<Int32>>>.
        unsafe {
            let bytes = outer.pointer.cast::<u8>();
            bytes.cast::<u32>().write(0);
            bytes.add(8).cast::<Buffer>().write(first);
            bytes.add(32).cast::<u32>().write(1);
            bytes.add(40).cast::<Buffer>().write(second);
        }
        outer.length = 2;
        assert_eq!(
            unsafe {
                buffer_destroy_multi_carrier_buffer(&mut outer, 32, 8, 8, 0, 1, 4, 4, 1, 2, 4, 4)
            },
            BufferStatus::Ok
        );
        assert_eq!(outer, Buffer::EMPTY);
    }

    #[test]
    #[allow(unsafe_code)]
    fn standalone_multi_carrier_destroy_selects_result_branch() {
        #[repr(C, align(8))]
        struct CarrierBytes([u8; 32]);

        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let leaf_result = buffer_create(4, 4, 1);
        assert_eq!(leaf_result.status, BufferStatus::Ok.code());
        let mut leaf = leaf_result.buffer;
        assert_eq!(unsafe { buffer_resize(&mut leaf, 1) }, BufferStatus::Ok);

        let mut carrier = CarrierBytes([0; 32]);
        // SAFETY: carrier is 8-byte aligned and models Result<Buffer<Int32>,
        // Buffer<Int32>> with the Error/tag-0 branch selected.
        unsafe {
            carrier.0.as_mut_ptr().cast::<u32>().write(0);
            carrier.0.as_mut_ptr().add(8).cast::<Buffer>().write(leaf);
        }
        assert_eq!(
            unsafe {
                carrier_destroy_multi_buffer(carrier.0.as_mut_ptr(), 32, 8, 0, 1, 4, 4, 1, 1, 4, 4)
            },
            BufferStatus::Ok
        );
        assert!(carrier.0.iter().all(|byte| *byte == 0));
    }

    #[test]
    #[allow(unsafe_code)]
    fn enum_carrier_branch_table_selects_multiple_named_variants() {
        #[repr(C, align(8))]
        struct CarrierBytes([u8; 32]);

        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let make_leaf = || {
            let result = buffer_create(4, 4, 1);
            assert_eq!(result.status, BufferStatus::Ok.code());
            let mut buffer = result.buffer;
            assert_eq!(unsafe { buffer_resize(&mut buffer, 1) }, BufferStatus::Ok);
            buffer
        };
        let first = make_leaf();
        let second = make_leaf();
        let third = make_leaf();
        let outer_result = buffer_create(32, 8, 4);
        assert_eq!(outer_result.status, BufferStatus::Ok.code());
        let mut outer = outer_result.buffer;
        // SAFETY: slots model Event { Idle, First(Buffer<Int32>),
        // Second(Buffer<Int32>), Third(Buffer<Int32>) } with a shared payload
        // offset of eight bytes. The first slot is copy-only and has no
        // descriptor.
        unsafe {
            let bytes = outer.pointer.cast::<u8>();
            bytes.cast::<u32>().write(0);
            bytes.add(32).cast::<u32>().write(1);
            bytes.add(40).cast::<Buffer>().write(first);
            bytes.add(64).cast::<u32>().write(2);
            bytes.add(72).cast::<Buffer>().write(second);
            bytes.add(96).cast::<u32>().write(3);
            bytes.add(104).cast::<Buffer>().write(third);
        }
        outer.length = 4;
        let branches = [
            CarrierDropBranchAbi {
                payload_variant: 1,
                depth: 1,
                leaf_element_size: 4,
                leaf_alignment: 4,
            },
            CarrierDropBranchAbi {
                payload_variant: 2,
                depth: 1,
                leaf_element_size: 4,
                leaf_alignment: 4,
            },
            CarrierDropBranchAbi {
                payload_variant: 3,
                depth: 1,
                leaf_element_size: 4,
                leaf_alignment: 4,
            },
        ];
        assert_eq!(
            unsafe {
                buffer_destroy_enum_carrier_buffer(
                    &mut outer,
                    32,
                    8,
                    8,
                    branches.as_ptr(),
                    branches.len() as u64,
                )
            },
            BufferStatus::Ok
        );
        assert_eq!(outer, Buffer::EMPTY);

        let standalone_leaf = make_leaf();
        let mut carrier = CarrierBytes([0; 32]);
        // SAFETY: the carrier selects owning variant three and stores a live
        // descriptor at the common payload offset.
        unsafe {
            carrier.0.as_mut_ptr().cast::<u32>().write(3);
            carrier
                .0
                .as_mut_ptr()
                .add(8)
                .cast::<Buffer>()
                .write(standalone_leaf);
        }
        assert_eq!(
            unsafe {
                carrier_destroy_enum_buffer(
                    carrier.0.as_mut_ptr(),
                    32,
                    8,
                    branches.as_ptr(),
                    branches.len() as u64,
                )
            },
            BufferStatus::Ok
        );
        assert!(carrier.0.iter().all(|byte| *byte == 0));
    }

    #[test]
    #[allow(unsafe_code)]
    fn enum_carrier_field_table_drops_multiple_fields_and_offsets() {
        #[repr(C, align(8))]
        struct CarrierBytes([u8; 40]);

        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let make_leaf = || {
            let result = buffer_create(4, 4, 1);
            assert_eq!(result.status, BufferStatus::Ok.code());
            let mut buffer = result.buffer;
            assert_eq!(unsafe { buffer_resize(&mut buffer, 1) }, BufferStatus::Ok);
            buffer
        };
        let first = make_leaf();
        let nested_leaf = make_leaf();
        let nested_result =
            buffer_create(size_of::<Buffer>() as u64, align_of::<Buffer>() as u64, 1);
        assert_eq!(nested_result.status, BufferStatus::Ok.code());
        let mut nested = nested_result.buffer;
        unsafe { nested.pointer.cast::<Buffer>().write(nested_leaf) };
        nested.length = 1;

        let outer_result = buffer_create(40, 8, 2);
        assert_eq!(outer_result.status, BufferStatus::Ok.code());
        let mut outer = outer_result.buffer;
        // Event { Pair(Buffer<Int32>, Int32),
        //         Nested(Int32, Buffer<Buffer<Int32>>) }.
        // The selected Buffer fields therefore sit at different offsets.
        unsafe {
            let bytes = outer.pointer.cast::<u8>();
            bytes.cast::<u32>().write(1);
            bytes.add(8).cast::<Buffer>().write(first);
            bytes.add(40).cast::<u32>().write(2);
            bytes.add(56).cast::<Buffer>().write(nested);
        }
        outer.length = 2;
        let fields = [
            CarrierDropFieldAbi {
                payload_variant: 1,
                payload_offset: 8,
                depth: 1,
                leaf_element_size: 4,
                leaf_alignment: 4,
            },
            CarrierDropFieldAbi {
                payload_variant: 2,
                payload_offset: 16,
                depth: 2,
                leaf_element_size: 4,
                leaf_alignment: 4,
            },
        ];
        assert_eq!(
            unsafe {
                buffer_destroy_enum_carrier_fields(
                    &mut outer,
                    40,
                    8,
                    fields.as_ptr(),
                    fields.len() as u64,
                )
            },
            BufferStatus::Ok
        );
        assert_eq!(outer, Buffer::EMPTY);

        let standalone_leaf = make_leaf();
        let mut carrier = CarrierBytes([0; 40]);
        unsafe {
            carrier.0.as_mut_ptr().cast::<u32>().write(1);
            carrier
                .0
                .as_mut_ptr()
                .add(8)
                .cast::<Buffer>()
                .write(standalone_leaf);
        }
        assert_eq!(
            unsafe {
                carrier_destroy_enum_fields(
                    carrier.0.as_mut_ptr(),
                    40,
                    fields.as_ptr(),
                    fields.len() as u64,
                )
            },
            BufferStatus::Ok
        );
        assert!(carrier.0.iter().all(|byte| *byte == 0));
    }

    #[test]
    #[allow(unsafe_code)]
    fn record_field_table_drops_direct_and_nested_buffer_fields() {
        #[repr(C, align(8))]
        struct RecordBytes([u8; 64]);

        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let make_leaf = || {
            let result = buffer_create(4, 4, 1);
            assert_eq!(result.status, BufferStatus::Ok.code());
            let mut buffer = result.buffer;
            assert_eq!(unsafe { buffer_resize(&mut buffer, 1) }, BufferStatus::Ok);
            buffer
        };
        let direct = make_leaf();
        let nested_leaf = make_leaf();
        let nested_result =
            buffer_create(size_of::<Buffer>() as u64, align_of::<Buffer>() as u64, 1);
        assert_eq!(nested_result.status, BufferStatus::Ok.code());
        let mut nested = nested_result.buffer;
        unsafe { nested.pointer.cast::<Buffer>().write(nested_leaf) };
        nested.length = 1;

        let outer_result = buffer_create(64, 8, 2);
        assert_eq!(outer_result.status, BufferStatus::Ok.code());
        let mut outer = outer_result.buffer;
        // One record with two owning fields at distinct offsets, modelling a
        // direct Buffer<Int32> followed by Buffer<Buffer<Int32>>.
        unsafe {
            outer.pointer.cast::<u8>().cast::<Buffer>().write(direct);
            outer
                .pointer
                .cast::<u8>()
                .add(32)
                .cast::<Buffer>()
                .write(nested);
        }
        outer.length = 1;
        let fields = [
            CarrierDropFieldAbi {
                payload_variant: u64::MAX,
                payload_offset: 0,
                depth: 1,
                leaf_element_size: 4,
                leaf_alignment: 4,
            },
            CarrierDropFieldAbi {
                payload_variant: u64::MAX,
                payload_offset: 32,
                depth: 2,
                leaf_element_size: 4,
                leaf_alignment: 4,
            },
        ];
        assert_eq!(
            unsafe {
                buffer_destroy_record_fields(
                    &mut outer,
                    64,
                    8,
                    fields.as_ptr(),
                    fields.len() as u64,
                )
            },
            BufferStatus::Ok
        );
        assert_eq!(outer, Buffer::EMPTY);

        let standalone_direct = make_leaf();
        let mut record = RecordBytes([0; 64]);
        unsafe {
            record
                .0
                .as_mut_ptr()
                .cast::<Buffer>()
                .write(standalone_direct);
        }
        assert_eq!(
            unsafe {
                carrier_destroy_record_fields(
                    record.0.as_mut_ptr(),
                    64,
                    fields.as_ptr(),
                    fields.len() as u64,
                )
            },
            BufferStatus::Ok
        );
        assert!(record.0.iter().all(|byte| *byte == 0));
    }

    #[test]
    #[allow(unsafe_code)]
    fn nested_record_field_table_drops_buffer_record_chain() {
        #[repr(C, align(8))]
        struct RecordBytes([u8; 32]);

        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let leaf_result = buffer_create(4, 4, 1);
        assert_eq!(leaf_result.status, BufferStatus::Ok.code());
        let mut leaf = leaf_result.buffer;
        assert_eq!(unsafe { buffer_resize(&mut leaf, 1) }, BufferStatus::Ok);

        let inner_result = buffer_create(size_of::<RecordBytes>() as u64, 8, 1);
        assert_eq!(inner_result.status, BufferStatus::Ok.code());
        let mut inner = inner_result.buffer;
        unsafe {
            let record = inner.pointer.cast::<u8>();
            record.add(8).cast::<Buffer>().write(leaf);
        }
        inner.length = 1;

        let outer_result =
            buffer_create(size_of::<Buffer>() as u64, align_of::<Buffer>() as u64, 1);
        assert_eq!(outer_result.status, BufferStatus::Ok.code());
        let mut outer = outer_result.buffer;
        unsafe { outer.pointer.cast::<Buffer>().write(inner) };
        outer.length = 1;

        let fields = [CarrierDropFieldAbi {
            payload_variant: u64::MAX,
            payload_offset: 8,
            depth: 1,
            leaf_element_size: 4,
            leaf_alignment: 4,
        }];
        assert_eq!(
            unsafe {
                buffer_destroy_nested_record_fields(
                    &mut outer,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    1,
                    size_of::<RecordBytes>() as u64,
                    align_of::<RecordBytes>() as u64,
                    fields.as_ptr(),
                    fields.len() as u64,
                )
            },
            BufferStatus::Ok
        );
        assert_eq!(outer, Buffer::EMPTY);
    }

    #[test]
    #[allow(unsafe_code)]
    fn nested_record_buffer_resize_move_zeroes_growth_and_drops_shrink() {
        #[repr(C, align(8))]
        struct RecordBytes([u8; 32]);

        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let leaf_result = buffer_create(4, 4, 1);
        assert_eq!(leaf_result.status, BufferStatus::Ok.code());
        let mut leaf = leaf_result.buffer;
        assert_eq!(unsafe { buffer_resize(&mut leaf, 1) }, BufferStatus::Ok);

        let inner_result = buffer_create(size_of::<RecordBytes>() as u64, 8, 1);
        assert_eq!(inner_result.status, BufferStatus::Ok.code());
        let mut inner = inner_result.buffer;
        unsafe {
            let record = inner.pointer.cast::<u8>();
            record.cast::<u32>().write(7);
            record.add(8).cast::<Buffer>().write(leaf);
        }
        inner.length = 1;

        let outer_result = buffer_create(size_of::<Buffer>() as u64, 8, 0);
        assert_eq!(outer_result.status, BufferStatus::Ok.code());
        let mut outer = outer_result.buffer;
        let fields = [CarrierDropFieldAbi {
            payload_variant: u64::MAX,
            payload_offset: 8,
            depth: 1,
            leaf_element_size: 4,
            leaf_alignment: 4,
        }];

        assert_eq!(
            unsafe {
                buffer_resize_move_nested_record_fields(
                    &mut outer,
                    1,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    1,
                    size_of::<RecordBytes>() as u64,
                    align_of::<RecordBytes>() as u64,
                    fields.as_ptr(),
                    fields.len() as u64,
                )
            },
            BufferStatus::Ok
        );
        unsafe { outer.pointer.cast::<Buffer>().write(inner) };
        assert_eq!(
            unsafe {
                buffer_resize_move_nested_record_fields(
                    &mut outer,
                    2,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    1,
                    size_of::<RecordBytes>() as u64,
                    align_of::<RecordBytes>() as u64,
                    fields.as_ptr(),
                    fields.len() as u64,
                )
            },
            BufferStatus::Ok
        );
        unsafe {
            assert_eq!(*outer.pointer.cast::<Buffer>().add(1), Buffer::EMPTY);
        }
        assert_eq!(
            unsafe {
                buffer_resize_move_nested_record_fields(
                    &mut outer,
                    0,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    1,
                    size_of::<RecordBytes>() as u64,
                    align_of::<RecordBytes>() as u64,
                    fields.as_ptr(),
                    fields.len() as u64,
                )
            },
            BufferStatus::Ok
        );
        assert_eq!(outer.length, 0);
        assert_eq!(
            unsafe {
                buffer_destroy(
                    &mut outer,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                )
            },
            BufferStatus::Ok
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn direct_record_buffer_resize_move_zeroes_growth_and_drops_shrink() {
        #[repr(C, align(8))]
        struct RecordBytes([u8; 32]);

        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let leaf_result = buffer_create(4, 4, 1);
        assert_eq!(leaf_result.status, BufferStatus::Ok.code());
        let mut leaf = leaf_result.buffer;
        assert_eq!(unsafe { buffer_resize(&mut leaf, 1) }, BufferStatus::Ok);

        let record_result = buffer_create(size_of::<RecordBytes>() as u64, 8, 0);
        assert_eq!(record_result.status, BufferStatus::Ok.code());
        let mut records = record_result.buffer;
        let fields = [CarrierDropFieldAbi {
            payload_variant: u64::MAX,
            payload_offset: 8,
            depth: 1,
            leaf_element_size: 4,
            leaf_alignment: 4,
        }];

        assert_eq!(
            unsafe {
                buffer_resize_move_record_fields(
                    &mut records,
                    1,
                    size_of::<RecordBytes>() as u64,
                    align_of::<RecordBytes>() as u64,
                    fields.as_ptr(),
                    fields.len() as u64,
                )
            },
            BufferStatus::Ok
        );
        unsafe {
            records
                .pointer
                .cast::<u8>()
                .add(8)
                .cast::<Buffer>()
                .write(leaf);
        }
        assert_eq!(
            unsafe {
                buffer_resize_move_record_fields(
                    &mut records,
                    2,
                    size_of::<RecordBytes>() as u64,
                    align_of::<RecordBytes>() as u64,
                    fields.as_ptr(),
                    fields.len() as u64,
                )
            },
            BufferStatus::Ok
        );
        unsafe {
            assert_eq!(
                records
                    .pointer
                    .cast::<u8>()
                    .add(size_of::<RecordBytes>() + 8)
                    .cast::<Buffer>()
                    .read(),
                Buffer::EMPTY
            );
        }
        assert_eq!(
            unsafe {
                buffer_resize_move_record_fields(
                    &mut records,
                    0,
                    size_of::<RecordBytes>() as u64,
                    align_of::<RecordBytes>() as u64,
                    fields.as_ptr(),
                    fields.len() as u64,
                )
            },
            BufferStatus::Ok
        );
        assert_eq!(records.length, 0);
        assert_eq!(
            unsafe {
                buffer_destroy(
                    &mut records,
                    size_of::<RecordBytes>() as u64,
                    align_of::<RecordBytes>() as u64,
                )
            },
            BufferStatus::Ok
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn buffer_remove_drop_record_fields_destroys_removed_owner_and_compacts() {
        #[repr(C, align(8))]
        struct RecordBytes([u8; 32]);

        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let first_result = buffer_create(4, 4, 1);
        let second_result = buffer_create(4, 4, 1);
        assert_eq!(first_result.status, BufferStatus::Ok.code());
        assert_eq!(second_result.status, BufferStatus::Ok.code());
        let mut first = first_result.buffer;
        let mut second = second_result.buffer;
        assert_eq!(unsafe { buffer_resize(&mut first, 1) }, BufferStatus::Ok);
        assert_eq!(unsafe { buffer_resize(&mut second, 1) }, BufferStatus::Ok);

        let outer_result = buffer_create(size_of::<RecordBytes>() as u64, 8, 2);
        assert_eq!(outer_result.status, BufferStatus::Ok.code());
        let mut outer = outer_result.buffer;
        unsafe {
            let first_record = outer.pointer.cast::<u8>();
            first_record.cast::<u32>().write(11);
            first_record.add(8).cast::<Buffer>().write(first);
            let second_record = first_record.add(size_of::<RecordBytes>());
            second_record.cast::<u32>().write(22);
            second_record.add(8).cast::<Buffer>().write(second);
        }
        outer.length = 2;
        let fields = [CarrierDropFieldAbi {
            payload_variant: u64::MAX,
            payload_offset: 8,
            depth: 1,
            leaf_element_size: 4,
            leaf_alignment: 4,
        }];

        assert_eq!(
            unsafe {
                buffer_remove_drop_record_fields(
                    &mut outer,
                    0,
                    size_of::<RecordBytes>() as u64,
                    align_of::<RecordBytes>() as u64,
                    fields.as_ptr(),
                    fields.len() as u64,
                )
            },
            BufferStatus::Ok
        );
        assert_eq!(outer.length, 1);
        unsafe {
            assert_eq!(outer.pointer.cast::<u32>().read(), 22);
        }
        assert_eq!(
            unsafe {
                buffer_remove_drop_record_fields(
                    &mut outer,
                    1,
                    size_of::<RecordBytes>() as u64,
                    align_of::<RecordBytes>() as u64,
                    fields.as_ptr(),
                    fields.len() as u64,
                )
            },
            BufferStatus::OutOfBounds
        );
        assert_eq!(
            unsafe {
                buffer_destroy_record_fields(
                    &mut outer,
                    size_of::<RecordBytes>() as u64,
                    align_of::<RecordBytes>() as u64,
                    fields.as_ptr(),
                    fields.len() as u64,
                )
            },
            BufferStatus::Ok
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn buffer_remove_drop_nested_record_fields_destroys_chain_and_compacts() {
        #[repr(C, align(8))]
        struct RecordBytes([u8; 32]);

        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let make_inner = |id: u32| {
            let leaf_result = buffer_create(4, 4, 1);
            assert_eq!(leaf_result.status, BufferStatus::Ok.code());
            let mut leaf = leaf_result.buffer;
            assert_eq!(unsafe { buffer_resize(&mut leaf, 1) }, BufferStatus::Ok);
            let inner_result = buffer_create(size_of::<RecordBytes>() as u64, 8, 1);
            assert_eq!(inner_result.status, BufferStatus::Ok.code());
            let mut inner = inner_result.buffer;
            unsafe {
                let record = inner.pointer.cast::<u8>();
                record.cast::<u32>().write(id);
                record.add(8).cast::<Buffer>().write(leaf);
            }
            inner.length = 1;
            inner
        };

        let first = make_inner(11);
        let second = make_inner(22);
        let outer_result =
            buffer_create(size_of::<Buffer>() as u64, align_of::<Buffer>() as u64, 2);
        assert_eq!(outer_result.status, BufferStatus::Ok.code());
        let mut outer = outer_result.buffer;
        unsafe {
            outer.pointer.cast::<Buffer>().write(first);
            outer.pointer.cast::<Buffer>().add(1).write(second);
        }
        outer.length = 2;

        let fields = [CarrierDropFieldAbi {
            payload_variant: u64::MAX,
            payload_offset: 8,
            depth: 1,
            leaf_element_size: 4,
            leaf_alignment: 4,
        }];
        assert_eq!(
            unsafe {
                buffer_remove_drop_nested_record_fields(
                    &mut outer,
                    0,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    1,
                    size_of::<RecordBytes>() as u64,
                    align_of::<RecordBytes>() as u64,
                    fields.as_ptr(),
                    fields.len() as u64,
                )
            },
            BufferStatus::Ok
        );
        assert_eq!(outer.length, 1);
        unsafe {
            let remaining = outer.pointer.cast::<Buffer>().read();
            assert_eq!(remaining.pointer.cast::<u8>().cast::<u32>().read(), 22);
            outer.pointer.cast::<Buffer>().write(remaining);
        }
        assert_eq!(
            unsafe {
                buffer_remove_drop_nested_record_fields(
                    &mut outer,
                    0,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    1,
                    size_of::<RecordBytes>() as u64,
                    align_of::<RecordBytes>() as u64,
                    fields.as_ptr(),
                    fields.len() as u64,
                )
            },
            BufferStatus::Ok
        );
        assert_eq!(outer.length, 0);
        assert_eq!(
            unsafe {
                buffer_remove_drop_nested_record_fields(
                    &mut outer,
                    0,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    1,
                    size_of::<RecordBytes>() as u64,
                    align_of::<RecordBytes>() as u64,
                    fields.as_ptr(),
                    fields.len() as u64,
                )
            },
            BufferStatus::OutOfBounds
        );
        assert_eq!(
            unsafe {
                buffer_destroy(
                    &mut outer,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                )
            },
            BufferStatus::Ok
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn nested_buffer_pop_moves_last_descriptor_and_reports_empty() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let inner_result = buffer_create(4, 4, 1);
        assert_eq!(inner_result.status, BufferStatus::Ok.code());
        let mut inner = inner_result.buffer;
        assert_eq!(unsafe { buffer_resize(&mut inner, 1) }, BufferStatus::Ok);

        let outer_result =
            buffer_create(size_of::<Buffer>() as u64, align_of::<Buffer>() as u64, 1);
        assert_eq!(outer_result.status, BufferStatus::Ok.code());
        let mut outer = outer_result.buffer;
        // SAFETY: the outer allocation has one aligned Buffer descriptor.
        unsafe { outer.pointer.cast::<Buffer>().write(inner) };
        outer.length = 1;

        let popped = unsafe {
            buffer_pop(
                &mut outer,
                size_of::<Buffer>() as u64,
                align_of::<Buffer>() as u64,
                4,
                4,
            )
        };
        assert_eq!(popped.status, BufferStatus::Ok.code());
        assert_eq!(outer.length, 0);
        let mut popped = popped.buffer;
        assert_eq!(
            unsafe { buffer_destroy(&mut popped, 4, 4) },
            BufferStatus::Ok
        );
        assert_eq!(
            unsafe {
                buffer_pop(
                    &mut outer,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    4,
                    4,
                )
            }
            .status,
            BufferStatus::OutOfBounds.code()
        );
        assert_eq!(
            unsafe {
                buffer_destroy_nested_buffer(
                    &mut outer,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    4,
                    4,
                )
            },
            BufferStatus::Ok
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn nested_buffer_resize_move_zeroes_growth_and_drops_shrink() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let inner_result = buffer_create(4, 4, 1);
        assert_eq!(inner_result.status, BufferStatus::Ok.code());
        let mut inner = inner_result.buffer;
        assert_eq!(unsafe { buffer_resize(&mut inner, 1) }, BufferStatus::Ok);

        let mut outer =
            buffer_create(size_of::<Buffer>() as u64, align_of::<Buffer>() as u64, 0).buffer;
        // SAFETY: resize_move reserves and initializes three aligned slots.
        outer.pointer = ptr::null_mut();
        assert_eq!(
            unsafe {
                buffer_resize_move(
                    &mut outer,
                    1,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    1,
                    4,
                    4,
                )
            },
            BufferStatus::Ok
        );
        // SAFETY: the first slot is initialized by the caller-owned move.
        unsafe { outer.pointer.cast::<Buffer>().write(inner) };
        assert_eq!(
            unsafe {
                buffer_resize_move(
                    &mut outer,
                    3,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    1,
                    4,
                    4,
                )
            },
            BufferStatus::Ok
        );
        assert_eq!(outer.length, 3);
        // SAFETY: growth zeroes newly exposed native Buffer descriptors.
        unsafe {
            let slots = outer.pointer.cast::<Buffer>();
            assert_eq!(*slots.add(1), Buffer::EMPTY);
            assert_eq!(*slots.add(2), Buffer::EMPTY);
        }
        assert_eq!(
            unsafe {
                buffer_resize_move(
                    &mut outer,
                    0,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    1,
                    4,
                    4,
                )
            },
            BufferStatus::Ok
        );
        assert_eq!(outer.length, 0);
        assert_eq!(
            unsafe {
                buffer_destroy(
                    &mut outer,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                )
            },
            BufferStatus::Ok
        );

        let leaf = buffer_create(4, 4, 1).buffer;
        let mut middle =
            buffer_create(size_of::<Buffer>() as u64, align_of::<Buffer>() as u64, 1).buffer;
        let mut recursive_outer =
            buffer_create(size_of::<Buffer>() as u64, align_of::<Buffer>() as u64, 1).buffer;
        // SAFETY: each allocation has one native Buffer descriptor slot.
        unsafe {
            middle.pointer.cast::<Buffer>().write(leaf);
            middle.length = 1;
            recursive_outer.pointer.cast::<Buffer>().write(middle);
            recursive_outer.length = 1;
            assert_eq!(
                buffer_resize_move(
                    &mut recursive_outer,
                    0,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    2,
                    4,
                    4,
                ),
                BufferStatus::Ok
            );
        }
        assert_eq!(recursive_outer.length, 0);
        assert_eq!(
            unsafe {
                buffer_destroy(
                    &mut recursive_outer,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                )
            },
            BufferStatus::Ok
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn nested_buffer_remove_move_closes_gap_and_preserves_owners() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let first = buffer_create(4, 4, 1).buffer;
        let second = buffer_create(4, 4, 1).buffer;
        let third = buffer_create(4, 4, 1).buffer;
        let first_pointer = first.pointer;
        let second_pointer = second.pointer;
        let third_pointer = third.pointer;
        let outer_result =
            buffer_create(size_of::<Buffer>() as u64, align_of::<Buffer>() as u64, 3);
        assert_eq!(outer_result.status, BufferStatus::Ok.code());
        let mut outer = outer_result.buffer;
        // SAFETY: the outer allocation has three aligned Buffer descriptors.
        unsafe {
            let slots = outer.pointer.cast::<Buffer>();
            slots.write(first);
            slots.add(1).write(second);
            slots.add(2).write(third);
        }
        outer.length = 3;
        let removed = unsafe {
            buffer_remove_move(
                &mut outer,
                1,
                size_of::<Buffer>() as u64,
                align_of::<Buffer>() as u64,
                4,
                4,
            )
        };
        assert_eq!(removed.status, BufferStatus::Ok.code());
        assert_eq!(outer.length, 2);
        assert_eq!(removed.buffer.pointer, second_pointer);
        // SAFETY: the remaining initialized slots are valid Buffer descriptors.
        unsafe {
            let slots = outer.pointer.cast::<Buffer>();
            assert_eq!((*slots).pointer, first_pointer);
            assert_eq!((*slots.add(1)).pointer, third_pointer);
        }
        let mut removed = removed.buffer;
        assert_eq!(
            unsafe { buffer_destroy(&mut removed, 4, 4) },
            BufferStatus::Ok
        );
        assert_eq!(
            unsafe {
                buffer_destroy_nested_buffer(
                    &mut outer,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                    4,
                    4,
                )
            },
            BufferStatus::Ok
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn nested_buffer_pop_move_into_transfers_last_owner_without_aggregate_return() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let first = buffer_create(4, 4, 1).buffer;
        let second = buffer_create(4, 4, 1).buffer;
        let first_pointer = first.pointer;
        let second_pointer = second.pointer;
        let outer_result =
            buffer_create(size_of::<Buffer>() as u64, align_of::<Buffer>() as u64, 2);
        assert_eq!(outer_result.status, BufferStatus::Ok.code());
        let mut outer = outer_result.buffer;
        // SAFETY: the outer allocation has two initialized Buffer slots.
        unsafe {
            let slots = outer.pointer.cast::<Buffer>();
            slots.write(first);
            slots.add(1).write(second);
        }
        outer.length = 2;

        let mut output_slot = std::mem::MaybeUninit::<Buffer>::uninit();
        let status = unsafe {
            buffer_pop_move_into(
                &mut outer,
                output_slot.as_mut_ptr().cast::<u8>(),
                size_of::<Buffer>() as u64,
                align_of::<Buffer>() as u64,
            )
        };
        assert_eq!(status, BufferStatus::Ok);
        assert_eq!(outer.length, 1);
        // SAFETY: a successful move initialized the output descriptor.
        let mut output = unsafe { output_slot.assume_init() };
        assert_eq!(output.pointer, second_pointer);
        // SAFETY: the remaining slot and output are the sole owners.
        unsafe {
            let slots = outer.pointer.cast::<Buffer>();
            assert_eq!((*slots).pointer, first_pointer);
            assert_eq!(buffer_destroy(&mut *slots, 4, 4), BufferStatus::Ok);
        }
        assert_eq!(
            unsafe { buffer_destroy(&mut output, 4, 4) },
            BufferStatus::Ok
        );
        assert_eq!(
            unsafe {
                buffer_destroy(
                    &mut outer,
                    size_of::<Buffer>() as u64,
                    align_of::<Buffer>() as u64,
                )
            },
            BufferStatus::Ok
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn owning_record_remove_move_into_transfers_record_and_closes_gap() {
        #[repr(C)]
        struct OwningEntry {
            values: Buffer,
            id: u32,
            _padding: u32,
        }

        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let first = buffer_create(4, 4, 1).buffer;
        let second = buffer_create(4, 4, 1).buffer;
        let third = buffer_create(4, 4, 1).buffer;
        let first_pointer = first.pointer;
        let second_pointer = second.pointer;
        let third_pointer = third.pointer;
        let outer_result = buffer_create(
            size_of::<OwningEntry>() as u64,
            align_of::<OwningEntry>() as u64,
            3,
        );
        assert_eq!(outer_result.status, BufferStatus::Ok.code());
        let mut outer = outer_result.buffer;
        // SAFETY: the outer allocation has three aligned OwningEntry slots.
        unsafe {
            let slots = outer.pointer.cast::<OwningEntry>();
            slots.write(OwningEntry {
                values: first,
                id: 1,
                _padding: 0,
            });
            slots.add(1).write(OwningEntry {
                values: second,
                id: 2,
                _padding: 0,
            });
            slots.add(2).write(OwningEntry {
                values: third,
                id: 3,
                _padding: 0,
            });
        }
        outer.length = 3;

        let mut output_slot = std::mem::MaybeUninit::<OwningEntry>::uninit();
        let status = unsafe {
            buffer_remove_move_into(
                &mut outer,
                1,
                output_slot.as_mut_ptr().cast::<u8>(),
                size_of::<OwningEntry>() as u64,
                align_of::<OwningEntry>() as u64,
            )
        };
        assert_eq!(status, BufferStatus::Ok);
        assert_eq!(outer.length, 2);
        // SAFETY: a successful move initialized the caller-owned output slot.
        let mut output = unsafe { output_slot.assume_init() };
        // SAFETY: the two remaining initialized slots retain their descriptors.
        unsafe {
            let slots = outer.pointer.cast::<OwningEntry>();
            assert_eq!((*slots).id, 1);
            assert_eq!((*slots).values.pointer, first_pointer);
            assert_eq!((*slots.add(1)).id, 3);
            assert_eq!((*slots.add(1)).values.pointer, third_pointer);
        }
        assert_eq!(output.id, 2);
        assert_eq!(output.values.pointer, second_pointer);

        // SAFETY: each descriptor is still the sole owner of its allocation.
        assert_eq!(
            unsafe { buffer_destroy(&mut output.values, 4, 4) },
            BufferStatus::Ok
        );
        unsafe {
            let slots = outer.pointer.cast::<OwningEntry>();
            assert_eq!(buffer_destroy(&mut (*slots).values, 4, 4), BufferStatus::Ok);
            assert_eq!(
                buffer_destroy(&mut (*slots.add(1)).values, 4, 4),
                BufferStatus::Ok
            );
        }
        assert_eq!(
            unsafe {
                buffer_destroy(
                    &mut outer,
                    size_of::<OwningEntry>() as u64,
                    align_of::<OwningEntry>() as u64,
                )
            },
            BufferStatus::Ok
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn owning_record_insert_move_from_transfers_source_and_shifts_records() {
        #[repr(C)]
        struct OwningEntry {
            values: Buffer,
            id: u32,
            _padding: u32,
        }

        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let first = buffer_create(4, 4, 1).buffer;
        let source_values = buffer_create(4, 4, 1).buffer;
        let first_pointer = first.pointer;
        let source_pointer = source_values.pointer;
        let outer_result = buffer_create(
            size_of::<OwningEntry>() as u64,
            align_of::<OwningEntry>() as u64,
            1,
        );
        assert_eq!(outer_result.status, BufferStatus::Ok.code());
        let mut outer = outer_result.buffer;
        // SAFETY: the allocation contains one aligned initialized record slot.
        unsafe {
            outer.pointer.cast::<OwningEntry>().write(OwningEntry {
                values: first,
                id: 1,
                _padding: 0,
            });
        }
        outer.length = 1;
        let mut source = OwningEntry {
            values: source_values,
            id: 99,
            _padding: 0,
        };
        let status = unsafe {
            buffer_insert_move_from(
                &mut outer,
                0,
                (&mut source as *mut OwningEntry).cast::<u8>(),
                size_of::<OwningEntry>() as u64,
                align_of::<OwningEntry>() as u64,
            )
        };
        assert_eq!(status, BufferStatus::Ok);
        assert_eq!(outer.length, 2);
        assert!(source.values.pointer.is_null());
        assert_eq!(source.values.length, 0);
        assert_eq!(source.values.capacity, 0);
        assert_eq!(source.id, 0);
        // SAFETY: successful insertion initialized both record slots.
        unsafe {
            let slots = outer.pointer.cast::<OwningEntry>();
            assert_eq!((*slots).id, 99);
            assert_eq!((*slots).values.pointer, source_pointer);
            assert_eq!((*slots.add(1)).id, 1);
            assert_eq!((*slots.add(1)).values.pointer, first_pointer);
            assert_eq!(buffer_destroy(&mut (*slots).values, 4, 4), BufferStatus::Ok);
            assert_eq!(
                buffer_destroy(&mut (*slots.add(1)).values, 4, 4),
                BufferStatus::Ok
            );
        }
        assert_eq!(
            unsafe {
                buffer_destroy(
                    &mut outer,
                    size_of::<OwningEntry>() as u64,
                    align_of::<OwningEntry>() as u64,
                )
            },
            BufferStatus::Ok
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn utf8_strings_validate_append_clear_and_destroy_without_copying_headers() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let empty = string_create(0);
        assert_eq!(empty.status, StringStatus::Ok.code());
        assert!(empty.string.pointer.is_null());
        assert_eq!(empty.string.length, 0);
        assert_eq!(empty.string.capacity, 0);
        assert_eq!(
            unsafe { string_from_utf8(ptr::null(), 0) }.status,
            StringStatus::Ok.code()
        );
        assert_eq!(
            unsafe { string_from_utf8([0xff_u8].as_ptr(), 1) }.status,
            StringStatus::InvalidUtf8.code()
        );

        let source = "Aho, 世界".as_bytes();
        let created = unsafe { string_from_utf8(source.as_ptr(), source.len() as u64) };
        assert_eq!(created.status, StringStatus::Ok.code());
        let mut string = created.string;
        assert_eq!(string.length, source.len() as u64);
        assert_eq!(string.capacity, source.len() as u64);
        let split_code_point = [0xe4_u8, 0xb8_u8];
        assert_eq!(
            unsafe { string_from_utf8(split_code_point.as_ptr(), split_code_point.len() as u64,) }
                .status,
            StringStatus::InvalidUtf8.code()
        );
        // SAFETY: `string` owns exactly `length` initialized UTF-8 bytes.
        assert_eq!(
            unsafe { std::slice::from_raw_parts(string.pointer, string.length as usize) },
            source
        );

        let suffix = "!".as_bytes();
        assert_eq!(
            unsafe { string_append_utf8(&mut string, suffix.as_ptr(), suffix.len() as u64) },
            StringStatus::Ok
        );
        assert_eq!(string.length, (source.len() + suffix.len()) as u64);
        let old_length = string.length;
        let invalid = [0xc3_u8, 0x28_u8];
        assert_eq!(
            unsafe { string_append_utf8(&mut string, invalid.as_ptr(), invalid.len() as u64) },
            StringStatus::InvalidUtf8
        );
        assert_eq!(string.length, old_length);
        assert_eq!(unsafe { string_reserve(&mut string, 64) }, StringStatus::Ok);
        assert!(string.capacity >= 64);
        assert_eq!(unsafe { string_clear(&mut string) }, StringStatus::Ok);
        assert_eq!(string.length, 0);
        assert_eq!(unsafe { string_destroy(&mut string) }, StringStatus::Ok);
        assert_eq!(string, Utf8String::EMPTY);
        assert_eq!(unsafe { string_destroy(&mut string) }, StringStatus::Ok);
    }

    #[test]
    #[allow(unsafe_code)]
    fn utf8_string_append_partition_property_holds_for_deterministic_chunks() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let chunks: &[&[u8]] = &[
            b"Jadren ",
            "bezpečný ".as_bytes(),
            "runtime ".as_bytes(),
            "🚀".as_bytes(),
            b"\0 tail",
        ];
        let expected: Vec<u8> = chunks
            .iter()
            .flat_map(|chunk| chunk.iter().copied())
            .collect();
        let mut string = string_create(0).string;
        let mut prefix = Vec::new();

        for chunk in chunks {
            assert_eq!(
                unsafe { string_append_utf8(&mut string, chunk.as_ptr(), chunk.len() as u64) },
                StringStatus::Ok
            );
            prefix.extend_from_slice(chunk);
            assert_eq!(string.length, prefix.len() as u64);
            // SAFETY: the runtime owns exactly `string.length` initialized UTF-8 bytes.
            assert_eq!(
                unsafe { std::slice::from_raw_parts(string.pointer, string.length as usize) },
                prefix.as_slice()
            );
            assert_eq!(
                unsafe { string_reserve(&mut string, string.length + 5) },
                StringStatus::Ok
            );
        }
        assert_eq!(prefix, expected);

        for invalid in [
            &[0xe2_u8, 0x28, 0xa1][..],
            &[0xc0_u8, 0xaf][..],
            &[0xf0_u8, 0x28, 0x8c, 0xbc][..],
        ] {
            let old_length = string.length;
            assert_eq!(
                unsafe { string_append_utf8(&mut string, invalid.as_ptr(), invalid.len() as u64) },
                StringStatus::InvalidUtf8
            );
            assert_eq!(string.length, old_length);
            // SAFETY: rejected input must not change the initialized payload.
            assert_eq!(
                unsafe { std::slice::from_raw_parts(string.pointer, string.length as usize) },
                expected.as_slice()
            );
        }

        assert_eq!(unsafe { string_clear(&mut string) }, StringStatus::Ok);
        assert_eq!(string.length, 0);
        assert_eq!(unsafe { string_destroy(&mut string) }, StringStatus::Ok);
        assert_eq!(string, Utf8String::EMPTY);
    }

    #[test]
    fn math_scalar_helpers_are_allocation_free_and_follow_ieee_edges() {
        assert_eq!(math_abs_f32(-3.5), 3.5);
        assert_eq!(math_abs_f64(-3.5), 3.5);
        assert_eq!(math_abs_f32(-0.0).to_bits(), 0.0_f32.to_bits());
        assert_eq!(math_sqrt_f32(9.0), 3.0);
        assert_eq!(math_sqrt_f64(9.0), 3.0);
        assert!(math_sqrt_f32(-1.0).is_nan());
        assert!(math_sqrt_f64(-1.0).is_nan());
        assert!((math_acos_f32(0.5) - std::f32::consts::FRAC_PI_3).abs() < 0.000001);
        assert!((math_sin_f32(0.5) - 0.47942555).abs() < 0.000001);
        assert!((math_cos_f32(0.5) - 0.87758255).abs() < 0.000001);
        assert_eq!(math_floor_f32(3.75), 3.0);
        assert_eq!(math_floor_f64(-3.25), -4.0);
        assert_eq!(math_ceil_f32(3.25), 4.0);
        assert_eq!(math_ceil_f64(-3.75), -3.0);
        assert!(math_floor_f32(f32::NAN).is_nan());
    }

    #[test]
    fn vector_quaternion_and_matrix_value_layouts_are_stable() {
        assert_eq!(size_of::<Float2>(), 8);
        assert_eq!(size_of::<Float3>(), 12);
        assert_eq!(size_of::<Float4>(), 16);
        assert_eq!(size_of::<Float8>(), 32);
        assert_eq!(align_of::<Float8>(), 4);
        assert_eq!(size_of::<Quaternion>(), 16);
        assert_eq!(size_of::<Matrix4>(), 64);
        assert_eq!(
            float2_add(Float2 { x: 1.0, y: 2.0 }, Float2 { x: 3.0, y: 4.0 }),
            Float2 { x: 4.0, y: 6.0 }
        );
        assert_eq!(
            float3_add(
                Float3 {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                },
                Float3 {
                    x: 4.0,
                    y: 5.0,
                    z: 6.0,
                }
            ),
            Float3 {
                x: 5.0,
                y: 7.0,
                z: 9.0,
            }
        );
        assert_eq!(
            float4_add(
                Float4 {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                    w: 4.0,
                },
                Float4 {
                    x: 4.0,
                    y: 3.0,
                    z: 2.0,
                    w: 1.0,
                }
            ),
            Float4 {
                x: 5.0,
                y: 5.0,
                z: 5.0,
                w: 5.0,
            }
        );
        assert_eq!(
            float3_dot(
                Float3 {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                },
                Float3 {
                    x: 4.0,
                    y: 5.0,
                    z: 6.0,
                }
            ),
            32.0
        );
        assert_eq!(quaternion_identity().w, 1.0);
        assert_eq!(matrix4_identity().values[0], 1.0);
        assert_eq!(matrix4_identity().values[15], 1.0);
    }

    #[test]
    fn quaternion_slerp_unclamped_matches_shortest_arc_contract() {
        let identity = quaternion_identity();
        let quarter_turn = Quaternion {
            x: 0.0,
            y: 0.0,
            z: 0.70710677,
            w: 0.70710677,
        };
        let half = quaternion_slerp_unclamped(identity, quarter_turn, 0.5);
        assert!((half.z - 0.38268343).abs() < 0.00001);
        assert!((half.w - 0.9238795).abs() < 0.00001);

        let opposite_sign = Quaternion {
            x: -quarter_turn.x,
            y: -quarter_turn.y,
            z: -quarter_turn.z,
            w: -quarter_turn.w,
        };
        let shortest = quaternion_slerp_unclamped(quarter_turn, opposite_sign, 0.5);
        assert!((shortest.x - quarter_turn.x).abs() < 0.00001);
        assert!((shortest.z - quarter_turn.z).abs() < 0.00001);
        assert!((shortest.w - quarter_turn.w).abs() < 0.00001);

        let extrapolated = quaternion_slerp_unclamped(identity, quarter_turn, -0.5);
        let magnitude = math_sqrt_f32(
            extrapolated.x * extrapolated.x
                + extrapolated.y * extrapolated.y
                + extrapolated.z * extrapolated.z
                + extrapolated.w * extrapolated.w,
        );
        assert!((magnitude - 1.0).abs() < 0.00001);
        assert!(extrapolated.z < 0.0);

        let threshold_dot = 0.9995_f32;
        let threshold = Quaternion {
            x: 0.0,
            y: math_sqrt_f32(1.0 - threshold_dot * threshold_dot),
            z: 0.0,
            w: threshold_dot,
        };
        let threshold_actual = quaternion_slerp_unclamped(identity, threshold, 0.5);
        let theta_zero = math_acos_f32(threshold_dot);
        let sin_theta_zero = math_sin_f32(theta_zero);
        let theta = theta_zero * 0.5;
        let sin_theta = math_sin_f32(theta);
        let expected_s0 = math_cos_f32(theta) - threshold_dot * sin_theta / sin_theta_zero;
        let expected_s1 = sin_theta / sin_theta_zero;
        assert!((threshold_actual.y - threshold.y * expected_s1).abs() < 0.00001);
        assert!((threshold_actual.w - (expected_s0 + threshold_dot * expected_s1)).abs() < 0.00001);
    }

    #[test]
    fn float8_add_is_lane_stable_and_target_neutral() {
        let left = Float8 {
            lanes: [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0],
        };
        let right = Float8 {
            lanes: [8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0],
        };
        assert_eq!(float8_add(left, right), Float8 { lanes: [8.0; 8] });
        assert_eq!(float8_add(left, Float8 { lanes: [0.0; 8] }), left);
        assert_eq!(float8_add(left, right), float8_add(right, left));
    }

    #[test]
    fn vector_math_properties_hold_for_deterministic_finite_domain() {
        fn next_value(state: &mut u32) -> f32 {
            *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((*state % 20_001) as f32 - 10_000.0) / 100.0
        }

        let zero2 = Float2 { x: 0.0, y: 0.0 };
        let zero3 = Float3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        let zero4 = Float4 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 0.0,
        };
        let mut state = 0x6d2b_79f5;
        for _ in 0..257 {
            let left2 = Float2 {
                x: next_value(&mut state),
                y: next_value(&mut state),
            };
            let right2 = Float2 {
                x: next_value(&mut state),
                y: next_value(&mut state),
            };
            assert_eq!(float2_add(left2, zero2), left2);
            assert_eq!(float2_add(left2, right2), float2_add(right2, left2));

            let left3 = Float3 {
                x: next_value(&mut state),
                y: next_value(&mut state),
                z: next_value(&mut state),
            };
            let right3 = Float3 {
                x: next_value(&mut state),
                y: next_value(&mut state),
                z: next_value(&mut state),
            };
            assert_eq!(float3_add(left3, zero3), left3);
            assert_eq!(float3_add(left3, right3), float3_add(right3, left3));
            assert_eq!(float3_dot(left3, right3), float3_dot(right3, left3));
            assert!(float3_dot(left3, left3) >= 0.0);

            let left4 = Float4 {
                x: next_value(&mut state),
                y: next_value(&mut state),
                z: next_value(&mut state),
                w: next_value(&mut state),
            };
            let right4 = Float4 {
                x: next_value(&mut state),
                y: next_value(&mut state),
                z: next_value(&mut state),
                w: next_value(&mut state),
            };
            assert_eq!(float4_add(left4, zero4), left4);
            assert_eq!(float4_add(left4, right4), float4_add(right4, left4));
        }
    }

    #[test]
    fn float8_math_properties_hold_for_deterministic_finite_domain() {
        fn next_value(state: &mut u32) -> f32 {
            *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((*state % 20_001) as f32 - 10_000.0) / 100.0
        }

        let zero = Float8 { lanes: [0.0; 8] };
        let mut state = 0x4f1b_2c73;
        for _ in 0..257 {
            let mut left = [0.0; 8];
            let mut right = [0.0; 8];
            let mut lane = 0;
            while lane < 8 {
                left[lane] = next_value(&mut state);
                right[lane] = next_value(&mut state);
                lane += 1;
            }

            let left = Float8 { lanes: left };
            let right = Float8 { lanes: right };
            let sum = float8_add(left, right);
            assert_eq!(float8_add(left, zero), left);
            assert_eq!(sum, float8_add(right, left));
            let mut lane = 0;
            while lane < 8 {
                assert_eq!(sum.lanes[lane], left.lanes[lane] + right.lanes[lane]);
                lane += 1;
            }
        }
    }

    #[test]
    #[allow(unsafe_code)]
    fn buffer_slice_partition_property_holds_for_deterministic_ranges() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);
        let result = buffer_create(1, 1, 257);
        assert_eq!(result.status, BufferStatus::Ok.code());
        let mut buffer = result.buffer;
        let bytes = buffer.pointer.cast::<u8>();
        for index in 0..257_u64 {
            // SAFETY: the allocation owns 257 writable bytes.
            unsafe { bytes.add(index as usize).write((index & 0xff) as u8) };
        }
        assert_eq!(unsafe { buffer_resize(&mut buffer, 257) }, BufferStatus::Ok);
        for start in (0..=257_u64).step_by(17) {
            let max_count = 257 - start;
            for count in [0, 1, max_count / 2, max_count] {
                let view = unsafe { buffer_slice(&buffer, 1, 1, start, count) };
                assert_eq!(view.status, BufferStatus::Ok.code());
                assert_eq!(view.slice.length, count);
                if count != 0 {
                    assert_eq!(
                        view.slice.pointer.addr(),
                        buffer.pointer.addr() + start as usize
                    );
                    // SAFETY: the slice range is proven within the initialized buffer.
                    assert_eq!(
                        unsafe { view.slice.pointer.cast::<u8>().read() },
                        (start & 0xff) as u8
                    );
                }
            }
            assert_eq!(
                unsafe { buffer_slice(&buffer, 1, 1, start, max_count + 1) }.status,
                BufferStatus::OutOfBounds.code()
            );
        }
        assert_eq!(
            unsafe { buffer_destroy(&mut buffer, 1, 1) },
            BufferStatus::Ok
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn nested_slice_partition_property_holds_for_deterministic_ranges() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let result = buffer_create(1, 1, 193);
        assert_eq!(result.status, BufferStatus::Ok.code());
        let mut buffer = result.buffer;
        assert_eq!(unsafe { buffer_resize(&mut buffer, 193) }, BufferStatus::Ok);
        let bytes = buffer.pointer.cast::<u8>();
        for index in 0..193_u64 {
            // SAFETY: the allocation owns 193 writable bytes.
            unsafe { bytes.add(index as usize).write((index % 251) as u8) };
        }

        for parent_start in (0..=193_u64).step_by(19) {
            let parent_length = 193 - parent_start;
            let parent = unsafe { buffer_slice(&buffer, 1, 1, parent_start, parent_length) };
            assert_eq!(parent.status, BufferStatus::Ok.code());
            for nested_start in (0..=parent_length).step_by(11) {
                let remaining = parent_length - nested_start;
                let one = if remaining == 0 { 0 } else { 1 };
                for nested_length in [0, one, remaining / 2, remaining] {
                    let nested =
                        unsafe { slice_subslice(&parent.slice, 1, 1, nested_start, nested_length) };
                    assert_eq!(nested.status, BufferStatus::Ok.code());
                    assert_eq!(nested.slice.length, nested_length);
                    if nested_length != 0 {
                        assert_eq!(
                            nested.slice.pointer.addr(),
                            parent.slice.pointer.addr() + nested_start as usize
                        );
                        // SAFETY: the nested range is within the parent slice.
                        assert_eq!(
                            unsafe { nested.slice.pointer.cast::<u8>().read() },
                            ((parent_start + nested_start) % 251) as u8
                        );
                    } else {
                        assert!(nested.slice.pointer.is_null());
                    }
                }
                assert_eq!(
                    unsafe { slice_subslice(&parent.slice, 1, 1, nested_start, remaining + 1) }
                        .status,
                    BufferStatus::OutOfBounds.code()
                );
            }
        }

        assert_eq!(
            unsafe { buffer_destroy(&mut buffer, 1, 1) },
            BufferStatus::Ok
        );
    }

    #[test]
    fn initialization_is_compatible_idempotent_and_thread_safe() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(runtime_state(), RuntimeState::Uninitialized);
        assert_eq!(jadren_rt_is_initialized(), 0);
        assert_eq!(
            initialize(AbiVersion {
                major: RUNTIME_ABI_MAJOR + 1,
                minor: 0,
            }),
            RuntimeStatus::IncompatibleMajor
        );
        assert_eq!(
            jadren_rt_initialize(RUNTIME_ABI_MAJOR, RUNTIME_ABI_MINOR + 1),
            RuntimeStatus::IncompatibleMinor.code()
        );
        assert_eq!(runtime_state(), RuntimeState::Uninitialized);

        let thread_count = 8;
        let barrier = Arc::new(Barrier::new(thread_count));
        let mut threads = Vec::new();
        for _ in 0..thread_count {
            let barrier = Arc::clone(&barrier);
            threads.push(thread::spawn(move || {
                barrier.wait();
                initialize(AbiVersion::CURRENT)
            }));
        }
        let statuses: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().expect("runtime initialization thread"))
            .collect();
        assert_eq!(
            statuses
                .iter()
                .filter(|status| **status == RuntimeStatus::Initialized)
                .count(),
            1
        );
        assert_eq!(
            statuses
                .iter()
                .filter(|status| **status == RuntimeStatus::AlreadyInitialized)
                .count(),
            thread_count - 1
        );
        assert_eq!(runtime_state(), RuntimeState::Initialized);
        assert_eq!(jadren_rt_is_initialized(), 1);
        assert_eq!(
            jadren_rt_initialize(RUNTIME_ABI_MAJOR, 0),
            RuntimeStatus::AlreadyInitialized.code()
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn allocator_validates_layout_preserves_bytes_and_handles_concurrency() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(
            system_allocate(64, 64).status,
            AllocatorStatus::RuntimeNotInitialized.code()
        );
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);
        assert_eq!(
            system_allocate(0, 8).status,
            AllocatorStatus::InvalidSize.code()
        );
        assert_eq!(
            system_allocate(8, 3).status,
            AllocatorStatus::InvalidAlignment.code()
        );
        assert_eq!(
            system_allocate(u64::MAX, 8).status,
            AllocatorStatus::SizeOverflow.code()
        );

        let allocation = system_allocate(64, 64);
        assert_eq!(allocation.status, AllocatorStatus::Ok.code());
        assert!(!allocation.pointer.is_null());
        assert_eq!(allocation.pointer.addr() % 64, 0);
        let bytes = allocation.pointer.cast::<u8>();
        for index in 0..64 {
            // SAFETY: the allocation owns 64 writable bytes and index is in range.
            unsafe { bytes.add(index).write(index as u8) };
        }
        // SAFETY: pointer is live and its current layout is exactly 64/64.
        let resized = unsafe { system_reallocate(allocation.pointer, 64, 128, 64) };
        assert_eq!(resized.status, AllocatorStatus::Ok.code());
        assert!(!resized.pointer.is_null());
        assert_eq!(resized.pointer.addr() % 64, 0);
        let resized_bytes = resized.pointer.cast::<u8>();
        for index in 0..64 {
            // SAFETY: realloc preserves the first 64 bytes of the live 128-byte allocation.
            assert_eq!(unsafe { resized_bytes.add(index).read() }, index as u8);
        }
        // SAFETY: pointer is live and its current layout is exactly 128/64.
        assert_eq!(
            unsafe { system_deallocate(resized.pointer, 128, 64) },
            AllocatorStatus::Ok
        );
        // SAFETY: null is rejected before any allocator operation.
        assert_eq!(
            unsafe { system_deallocate(ptr::null_mut(), 8, 8) },
            AllocatorStatus::NullPointer
        );

        let thread_count = 8;
        let iterations = 128;
        let barrier = Arc::new(Barrier::new(thread_count));
        let threads: Vec<_> = (0..thread_count)
            .map(|thread_index| {
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    for iteration in 0..iterations {
                        let alignment = 1_u64 << ((thread_index + iteration) % 7);
                        let size = 1 + ((thread_index * 31 + iteration * 17) % 257) as u64;
                        let allocation = system_allocate(size, alignment);
                        assert_eq!(allocation.status, AllocatorStatus::Ok.code());
                        assert_eq!(allocation.pointer.addr() % alignment as usize, 0);
                        // SAFETY: pointer is live and size/alignment are its exact layout.
                        assert_eq!(
                            unsafe { system_deallocate(allocation.pointer, size, alignment) },
                            AllocatorStatus::Ok
                        );
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().expect("allocator stress thread");
        }
    }

    #[test]
    #[allow(unsafe_code)]
    fn region_allocator_batches_aligned_blocks_until_destroy() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(
            region_create().status,
            AllocatorStatus::RuntimeNotInitialized.code()
        );
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);

        let created = region_create();
        assert_eq!(created.status, AllocatorStatus::Ok.code());
        assert!(!created.pointer.is_null());

        // The C ABI and Rust API share the same validation/status contract.
        assert_eq!(
            unsafe { region_allocate(created.pointer, 0, 8) }.status,
            AllocatorStatus::InvalidSize.code()
        );
        assert_eq!(
            unsafe { jadren_rt_region_allocate(created.pointer, 8, 3) }.status,
            AllocatorStatus::InvalidAlignment.code()
        );
        assert_eq!(
            unsafe { region_allocate(created.pointer, u64::MAX, 8) }.status,
            AllocatorStatus::SizeOverflow.code()
        );

        let first = unsafe { region_allocate(created.pointer, 64, 64) };
        assert_eq!(first.status, AllocatorStatus::Ok.code());
        assert!(!first.pointer.is_null());
        assert_eq!(first.pointer.addr() % 64, 0);
        let bytes = first.pointer.cast::<u8>();
        for index in 0..64 {
            // Region-backed Buffer storage is deterministic before the first
            // Jadren assignment.
            assert_eq!(unsafe { bytes.add(index).read() }, 0);
        }
        for index in 0..64 {
            // SAFETY: this block owns 64 writable bytes until region destroy.
            unsafe { bytes.add(index).write((index ^ 0x5a) as u8) };
        }

        let second = unsafe { jadren_rt_region_allocate(created.pointer, 128, 32) };
        assert_eq!(second.status, AllocatorStatus::Ok.code());
        assert!(!second.pointer.is_null());
        assert_eq!(second.pointer.addr() % 32, 0);
        for index in 0..64 {
            // SAFETY: the first allocation remains live until region destroy.
            assert_eq!(unsafe { bytes.add(index).read() }, (index ^ 0x5a) as u8);
        }

        assert_eq!(
            unsafe { jadren_rt_region_destroy(created.pointer) },
            AllocatorStatus::Ok.code()
        );
        assert_eq!(
            unsafe { region_destroy(ptr::null_mut()) },
            AllocatorStatus::NullPointer
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn region_abi_create_and_destroy_validate_lifecycle() {
        let _guard = TEST_RUNTIME_LOCK.lock().expect("runtime test lock");
        RUNTIME_STATE.store(STATE_UNINITIALIZED, Ordering::Release);
        assert_eq!(jadren_rt_region_create().status, -10);
        assert_eq!(initialize(AbiVersion::CURRENT), RuntimeStatus::Initialized);
        let region = jadren_rt_region_create();
        assert_eq!(region.status, 0);
        assert_eq!(
            unsafe { jadren_rt_region_destroy(region.pointer) },
            AllocatorStatus::Ok.code()
        );
    }
}
