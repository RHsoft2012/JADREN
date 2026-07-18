//! Versioned initialization boundary for the Jadren native runtime.
//!
//! JADREN-UNSAFE-AUDIT: raw allocation, callback and C-ABI pointer code is
//! isolated in this module. Public pointer entry points document their caller
//! invariants in `# Safety` sections; safe Rust owns validation and status
//! conversion before any dereference or deallocation.

use std::alloc::{Layout, alloc, dealloc, realloc};
use std::ffi::c_void;
use std::mem::transmute;
use std::process;
use std::ptr;
use std::slice;
use std::str;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

/// First incompatible-change generation of the native runtime ABI.
pub const RUNTIME_ABI_MAJOR: u32 = 0;
/// Backward-compatible feature generation of the native runtime ABI.
pub const RUNTIME_ABI_MINOR: u32 = 10;
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
        // storage is raw; typed initialization remains a compiler/core
        // responsibility before a value is observed.
        let pointer = unsafe { alloc(layout) };
        if pointer.is_null() {
            return AllocationResult::failure(AllocatorStatus::OutOfMemory);
        }
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
        AbiVersion, AllocatorStatus, Buffer, BufferStatus, CallbackStatus, Float2, Float3, Float4,
        Float8, LogLevel, Matrix4, PanicCode, Quaternion, RUNTIME_ABI_MAJOR, RUNTIME_ABI_MINOR,
        RUNTIME_BUILD_ID, RUNTIME_STATE, RuntimeState, RuntimeStatus, STATE_UNINITIALIZED,
        StringStatus, Utf8String, bounds_panic_info, buffer_create, buffer_destroy, buffer_reserve,
        buffer_resize, buffer_slice, float2_add, float3_add, float3_dot, float4_add, float8_add,
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
