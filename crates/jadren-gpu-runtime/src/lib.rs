//! Host-side residency and synchronization model consumed by the Vulkan runtime.
//!
//! This crate deliberately owns no Vulkan handles yet. It proves the state
//! transitions and access-token invariants that a native backend must preserve.

mod supported_subset;

pub use supported_subset::{
    JADREN_GPU_SUPPORTED_SUBSET_EXPANSION_CASE_IDS, emit_gpu_supported_subset_case_words,
    gpu_supported_subset_case_entry_name,
};

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use jadren_codegen_spirv::{
    ResourceAccess, ResourceElementType, SpirvArtifact, SpirvValidationError,
    annotate_spirv_resource_names,
};
use jadren_jir::AddressSpace;

/// Stable host-side identity for one GPU buffer allocation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BufferId(u32);

impl BufferId {
    /// Returns the stable numeric identity.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

/// Current ownership/residency state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Residency {
    /// Host-owned and not usable by a GPU queue.
    Host,
    /// Device-local or device-visible and usable by a GPU queue.
    Device,
    /// Explicitly shared host/device allocation.
    Shared,
}

/// One command scope's declared buffer access.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessKind {
    Read,
    Write,
    ReadWrite,
}

impl AccessKind {
    /// Returns whether two live accesses conflict.
    #[must_use]
    pub const fn conflicts(self, other: Self) -> bool {
        !matches!((self, other), (Self::Read, Self::Read))
    }
}

/// Linear-ish completion token for one live buffer access.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessToken {
    buffer: BufferId,
    generation: u64,
    kind: AccessKind,
}

impl AccessToken {
    /// Returns the buffer covered by this token.
    #[must_use]
    pub const fn buffer(self) -> BufferId {
        self.buffer
    }

    /// Returns the declared access mode.
    #[must_use]
    pub const fn kind(self) -> AccessKind {
        self.kind
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ActiveAccess {
    generation: u64,
    kind: AccessKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BufferState {
    size: u64,
    residency: Residency,
    next_generation: u64,
    active: Vec<ActiveAccess>,
}

/// Errors raised by residency or synchronization transitions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceError {
    InvalidSize,
    UnknownBuffer(BufferId),
    AlreadyResident(BufferId),
    NotResident(BufferId),
    Busy(BufferId),
    AccessConflict(BufferId),
    StaleToken(BufferId),
    IdExhausted,
}

impl fmt::Display for ResourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSize => formatter.write_str("GPU buffer size must be positive"),
            Self::UnknownBuffer(id) => write!(formatter, "unknown GPU buffer {}", id.value()),
            Self::AlreadyResident(id) => {
                write!(formatter, "GPU buffer {} is already resident", id.value())
            }
            Self::NotResident(id) => write!(
                formatter,
                "GPU buffer {} is not device-resident",
                id.value()
            ),
            Self::Busy(id) => write!(
                formatter,
                "GPU buffer {} has live access tokens",
                id.value()
            ),
            Self::AccessConflict(id) => write!(
                formatter,
                "GPU buffer {} has a conflicting live access",
                id.value()
            ),
            Self::StaleToken(id) => {
                write!(formatter, "GPU buffer {} access token is stale", id.value())
            }
            Self::IdExhausted => formatter.write_str("GPU buffer ID space exhausted"),
        }
    }
}

impl Error for ResourceError {}

/// Stable CPU symbol used when a GPU path is unavailable or rejected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CpuFallbackLink {
    symbol: String,
    abi_minor: u16,
}

impl CpuFallbackLink {
    /// Creates a validated C-compatible fallback linkage record.
    pub fn new(symbol: impl Into<String>, abi_minor: u16) -> Result<Self, LinkageError> {
        let symbol = symbol.into();
        if !valid_symbol(&symbol) {
            return Err(LinkageError::InvalidSymbol);
        }
        if abi_minor == 0 {
            return Err(LinkageError::InvalidAbi);
        }
        Ok(Self { symbol, abi_minor })
    }

    /// Returns the exported CPU fallback symbol.
    #[must_use]
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    /// Returns the required runtime ABI minor version.
    #[must_use]
    pub const fn abi_minor(&self) -> u16 {
        self.abi_minor
    }
}

/// CPU fallback linkage and optional GPU entrypoint for one logical kernel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelLinkage {
    gpu_entry: Option<String>,
    cpu_fallback: CpuFallbackLink,
}

impl KernelLinkage {
    /// Creates explicit linkage metadata; no dispatch or library loading occurs.
    pub fn new(
        gpu_entry: Option<impl Into<String>>,
        cpu_fallback: CpuFallbackLink,
    ) -> Result<Self, LinkageError> {
        let gpu_entry = gpu_entry.map(Into::into);
        if let Some(entry) = &gpu_entry
            && !valid_symbol(entry)
        {
            return Err(LinkageError::InvalidGpuEntry);
        }
        Ok(Self {
            gpu_entry,
            cpu_fallback,
        })
    }

    /// Returns the optional GPU entrypoint symbol.
    #[must_use]
    pub fn gpu_entry(&self) -> Option<&str> {
        self.gpu_entry.as_deref()
    }

    /// Returns the mandatory CPU fallback link.
    #[must_use]
    pub const fn cpu_fallback(&self) -> &CpuFallbackLink {
        &self.cpu_fallback
    }
}

/// Linkage metadata validation errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkageError {
    InvalidSymbol,
    InvalidAbi,
    InvalidGpuEntry,
}

impl fmt::Display for LinkageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSymbol => {
                formatter.write_str("fallback symbol must be non-empty ASCII [A-Za-z0-9_]")
            }
            Self::InvalidAbi => formatter.write_str("fallback ABI minor must be positive"),
            Self::InvalidGpuEntry => formatter.write_str("GPU entry symbol is invalid"),
        }
    }
}

impl Error for LinkageError {}

fn valid_symbol(symbol: &str) -> bool {
    !symbol.is_empty()
        && symbol
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// Errors raised while constructing a bounded tensor layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TensorLayoutError {
    /// An extent, stride or capacity was zero.
    ZeroComponent,
    /// The maximum affine index cannot be represented by `usize`.
    ArithmeticOverflow,
}

impl fmt::Display for TensorLayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroComponent => {
                formatter.write_str("tensor extents, strides and capacity must be positive")
            }
            Self::ArithmeticOverflow => formatter.write_str("tensor affine index overflows usize"),
        }
    }
}

impl Error for TensorLayoutError {}

/// Overflow-safe 2D affine tensor layout used by host differential oracles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TensorLayout2D {
    width: usize,
    height: usize,
    stride_x: usize,
    stride_y: usize,
    capacity: usize,
}

impl TensorLayout2D {
    /// Creates a positive 2D affine layout and checks its maximum index.
    pub fn new(
        width: usize,
        height: usize,
        stride_x: usize,
        stride_y: usize,
        capacity: usize,
    ) -> Result<Self, TensorLayoutError> {
        if width == 0 || height == 0 || stride_x == 0 || stride_y == 0 || capacity == 0 {
            return Err(TensorLayoutError::ZeroComponent);
        }
        (width - 1)
            .checked_mul(stride_x)
            .and_then(|x| {
                (height - 1)
                    .checked_mul(stride_y)
                    .and_then(|y| x.checked_add(y))
            })
            .ok_or(TensorLayoutError::ArithmeticOverflow)?;
        Ok(Self {
            width,
            height,
            stride_x,
            stride_y,
            capacity,
        })
    }

    /// Creates a contiguous row-major layout with `x` as the fastest axis.
    pub fn row_major(
        width: usize,
        height: usize,
        capacity: usize,
    ) -> Result<Self, TensorLayoutError> {
        Self::new(width, height, 1, width, capacity)
    }

    /// Returns the logical width.
    #[must_use]
    pub const fn width(self) -> usize {
        self.width
    }

    /// Returns the logical height.
    #[must_use]
    pub const fn height(self) -> usize {
        self.height
    }

    /// Returns the x-axis element stride.
    #[must_use]
    pub const fn stride_x(self) -> usize {
        self.stride_x
    }

    /// Returns the y-axis element stride.
    #[must_use]
    pub const fn stride_y(self) -> usize {
        self.stride_y
    }

    /// Returns the physical capacity in elements.
    #[must_use]
    pub const fn capacity(self) -> usize {
        self.capacity
    }

    /// Maps a logical coordinate to a physical element when both bounds hold.
    #[must_use]
    pub fn physical_index(self, x: usize, y: usize) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let x_offset = x.checked_mul(self.stride_x)?;
        let y_offset = y.checked_mul(self.stride_y)?;
        let index = x_offset.checked_add(y_offset)?;
        (index < self.capacity).then_some(index)
    }
}

/// Overflow-safe 3D affine tensor layout used by host differential oracles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TensorLayout3D {
    width: usize,
    height: usize,
    depth: usize,
    stride_x: usize,
    stride_y: usize,
    stride_z: usize,
    capacity: usize,
}

impl TensorLayout3D {
    /// Creates a positive 3D affine layout and checks its maximum index.
    pub fn new(
        width: usize,
        height: usize,
        depth: usize,
        stride_x: usize,
        stride_y: usize,
        stride_z: usize,
        capacity: usize,
    ) -> Result<Self, TensorLayoutError> {
        if width == 0
            || height == 0
            || depth == 0
            || stride_x == 0
            || stride_y == 0
            || stride_z == 0
            || capacity == 0
        {
            return Err(TensorLayoutError::ZeroComponent);
        }
        let max_index = (width - 1)
            .checked_mul(stride_x)
            .and_then(|x| {
                (height - 1)
                    .checked_mul(stride_y)
                    .and_then(|y| x.checked_add(y))
            })
            .and_then(|xy| {
                (depth - 1)
                    .checked_mul(stride_z)
                    .and_then(|z| xy.checked_add(z))
            })
            .ok_or(TensorLayoutError::ArithmeticOverflow)?;
        let _ = max_index;
        Ok(Self {
            width,
            height,
            depth,
            stride_x,
            stride_y,
            stride_z,
            capacity,
        })
    }

    /// Creates a contiguous row-major layout with `x` as the fastest axis.
    pub fn row_major(
        width: usize,
        height: usize,
        depth: usize,
        capacity: usize,
    ) -> Result<Self, TensorLayoutError> {
        let stride_z = width
            .checked_mul(height)
            .ok_or(TensorLayoutError::ArithmeticOverflow)?;
        Self::new(width, height, depth, 1, width, stride_z, capacity)
    }

    /// Returns the logical width.
    #[must_use]
    pub const fn width(self) -> usize {
        self.width
    }

    /// Returns the logical height.
    #[must_use]
    pub const fn height(self) -> usize {
        self.height
    }

    /// Returns the logical depth.
    #[must_use]
    pub const fn depth(self) -> usize {
        self.depth
    }

    /// Returns the physical capacity in elements.
    #[must_use]
    pub const fn capacity(self) -> usize {
        self.capacity
    }

    /// Maps a logical coordinate to a physical element when all bounds hold.
    #[must_use]
    pub fn physical_index(self, x: usize, y: usize, z: usize) -> Option<usize> {
        if x >= self.width || y >= self.height || z >= self.depth {
            return None;
        }
        let x_offset = x.checked_mul(self.stride_x)?;
        let y_offset = y.checked_mul(self.stride_y)?;
        let z_offset = z.checked_mul(self.stride_z)?;
        let xy = x_offset.checked_add(y_offset)?;
        let index = xy.checked_add(z_offset)?;
        (index < self.capacity).then_some(index)
    }
}

/// Explicit dispatch preference supplied by the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetPreference {
    Cpu,
    Gpu,
    Auto,
}

/// Floating-point contract relevant to target selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FpPolicy {
    Strict,
    Fast,
    Deterministic,
}

/// Inputs to the explicit CPU/GPU route decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectionRequest {
    pub preference: TargetPreference,
    pub fp: FpPolicy,
    pub gpu_available: bool,
    pub gpu_supports_fp: bool,
    pub transfer_cost: u64,
    pub estimated_cpu_cost: u64,
    pub estimated_gpu_cost: u64,
    pub allow_cpu_fallback: bool,
}

/// Why a target was selected or why the CPU fallback was used.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionReason {
    ExplicitCpu,
    ExplicitGpu,
    AutoGpuFaster,
    GpuUnavailable,
    GpuFpUnsupported,
    TransferTooExpensive,
    GpuNotFaster,
}

/// Result of a route decision; no device is loaded or command is submitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DispatchDecision {
    pub target: TargetPreference,
    pub reason: SelectionReason,
}

/// Selection failures when the host forbids fallback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionError {
    GpuUnavailable,
    GpuFpUnsupported,
    FallbackDisabled,
}

impl fmt::Display for SelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GpuUnavailable => {
                formatter.write_str("GPU target was requested but no GPU is available")
            }
            Self::GpuFpUnsupported => {
                formatter.write_str("GPU target does not support the requested FP policy")
            }
            Self::FallbackDisabled => {
                formatter.write_str("CPU fallback is disabled for this selection")
            }
        }
    }
}

impl Error for SelectionError {}

/// Applies an explicit route policy using host-provided capability/cost inputs.
pub fn select_target(request: SelectionRequest) -> Result<DispatchDecision, SelectionError> {
    if request.preference == TargetPreference::Cpu {
        return Ok(DispatchDecision {
            target: TargetPreference::Cpu,
            reason: SelectionReason::ExplicitCpu,
        });
    }

    let fallback = |reason| {
        if request.allow_cpu_fallback {
            Ok(DispatchDecision {
                target: TargetPreference::Cpu,
                reason,
            })
        } else {
            Err(SelectionError::FallbackDisabled)
        }
    };
    if !request.gpu_available {
        return if request.preference == TargetPreference::Gpu && !request.allow_cpu_fallback {
            Err(SelectionError::GpuUnavailable)
        } else {
            fallback(SelectionReason::GpuUnavailable)
        };
    }
    if !request.gpu_supports_fp {
        return if request.preference == TargetPreference::Gpu && !request.allow_cpu_fallback {
            Err(SelectionError::GpuFpUnsupported)
        } else {
            fallback(SelectionReason::GpuFpUnsupported)
        };
    }
    if request.preference == TargetPreference::Gpu {
        return Ok(DispatchDecision {
            target: TargetPreference::Gpu,
            reason: SelectionReason::ExplicitGpu,
        });
    }
    let total_gpu_cost = request
        .estimated_gpu_cost
        .saturating_add(request.transfer_cost);
    if total_gpu_cost < request.estimated_cpu_cost {
        Ok(DispatchDecision {
            target: TargetPreference::Gpu,
            reason: SelectionReason::AutoGpuFaster,
        })
    } else if request.transfer_cost >= request.estimated_cpu_cost {
        fallback(SelectionReason::TransferTooExpensive)
    } else {
        fallback(SelectionReason::GpuNotFaster)
    }
}

/// Comparison policy for CPU/GPU differential outputs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DifferentialPolicy {
    /// Compare IEEE bits, including signed zero and NaN payload.
    Exact,
    /// Compare finite values with absolute and relative tolerance.
    FloatTolerance { absolute: f32, relative: f32 },
}

/// Differential comparison failure.
#[derive(Clone, Debug, PartialEq)]
pub enum DifferentialError {
    LengthMismatch {
        expected: usize,
        actual: usize,
    },
    InvalidTolerance,
    ValueMismatch {
        index: usize,
        expected_bits: u32,
        actual_bits: u32,
        absolute_error_bits: u32,
    },
}

impl fmt::Display for DifferentialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LengthMismatch { expected, actual } => {
                write!(
                    formatter,
                    "differential length mismatch: expected {expected}, actual {actual}"
                )
            }
            Self::InvalidTolerance => {
                formatter.write_str("differential tolerance must be finite and non-negative")
            }
            Self::ValueMismatch {
                index,
                expected_bits,
                actual_bits,
                absolute_error_bits,
            } => write!(
                formatter,
                "differential mismatch at {index}: expected 0x{expected_bits:08x}, actual 0x{actual_bits:08x}, abs_error_bits 0x{absolute_error_bits:08x}"
            ),
        }
    }
}

impl Error for DifferentialError {}

/// Compares f32 outputs according to strict/deterministic or fast policy.
pub fn compare_f32(
    expected: &[f32],
    actual: &[f32],
    policy: DifferentialPolicy,
) -> Result<(), DifferentialError> {
    if expected.len() != actual.len() {
        return Err(DifferentialError::LengthMismatch {
            expected: expected.len(),
            actual: actual.len(),
        });
    }
    let tolerance = match policy {
        DifferentialPolicy::Exact => None,
        DifferentialPolicy::FloatTolerance { absolute, relative }
            if absolute.is_finite()
                && relative.is_finite()
                && absolute >= 0.0
                && relative >= 0.0 =>
        {
            Some((absolute, relative))
        }
        DifferentialPolicy::FloatTolerance { .. } => {
            return Err(DifferentialError::InvalidTolerance);
        }
    };
    for (index, (&expected_value, &actual_value)) in expected.iter().zip(actual).enumerate() {
        let exact = expected_value.to_bits() == actual_value.to_bits();
        let equal = exact
            || tolerance.is_some_and(|(absolute, relative)| {
                if expected_value.is_nan() && actual_value.is_nan() {
                    true
                } else if expected_value.is_infinite() || actual_value.is_infinite() {
                    expected_value == actual_value
                } else {
                    let error = (expected_value - actual_value).abs();
                    error <= absolute + relative * expected_value.abs().max(actual_value.abs())
                }
            });
        if !equal {
            let absolute_error = (expected_value - actual_value).abs();
            return Err(DifferentialError::ValueMismatch {
                index,
                expected_bits: expected_value.to_bits(),
                actual_bits: actual_value.to_bits(),
                absolute_error_bits: absolute_error.to_bits(),
            });
        }
    }
    Ok(())
}

/// Compares exact `u32` outputs for integer CPU/GPU kernels.
pub fn compare_u32(expected: &[u32], actual: &[u32]) -> Result<(), DifferentialError> {
    if expected.len() != actual.len() {
        return Err(DifferentialError::LengthMismatch {
            expected: expected.len(),
            actual: actual.len(),
        });
    }
    for (index, (&expected_value, &actual_value)) in expected.iter().zip(actual).enumerate() {
        if expected_value != actual_value {
            return Err(DifferentialError::ValueMismatch {
                index,
                expected_bits: expected_value,
                actual_bits: actual_value,
                absolute_error_bits: expected_value.abs_diff(actual_value),
            });
        }
    }
    Ok(())
}

/// Snapshot of one buffer's public state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BufferInfo {
    pub size: u64,
    pub residency: Residency,
    pub live_accesses: usize,
}

/// Safe host-side resource table used by the Vulkan adapter.
#[derive(Clone, Debug, Default)]
pub struct ResourceTable {
    next_id: u32,
    buffers: BTreeMap<BufferId, BufferState>,
}

impl ResourceTable {
    /// Creates an empty resource table.
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_id: 1,
            buffers: BTreeMap::new(),
        }
    }

    /// Creates a host-owned buffer descriptor.
    pub fn create_buffer(&mut self, size: u64) -> Result<BufferId, ResourceError> {
        if size == 0 {
            return Err(ResourceError::InvalidSize);
        }
        let id = BufferId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(ResourceError::IdExhausted)?;
        self.buffers.insert(
            id,
            BufferState {
                size,
                residency: Residency::Host,
                next_generation: 1,
                active: Vec::new(),
            },
        );
        Ok(id)
    }

    /// Returns a state snapshot.
    pub fn info(&self, id: BufferId) -> Result<BufferInfo, ResourceError> {
        let state = self
            .buffers
            .get(&id)
            .ok_or(ResourceError::UnknownBuffer(id))?;
        Ok(BufferInfo {
            size: state.size,
            residency: state.residency,
            live_accesses: state.active.len(),
        })
    }

    /// Promotes a host buffer to device residency.
    pub fn make_resident(&mut self, id: BufferId) -> Result<(), ResourceError> {
        let state = self.state_mut(id)?;
        if state.residency != Residency::Host {
            return Err(ResourceError::AlreadyResident(id));
        }
        state.residency = Residency::Device;
        Ok(())
    }

    /// Promotes a host buffer to explicitly shared residency.
    pub fn make_shared(&mut self, id: BufferId) -> Result<(), ResourceError> {
        let state = self.state_mut(id)?;
        if state.residency != Residency::Host {
            return Err(ResourceError::AlreadyResident(id));
        }
        state.residency = Residency::Shared;
        Ok(())
    }

    /// Acquires a live access token after conflict checking.
    pub fn acquire(
        &mut self,
        id: BufferId,
        kind: AccessKind,
    ) -> Result<AccessToken, ResourceError> {
        let state = self.state_mut(id)?;
        if state.residency == Residency::Host {
            return Err(ResourceError::NotResident(id));
        }
        if state
            .active
            .iter()
            .any(|active| active.kind.conflicts(kind))
        {
            return Err(ResourceError::AccessConflict(id));
        }
        let generation = state.next_generation;
        state.next_generation = state
            .next_generation
            .checked_add(1)
            .ok_or(ResourceError::IdExhausted)?;
        state.active.push(ActiveAccess { generation, kind });
        Ok(AccessToken {
            buffer: id,
            generation,
            kind,
        })
    }

    /// Releases a live access token and makes the completion observable.
    pub fn release(&mut self, token: AccessToken) -> Result<(), ResourceError> {
        let state = self.state_mut(token.buffer)?;
        let Some(index) = state
            .active
            .iter()
            .position(|active| active.generation == token.generation && active.kind == token.kind)
        else {
            return Err(ResourceError::StaleToken(token.buffer));
        };
        state.active.remove(index);
        Ok(())
    }

    /// Acquires all resource accesses required by a validated artifact plan.
    ///
    /// Acquisition is transactional: if any binding cannot become live, all
    /// tokens acquired earlier in this call are released before the error is
    /// returned. Native adapters can therefore encode descriptors only while
    /// holding one explicit lease for the complete dispatch scope.
    pub fn acquire_artifact_resources(
        &mut self,
        plan: &ArtifactResourcePlan,
    ) -> Result<ArtifactResourceLease, ResourceError> {
        let mut tokens = Vec::with_capacity(plan.bindings.len());
        for binding in &plan.bindings {
            match self.acquire(binding.buffer, binding.access) {
                Ok(token) => tokens.push(token),
                Err(error) => {
                    for token in tokens.into_iter().rev() {
                        let _ = self.release(token);
                    }
                    return Err(error);
                }
            }
        }
        Ok(ArtifactResourceLease {
            plan: plan.clone(),
            tokens,
        })
    }

    /// Releases every access token held by an artifact resource lease.
    pub fn release_artifact_resources(
        &mut self,
        lease: ArtifactResourceLease,
    ) -> Result<(), ResourceError> {
        for token in lease.tokens {
            self.release(token)?;
        }
        Ok(())
    }

    /// Releases the resource scope owned by a prepared artifact dispatch.
    pub fn release_prepared_artifact_dispatch(
        &mut self,
        dispatch: PreparedArtifactDispatch,
    ) -> Result<(), ResourceError> {
        self.release_artifact_resources(dispatch.lease)
    }

    /// Evicts a device/shared buffer only after all command scopes released it.
    pub fn evict(&mut self, id: BufferId) -> Result<(), ResourceError> {
        let state = self.state_mut(id)?;
        if !state.active.is_empty() {
            return Err(ResourceError::Busy(id));
        }
        if state.residency == Residency::Host {
            return Err(ResourceError::NotResident(id));
        }
        state.residency = Residency::Host;
        Ok(())
    }

    fn state_mut(&mut self, id: BufferId) -> Result<&mut BufferState, ResourceError> {
        self.buffers
            .get_mut(&id)
            .ok_or(ResourceError::UnknownBuffer(id))
    }
}

/// GPU API family covered by the platform portability prototype.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GpuBackend {
    /// Native SPIR-V path used by the verified Vulkan runtime.
    Vulkan,
    /// DirectX 12 path, currently requiring an explicit SPIR-V to DXIL step.
    DirectX12,
    /// Apple Metal path, currently requiring a dedicated MSL lowering.
    Metal,
}

/// Shader artifact transport selected by a backend plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShaderTransport {
    /// Submit the emitted SPIR-V directly to the API.
    NativeSpirv,
    /// Translate emitted SPIR-V to DXIL through an explicit host tool/SDK.
    SpirvToDxil,
    /// Lower the verified subset to Metal Shading Language.
    Msl,
}

/// Canonical source/transport route for one GPU backend.
///
/// Keeping this mapping in one value prevents the capability planner,
/// prepared descriptor validation and native adapters from independently
/// reconstructing which source language belongs to a transport. `None`
/// explicitly represents the native SPIR-V Vulkan path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShaderTranslationRoute {
    /// Shader bytes/source transport expected by the backend.
    pub transport: ShaderTransport,
    /// Source language required before the native API, if any.
    pub source_backend: Option<ArtifactSourceBackend>,
}

impl ShaderTranslationRoute {
    /// Returns the canonical route for a backend family.
    #[must_use]
    pub const fn for_backend(backend: GpuBackend) -> Self {
        match backend {
            GpuBackend::Vulkan => Self {
                transport: ShaderTransport::NativeSpirv,
                source_backend: None,
            },
            GpuBackend::DirectX12 => Self {
                transport: ShaderTransport::SpirvToDxil,
                source_backend: Some(ArtifactSourceBackend::Hlsl),
            },
            GpuBackend::Metal => Self {
                transport: ShaderTransport::Msl,
                source_backend: Some(ArtifactSourceBackend::Msl),
            },
        }
    }
}

/// Completion primitive expected by the host adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionModel {
    /// Vulkan fence completion for the current native runtime.
    Fence,
    /// DirectX queue fence that can represent timeline progress.
    TimelineFence,
    /// Metal command-buffer completion callback/status.
    CommandBufferCompletion,
}

/// Host-provided capability probe for one platform backend.
///
/// The probe is intentionally data-only: constructing it never loads a GPU
/// library, creates a device, or submits work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackendProbe {
    /// Whether a usable device/queue was discovered by the host.
    pub device_available: bool,
    /// Whether storage buffers with the required layout are supported.
    pub storage_buffers: bool,
    /// Whether the backend exposes a one-dimensional global invocation ID.
    pub global_invocation_id_x: bool,
    /// Whether structured bounds predicates are supported.
    pub structured_bounds: bool,
    /// Whether deterministic f32 semantics have been verified for this host.
    pub deterministic_f32: bool,
    /// Whether an asynchronous completion primitive is available.
    pub async_completion: bool,
    /// Whether the required shader translation toolchain is installed.
    pub shader_translation_available: bool,
    /// Maximum supported product of workgroup dimensions.
    pub max_workgroup_size: u32,
}

impl BackendProbe {
    /// Returns a conservative prototype profile, without probing the host.
    #[must_use]
    pub const fn prototype(backend: GpuBackend) -> Self {
        match backend {
            GpuBackend::Vulkan => Self {
                device_available: true,
                storage_buffers: true,
                global_invocation_id_x: true,
                structured_bounds: true,
                deterministic_f32: true,
                async_completion: true,
                shader_translation_available: false,
                max_workgroup_size: 1024,
            },
            GpuBackend::DirectX12 => Self {
                device_available: false,
                storage_buffers: true,
                global_invocation_id_x: true,
                structured_bounds: true,
                deterministic_f32: false,
                async_completion: true,
                shader_translation_available: false,
                max_workgroup_size: 1024,
            },
            GpuBackend::Metal => Self {
                device_available: false,
                storage_buffers: true,
                global_invocation_id_x: true,
                structured_bounds: true,
                deterministic_f32: false,
                async_completion: true,
                shader_translation_available: false,
                max_workgroup_size: 1024,
            },
        }
    }
}

/// Requirements used to build a platform backend plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackendRequest {
    /// Requested floating-point contract.
    pub fp: FpPolicy,
    /// One-dimensional workgroup size required by the kernel.
    pub workgroup_size: u32,
    /// Whether the current bounded global-id array shape is required.
    pub require_bounded_global_u32_array: bool,
    /// Whether the host needs a completion handle that can span a frame.
    pub require_async_completion: bool,
}

/// Planned route from JIR/SPIR-V to one platform API.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackendPlan {
    /// Selected backend family.
    pub backend: GpuBackend,
    /// Shader artifact transport/lowering route.
    pub shader_transport: ShaderTransport,
    /// Completion model the host adapter must own.
    pub completion: CompletionModel,
    /// Whether an external shader translation/lowering step is required.
    pub requires_shader_translation: bool,
    /// Requested floating-point contract.
    pub fp: FpPolicy,
    /// Validated one-dimensional workgroup size.
    pub workgroup_size: u32,
}

/// Why a platform plan was rejected before any GPU API call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendPlanError {
    /// The host did not report a usable device/queue.
    DeviceUnavailable,
    /// Storage buffers are unavailable.
    StorageBuffersUnsupported,
    /// GlobalInvocationId.x is unavailable.
    GlobalInvocationIdUnsupported,
    /// Structured bounds predicates are unavailable.
    StructuredBoundsUnsupported,
    /// Deterministic f32 semantics have not been proven.
    DeterministicFpUnsupported,
    /// The API cannot provide the requested async completion primitive.
    AsyncCompletionUnsupported,
    /// Workgroup size exceeds the host-reported limit.
    WorkgroupSizeUnsupported,
    /// The required SPIR-V translation toolchain is missing.
    ShaderTranslationUnavailable,
}

impl fmt::Display for BackendPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::DeviceUnavailable => "GPU device is unavailable",
            Self::StorageBuffersUnsupported => "storage buffers are unsupported",
            Self::GlobalInvocationIdUnsupported => "GlobalInvocationId.x is unsupported",
            Self::StructuredBoundsUnsupported => "structured bounds predicates are unsupported",
            Self::DeterministicFpUnsupported => "deterministic f32 is not verified",
            Self::AsyncCompletionUnsupported => "async completion is unsupported",
            Self::WorkgroupSizeUnsupported => "workgroup size exceeds backend limit",
            Self::ShaderTranslationUnavailable => "shader translation toolchain is unavailable",
        };
        formatter.write_str(message)
    }
}

impl Error for BackendPlanError {}

/// Builds a capability-gated platform plan without loading an API or
/// submitting a command. Unsupported routes must be sent to the declared CPU
/// fallback by the caller.
pub fn plan_backend(
    backend: GpuBackend,
    probe: BackendProbe,
    request: BackendRequest,
) -> Result<BackendPlan, BackendPlanError> {
    if !probe.device_available {
        return Err(BackendPlanError::DeviceUnavailable);
    }
    if !probe.storage_buffers {
        return Err(BackendPlanError::StorageBuffersUnsupported);
    }
    if request.require_bounded_global_u32_array {
        if !probe.global_invocation_id_x {
            return Err(BackendPlanError::GlobalInvocationIdUnsupported);
        }
        if !probe.structured_bounds {
            return Err(BackendPlanError::StructuredBoundsUnsupported);
        }
    }
    if request.fp == FpPolicy::Deterministic && !probe.deterministic_f32 {
        return Err(BackendPlanError::DeterministicFpUnsupported);
    }
    if request.require_async_completion && !probe.async_completion {
        return Err(BackendPlanError::AsyncCompletionUnsupported);
    }
    if request.workgroup_size == 0 || request.workgroup_size > probe.max_workgroup_size {
        return Err(BackendPlanError::WorkgroupSizeUnsupported);
    }

    let route = ShaderTranslationRoute::for_backend(backend);
    let completion = match backend {
        GpuBackend::Vulkan => CompletionModel::Fence,
        GpuBackend::DirectX12 => {
            if !probe.shader_translation_available {
                return Err(BackendPlanError::ShaderTranslationUnavailable);
            }
            CompletionModel::TimelineFence
        }
        GpuBackend::Metal => CompletionModel::CommandBufferCompletion,
    };
    Ok(BackendPlan {
        backend,
        shader_transport: route.transport,
        completion,
        requires_shader_translation: route.source_backend.is_some(),
        fp: request.fp,
        workgroup_size: request.workgroup_size,
    })
}

/// Stable identity and ABI metadata for one validated SPIR-V artifact.
///
/// Native adapters use this boundary before creating an API shader module or
/// descriptor layout. It deliberately contains no backend handles and does
/// not imply that a backend can execute every artifact shape.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpirvArtifactIdentity {
    /// Entry-point spelling preserved from the portable artifact.
    pub entry_name: String,
    /// Local workgroup dimensions encoded by the artifact.
    pub workgroup_size: [u32; 3],
    /// Number of reflected resource bindings.
    pub resource_binding_count: usize,
    /// Number of SPIR-V words, including the five-word module header.
    pub word_count: usize,
    /// Stable FNV-1a hash of the little-endian SPIR-V words.
    pub word_hash: u64,
}

/// Host-side metadata failures before an artifact crosses a backend ABI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpirvArtifactContractError {
    /// The portable SPIR-V word stream is structurally invalid.
    InvalidSpirv(SpirvValidationError),
    /// Entry-point spelling is empty.
    EmptyEntryName,
    /// Entry-point spelling contains an embedded NUL.
    EntryNameContainsNul,
    /// At least one workgroup dimension is zero.
    ZeroWorkgroup([u32; 3]),
    /// Workgroup product cannot be represented or exceeds the portable limit.
    WorkgroupTooLarge(u64),
    /// Resource binding ordinals must be dense and declaration ordered.
    NonCanonicalBinding { expected: u32, actual: u32 },
    /// Resource names must be unique for descriptor diagnostics.
    DuplicateResourceName(String),
    /// A reflected resource has an empty name.
    EmptyResourceName(u32),
    /// A reflected resource name contains an embedded NUL.
    ResourceNameContainsNul(u32),
    /// A reflected resource advertised an impossible zero-byte element stride.
    InvalidResourceStride { binding: u32, stride: u32 },
    /// A reflected scalar/vector type metadata record is malformed.
    InvalidResourceTypeMetadata { binding: u32 },
    /// Reflected type layout disagrees with the explicit stride metadata.
    ResourceTypeStrideMismatch {
        binding: u32,
        type_stride: u32,
        stride: u32,
    },
    /// A backend source translator returned an empty source string.
    EmptySourceOutput,
}

impl fmt::Display for SpirvArtifactContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSpirv(error) => write!(formatter, "invalid SPIR-V artifact: {error}"),
            Self::EmptyEntryName => formatter.write_str("SPIR-V artifact entry name is empty"),
            Self::EntryNameContainsNul => {
                formatter.write_str("SPIR-V artifact entry name contains NUL")
            }
            Self::ZeroWorkgroup(size) => {
                write!(formatter, "SPIR-V artifact has zero workgroup {size:?}")
            }
            Self::WorkgroupTooLarge(product) => {
                write!(
                    formatter,
                    "SPIR-V artifact workgroup product {product} exceeds 1024"
                )
            }
            Self::NonCanonicalBinding { expected, actual } => write!(
                formatter,
                "SPIR-V artifact binding is not dense: expected {expected}, got {actual}"
            ),
            Self::DuplicateResourceName(name) => {
                write!(
                    formatter,
                    "SPIR-V artifact has duplicate resource name `{name}`"
                )
            }
            Self::EmptyResourceName(binding) => {
                write!(
                    formatter,
                    "SPIR-V artifact resource {binding} has an empty name"
                )
            }
            Self::ResourceNameContainsNul(binding) => write!(
                formatter,
                "SPIR-V artifact resource {binding} name contains NUL"
            ),
            Self::InvalidResourceStride { binding, stride } => write!(
                formatter,
                "SPIR-V artifact resource {binding} has invalid element stride {stride}"
            ),
            Self::InvalidResourceTypeMetadata { binding } => write!(
                formatter,
                "SPIR-V artifact resource {binding} has invalid element type metadata"
            ),
            Self::ResourceTypeStrideMismatch {
                binding,
                type_stride,
                stride,
            } => write!(
                formatter,
                "SPIR-V artifact resource {binding} type stride {type_stride} differs from metadata stride {stride}"
            ),
            Self::EmptySourceOutput => {
                formatter.write_str("backend source translation returned empty output")
            }
        }
    }
}

impl Error for SpirvArtifactContractError {}

/// Computes the cross-backend FNV-1a identity hash for SPIR-V words.
#[must_use]
pub fn stable_spirv_word_hash(words: &[u32]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    words.iter().fold(OFFSET_BASIS, |hash, word| {
        word.to_le_bytes().into_iter().fold(hash, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(PRIME)
        })
    })
}

/// Computes the cross-backend FNV-1a identity hash for generated source bytes.
#[must_use]
pub fn stable_source_hash(source: &str) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    source.as_bytes().iter().fold(OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
    })
}

/// Layout classification exposed to source translators without reconstructing
/// the session-local JIR type table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactResourceLayout {
    /// Scalar/vector identity is known; stride can still be unknown for a
    /// non-byte-addressable internal type.
    ScalarVector {
        element: ResourceElementType,
        stride: Option<u32>,
    },
    /// Composite or otherwise opaque layout is intentionally not claimed;
    /// an explicit byte stride may still be preserved independently.
    Opaque { stride: Option<u32> },
}

/// Portable capability facts for one artifact resource binding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactResourceCapability {
    /// Dense artifact binding ordinal.
    pub binding: u32,
    /// Descriptor set/space carried by the artifact.
    pub descriptor_set: u32,
    /// JIR address space preserved by reflection.
    pub address_space: AddressSpace,
    /// Conservative resource access contract.
    pub access: ResourceAccess,
    /// Layout information available to a backend source translator.
    pub layout: ArtifactResourceLayout,
}

/// Returns the validated resource capability matrix for one artifact.
pub fn artifact_resource_capability_matrix(
    artifact: &SpirvArtifact,
) -> Result<Vec<ArtifactResourceCapability>, SpirvArtifactContractError> {
    validate_spirv_artifact_contract(artifact)?;
    Ok(artifact
        .resources
        .iter()
        .map(|resource| ArtifactResourceCapability {
            binding: resource.binding,
            descriptor_set: resource.descriptor_set,
            address_space: resource.address_space,
            access: resource.access,
            layout: resource
                .element_type_info
                .map(|element| ArtifactResourceLayout::ScalarVector {
                    element,
                    stride: resource.element_stride,
                })
                .unwrap_or(ArtifactResourceLayout::Opaque {
                    stride: resource.element_stride,
                }),
        })
        .collect())
}

/// Identifies the source backend represented by a translation report.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactSourceBackend {
    /// Metal Shading Language source emitted for Metal/Apple targets.
    Msl,
    /// HLSL source emitted for DirectX shader targets.
    Hlsl,
}

/// Returns the target-specific SPIRV-Cross flags for one selected entry point.
///
/// Keeping the argument construction in one helper makes the MSL and HLSL
/// routes auditable together and prevents a multi-entry SPIR-V module from
/// silently translating the tool's default entry point on one backend.
fn spirv_cross_target_arguments(entry_name: &str, target: ArtifactSourceBackend) -> Vec<String> {
    match target {
        ArtifactSourceBackend::Msl => vec![
            "--msl".to_owned(),
            "--entry".to_owned(),
            entry_name.to_owned(),
            "--rename-entry-point".to_owned(),
            entry_name.to_owned(),
            entry_name.to_owned(),
            "comp".to_owned(),
            "--output".to_owned(),
        ],
        ArtifactSourceBackend::Hlsl => vec![
            "--hlsl".to_owned(),
            "--shader-model".to_owned(),
            "60".to_owned(),
            "--entry".to_owned(),
            entry_name.to_owned(),
            "--rename-entry-point".to_owned(),
            entry_name.to_owned(),
            entry_name.to_owned(),
            "comp".to_owned(),
            "--output".to_owned(),
        ],
    }
}

/// Failure while invoking the shared external SPIRV-Cross source translator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpirvCrossError {
    /// Temporary-file or process I/O failed.
    Io(String),
    /// SPIRV-Cross returned a non-success status.
    Process(String),
    /// SPIRV-Cross completed without producing source text.
    EmptyOutput,
}

impl fmt::Display for SpirvCrossError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(formatter, "SPIRV-Cross I/O failed: {message}"),
            Self::Process(message) => write!(formatter, "SPIRV-Cross process failed: {message}"),
            Self::EmptyOutput => formatter.write_str("SPIRV-Cross returned empty source"),
        }
    }
}

impl Error for SpirvCrossError {}

/// Runs the shared, shell-free SPIRV-Cross source translation boundary.
///
/// Backend crates remain responsible for SPIR-V structural validation and
/// source/resource reflection. This helper owns only deterministic temporary
/// input/output handling and target-specific SPIRV-Cross arguments, so MSL and
/// HLSL routes cannot drift in process lifecycle or cleanup semantics.
pub fn translate_spirv_with_spirv_cross(
    tool: &Path,
    words: &[u32],
    entry_name: &str,
    target: ArtifactSourceBackend,
) -> Result<String, SpirvCrossError> {
    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "jadren-spirv-cross-{}-{suffix}-{sequence}",
        std::process::id()
    ));
    fs::create_dir(&directory).map_err(|error| SpirvCrossError::Io(error.to_string()))?;
    let spirv_path = directory.join("kernel.spv");
    let source_extension = match target {
        ArtifactSourceBackend::Msl => "metal",
        ArtifactSourceBackend::Hlsl => "hlsl",
    };
    let source_path = directory.join(format!("kernel.{source_extension}"));
    let result = (|| {
        let mut bytes = Vec::with_capacity(std::mem::size_of_val(words));
        for word in words {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        fs::write(&spirv_path, bytes).map_err(|error| SpirvCrossError::Io(error.to_string()))?;

        let mut command = Command::new(tool);
        command.arg(&spirv_path);
        command.args(spirv_cross_target_arguments(entry_name, target));
        let output = command
            .arg(&source_path)
            .output()
            .map_err(|error| SpirvCrossError::Io(error.to_string()))?;
        if !output.status.success() {
            let diagnostics = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let message = if diagnostics.is_empty() {
                format!("status {}", output.status)
            } else {
                diagnostics
            };
            return Err(SpirvCrossError::Process(message));
        }
        let source = fs::read_to_string(&source_path)
            .map_err(|error| SpirvCrossError::Io(error.to_string()))?;
        if source.trim().is_empty() {
            return Err(SpirvCrossError::EmptyOutput);
        }
        Ok(source)
    })();
    let _ = fs::remove_dir_all(&directory);
    result
}

/// Portable input contract handed to a backend source translator before any
/// external toolchain or native API work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactSourceTranslationPlan {
    /// Source backend that is expected to consume the artifact.
    pub backend: ArtifactSourceBackend,
    /// Shared identity of the validated input artifact.
    pub artifact: SpirvArtifactIdentity,
    /// Resource capability facts available to the translator.
    pub resources: Vec<ArtifactResourceCapability>,
}

impl ArtifactSourceTranslationPlan {
    /// Validates the artifact and snapshots its portable source capabilities.
    pub fn from_artifact(
        artifact: &SpirvArtifact,
        backend: ArtifactSourceBackend,
    ) -> Result<Self, SpirvArtifactContractError> {
        let identity = validate_spirv_artifact_contract(artifact)?;
        let resources = artifact_resource_capability_matrix(artifact)?;
        Ok(Self {
            backend,
            artifact: identity,
            resources,
        })
    }

    /// Completes the plan with a non-empty backend source output.
    pub fn into_report(
        self,
        source: String,
    ) -> Result<ArtifactSourceTranslationReport, SpirvArtifactContractError> {
        if source.trim().is_empty() {
            return Err(SpirvArtifactContractError::EmptySourceOutput);
        }
        let source_byte_count = source.len();
        let source_hash = stable_source_hash(&source);
        Ok(ArtifactSourceTranslationReport {
            backend: self.backend,
            resources: self.resources,
            artifact: self.artifact,
            source,
            source_byte_count,
            source_hash,
        })
    }
}

/// Shared report for a validated SPIR-V artifact translated to backend source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactSourceTranslationReport {
    /// Source backend that produced the report.
    pub backend: ArtifactSourceBackend,
    /// Resource capability facts used by the source contract.
    pub resources: Vec<ArtifactResourceCapability>,
    /// Shared identity of the validated input artifact.
    pub artifact: SpirvArtifactIdentity,
    /// Backend source returned after backend-specific reflection validation.
    pub source: String,
    /// UTF-8 byte length of `source`.
    pub source_byte_count: usize,
    /// Stable FNV-1a hash of the UTF-8 source bytes.
    pub source_hash: u64,
}

impl ArtifactSourceTranslationReport {
    /// Builds a report after validating the artifact and requiring non-empty output.
    pub fn from_artifact(
        artifact: &SpirvArtifact,
        backend: ArtifactSourceBackend,
        source: String,
    ) -> Result<Self, SpirvArtifactContractError> {
        ArtifactSourceTranslationPlan::from_artifact(artifact, backend)?.into_report(source)
    }
}

/// Stable identity for a raw SPIR-V module before backend-specific resource
/// reflection is available.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpirvSourceTranslationIdentity {
    /// Source backend represented by the translation.
    pub backend: ArtifactSourceBackend,
    /// Explicit entry point selected for the external translator.
    pub entry_name: String,
    /// SPIR-V execution model carried by the selected entry point.
    pub execution_model: u32,
    /// Reflected `OpExecutionMode LocalSize` for the selected entry, when it
    /// is encoded as literal dimensions.
    pub workgroup_size: Option<[u32; 3]>,
    /// Reflected `OpExecutionModeId LocalSizeId` operands for the selected
    /// entry, when dimensions remain specialization-constant IDs.
    pub workgroup_size_ids: Option<[u32; 3]>,
    /// Reflected `SpecId` decorations for LocalSizeId operands, when all three
    /// dimensions have one, in x/y/z order.
    pub workgroup_size_spec_ids: Option<[u32; 3]>,
    /// Descriptor decorations and conservative reflected capabilities.
    pub resources: Vec<SpirvRawResourceBinding>,
    /// Number of SPIR-V words handed to the translator.
    pub word_count: usize,
    /// Stable FNV-1a hash of the SPIR-V words.
    pub word_hash: u64,
}

/// Conservative descriptor capability discovered directly from raw SPIR-V.
///
/// Type, stride and access are populated only when the pointer/array/struct
/// chain and storage class prove them without guessing. Unknown fields remain
/// explicit so a native adapter cannot silently widen the accepted contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpirvRawResourceBinding {
    /// SPIR-V result id decorated with `Binding`.
    pub variable_id: u32,
    /// Descriptor binding number.
    pub binding: u32,
    /// Descriptor set number, defaulting to SPIR-V set zero when omitted.
    pub descriptor_set: u32,
    /// `OpVariable` storage-class literal when the variable declaration was
    /// present in the raw module.
    pub storage_class: Option<u32>,
    /// Scalar/vector element type when the pointer/array/struct chain is
    /// unambiguous and supported by the portable type model.
    pub element_type: Option<ResourceElementType>,
    /// Byte stride derived from the reflected scalar/vector element type.
    pub element_stride: Option<u32>,
    /// Access policy derived from storage class and NonWritable/NonReadable
    /// decorations when present.
    pub access: Option<ResourceAccess>,
}

/// Backend-neutral contract snapshot for one selected raw SPIR-V entry point.
///
/// This is the common hand-off boundary for source translators and future
/// native adapters. It contains only facts proven from the module itself;
/// backend source reflection and command execution remain separate steps.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpirvRawModuleContract {
    /// Explicit entry point selected for the module.
    pub entry_name: String,
    /// SPIR-V execution model carried by the selected entry point.
    pub execution_model: u32,
    /// Reflected literal local workgroup dimensions, when present.
    pub workgroup_size: Option<[u32; 3]>,
    /// Reflected specialization-constant IDs for `LocalSizeId`, when present.
    pub workgroup_size_ids: Option<[u32; 3]>,
    /// External specialization IDs for LocalSizeId, when all are decorated.
    pub workgroup_size_spec_ids: Option<[u32; 3]>,
    /// Deterministically ordered descriptor/resource capabilities.
    pub resources: Vec<SpirvRawResourceBinding>,
    /// Number of words in the validated module.
    pub word_count: usize,
    /// Stable FNV-1a identity of the module words.
    pub word_hash: u64,
}

/// Stable schema for the first explicitly bounded Jadren GPU execution
/// subset. This is an exact-word allowlist, not a general SPIR-V capability
/// declaration.
pub const JADREN_GPU_SUPPORTED_SUBSET_SCHEMA: &str = "jadren-gpu-supported-subset-0.2";

/// Candidate schema for the complete already-declared GPU artifact family.
/// It becomes the active gate only after both native family runners cover all
/// exact identities.
pub const JADREN_GPU_SUPPORTED_SUBSET_SCHEMA_V0_3: &str = "jadren-gpu-supported-subset-0.3";

/// One exact SPIR-V family identity admitted by the first supported subset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GpuSupportedSubsetResource {
    /// Dense descriptor binding admitted by the exact case.
    pub binding: u32,
    /// Descriptor set admitted by the exact case.
    pub descriptor_set: u32,
    /// SPIR-V storage class admitted by the exact case.
    pub storage_class: u32,
    /// Exact reflected scalar/vector element type.
    pub element_type: ResourceElementType,
    /// Exact reflected byte stride.
    pub element_stride: u32,
    /// Exact reflected read/write capability.
    pub access: ResourceAccess,
}

/// One exact SPIR-V family identity admitted by the first supported subset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GpuSupportedSubsetCase {
    /// Stable case identifier used at the admission boundary.
    pub id: &'static str,
    /// Human-readable scalar/vector shape.
    pub shape: &'static str,
    /// Arithmetic operation represented by the exact module.
    pub operation: &'static str,
    /// Required compute entry point.
    pub entry_name: &'static str,
    /// Exact canonical SPIR-V word count.
    pub word_count: usize,
    /// Exact stable FNV-1a identity of the canonical SPIR-V words.
    pub word_hash: u64,
    /// Exact ordered descriptor/resource contract. The slice length is part of
    /// the admitted identity and is not restricted to three bindings.
    pub resources: &'static [GpuSupportedSubsetResource],
    /// Explicit output/readback binding selected by the execution contract.
    pub output_binding: u32,
    /// Literal local workgroup size admitted by this revision.
    pub workgroup_size: [u32; 3],
}

/// Versioned, exact-word supported-subset manifest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GpuSupportedSubsetManifest {
    /// Stable manifest schema.
    pub schema: &'static str,
    /// Number of exact cases carried by the manifest.
    pub case_count: usize,
    /// Canonical case order used by reports and cross-device gates.
    pub cases: &'static [GpuSupportedSubsetCase],
    /// Explicitly bounded claim carried with the manifest.
    pub claim_scope: &'static str,
}

/// First exact-word Jadren GPU supported-subset manifest.
const GPU_SUBSET_U32_DYNAMIC_RESOURCES: [GpuSupportedSubsetResource; 3] = [
    GpuSupportedSubsetResource {
        binding: 0,
        descriptor_set: 0,
        storage_class: 12,
        element_type: ResourceElementType::Integer {
            signed: false,
            bits: 32,
            lanes: 1,
        },
        element_stride: 4,
        access: ResourceAccess::ReadOnly,
    },
    GpuSupportedSubsetResource {
        binding: 1,
        descriptor_set: 0,
        storage_class: 12,
        element_type: ResourceElementType::Integer {
            signed: false,
            bits: 32,
            lanes: 1,
        },
        element_stride: 4,
        access: ResourceAccess::WriteOnly,
    },
    GpuSupportedSubsetResource {
        binding: 2,
        descriptor_set: 0,
        storage_class: 12,
        element_type: ResourceElementType::Integer {
            signed: false,
            bits: 32,
            lanes: 1,
        },
        element_stride: 4,
        access: ResourceAccess::ReadOnly,
    },
];

const GPU_SUBSET_F32_DYNAMIC_RESOURCES: [GpuSupportedSubsetResource; 3] = [
    GpuSupportedSubsetResource {
        binding: 0,
        descriptor_set: 0,
        storage_class: 12,
        element_type: ResourceElementType::Float { bits: 32, lanes: 1 },
        element_stride: 4,
        access: ResourceAccess::ReadOnly,
    },
    GpuSupportedSubsetResource {
        binding: 1,
        descriptor_set: 0,
        storage_class: 12,
        element_type: ResourceElementType::Float { bits: 32, lanes: 1 },
        element_stride: 4,
        access: ResourceAccess::WriteOnly,
    },
    GPU_SUBSET_U32_DYNAMIC_RESOURCES[2],
];

const GPU_SUBSET_F32X4_DYNAMIC_RESOURCES: [GpuSupportedSubsetResource; 3] = [
    GpuSupportedSubsetResource {
        binding: 0,
        descriptor_set: 0,
        storage_class: 12,
        element_type: ResourceElementType::Float { bits: 32, lanes: 4 },
        element_stride: 16,
        access: ResourceAccess::ReadOnly,
    },
    GpuSupportedSubsetResource {
        binding: 1,
        descriptor_set: 0,
        storage_class: 12,
        element_type: ResourceElementType::Float { bits: 32, lanes: 4 },
        element_stride: 16,
        access: ResourceAccess::WriteOnly,
    },
    GPU_SUBSET_U32_DYNAMIC_RESOURCES[2],
];

const GPU_SUBSET_F32X2_DYNAMIC_RESOURCES: [GpuSupportedSubsetResource; 3] = [
    GpuSupportedSubsetResource {
        binding: 0,
        descriptor_set: 0,
        storage_class: 12,
        element_type: ResourceElementType::Float { bits: 32, lanes: 2 },
        element_stride: 8,
        access: ResourceAccess::ReadOnly,
    },
    GpuSupportedSubsetResource {
        binding: 1,
        descriptor_set: 0,
        storage_class: 12,
        element_type: ResourceElementType::Float { bits: 32, lanes: 2 },
        element_stride: 8,
        access: ResourceAccess::WriteOnly,
    },
    GPU_SUBSET_U32_DYNAMIC_RESOURCES[2],
];

const GPU_SUBSET_F32X3_DYNAMIC_RESOURCES: [GpuSupportedSubsetResource; 3] = [
    GpuSupportedSubsetResource {
        binding: 0,
        descriptor_set: 0,
        storage_class: 12,
        element_type: ResourceElementType::Float { bits: 32, lanes: 3 },
        element_stride: 12,
        access: ResourceAccess::ReadOnly,
    },
    GpuSupportedSubsetResource {
        binding: 1,
        descriptor_set: 0,
        storage_class: 12,
        element_type: ResourceElementType::Float { bits: 32, lanes: 3 },
        element_stride: 12,
        access: ResourceAccess::WriteOnly,
    },
    GPU_SUBSET_U32_DYNAMIC_RESOURCES[2],
];

const fn gpu_subset_u32_resource(
    binding: u32,
    access: ResourceAccess,
) -> GpuSupportedSubsetResource {
    GpuSupportedSubsetResource {
        binding,
        descriptor_set: 0,
        storage_class: 12,
        element_type: ResourceElementType::Integer {
            signed: false,
            bits: 32,
            lanes: 1,
        },
        element_stride: 4,
        access,
    }
}

const GPU_SUBSET_WRITE_1_RESOURCES: [GpuSupportedSubsetResource; 1] =
    [gpu_subset_u32_resource(0, ResourceAccess::WriteOnly)];
const GPU_SUBSET_WRITE_4_RESOURCES: [GpuSupportedSubsetResource; 4] = [
    gpu_subset_u32_resource(0, ResourceAccess::WriteOnly),
    gpu_subset_u32_resource(1, ResourceAccess::ReadOnly),
    gpu_subset_u32_resource(2, ResourceAccess::ReadOnly),
    gpu_subset_u32_resource(3, ResourceAccess::ReadOnly),
];
const GPU_SUBSET_WRITE_5_RESOURCES: [GpuSupportedSubsetResource; 5] = [
    gpu_subset_u32_resource(0, ResourceAccess::WriteOnly),
    gpu_subset_u32_resource(1, ResourceAccess::ReadOnly),
    gpu_subset_u32_resource(2, ResourceAccess::ReadOnly),
    gpu_subset_u32_resource(3, ResourceAccess::ReadOnly),
    gpu_subset_u32_resource(4, ResourceAccess::ReadOnly),
];
const GPU_SUBSET_WRITE_6_RESOURCES: [GpuSupportedSubsetResource; 6] = [
    gpu_subset_u32_resource(0, ResourceAccess::WriteOnly),
    gpu_subset_u32_resource(1, ResourceAccess::ReadOnly),
    gpu_subset_u32_resource(2, ResourceAccess::ReadOnly),
    gpu_subset_u32_resource(3, ResourceAccess::ReadOnly),
    gpu_subset_u32_resource(4, ResourceAccess::ReadOnly),
    gpu_subset_u32_resource(5, ResourceAccess::ReadOnly),
];
const GPU_SUBSET_WRITE_8_RESOURCES: [GpuSupportedSubsetResource; 8] = [
    gpu_subset_u32_resource(0, ResourceAccess::WriteOnly),
    gpu_subset_u32_resource(1, ResourceAccess::ReadOnly),
    gpu_subset_u32_resource(2, ResourceAccess::ReadOnly),
    gpu_subset_u32_resource(3, ResourceAccess::ReadOnly),
    gpu_subset_u32_resource(4, ResourceAccess::ReadOnly),
    gpu_subset_u32_resource(5, ResourceAccess::ReadOnly),
    gpu_subset_u32_resource(6, ResourceAccess::ReadOnly),
    gpu_subset_u32_resource(7, ResourceAccess::ReadOnly),
];

macro_rules! gpu_subset_case {
    ($id:literal, $shape:literal, $operation:literal, $entry:literal, $words:literal, $hash:literal, $resources:expr, $output:literal, $workgroup:expr) => {
        GpuSupportedSubsetCase {
            id: $id,
            shape: $shape,
            operation: $operation,
            entry_name: $entry,
            word_count: $words,
            word_hash: $hash,
            resources: $resources,
            output_binding: $output,
            workgroup_size: $workgroup,
        }
    };
}

const GPU_SUPPORTED_SUBSET_CASES_V0_2: [GpuSupportedSubsetCase; 7] = [
    GpuSupportedSubsetCase {
        id: "u32.multiply",
        shape: "u32",
        operation: "multiply",
        entry_name: "global_multiply_dynamic_u32",
        word_count: 206,
        word_hash: 7_083_140_853_810_565_206,
        resources: &GPU_SUBSET_U32_DYNAMIC_RESOURCES,
        output_binding: 1,
        workgroup_size: [64, 1, 1],
    },
    GpuSupportedSubsetCase {
        id: "f32.add",
        shape: "f32",
        operation: "add",
        entry_name: "global_add_dynamic_f32",
        word_count: 234,
        word_hash: 14_373_886_480_406_866_037,
        resources: &GPU_SUBSET_F32_DYNAMIC_RESOURCES,
        output_binding: 1,
        workgroup_size: [64, 1, 1],
    },
    GpuSupportedSubsetCase {
        id: "f32.subtract",
        shape: "f32",
        operation: "subtract",
        entry_name: "global_subtract_dynamic_f32",
        word_count: 235,
        word_hash: 16_121_106_323_021_775_875,
        resources: &GPU_SUBSET_F32_DYNAMIC_RESOURCES,
        output_binding: 1,
        workgroup_size: [64, 1, 1],
    },
    GpuSupportedSubsetCase {
        id: "f32.multiply",
        shape: "f32",
        operation: "multiply",
        entry_name: "global_multiply_dynamic_f32",
        word_count: 235,
        word_hash: 4_313_561_119_742_553_448,
        resources: &GPU_SUBSET_F32_DYNAMIC_RESOURCES,
        output_binding: 1,
        workgroup_size: [64, 1, 1],
    },
    GpuSupportedSubsetCase {
        id: "f32x4.add",
        shape: "f32x4",
        operation: "add",
        entry_name: "global_add_dynamic_f32x4",
        word_count: 246,
        word_hash: 6_890_100_016_184_533_701,
        resources: &GPU_SUBSET_F32X4_DYNAMIC_RESOURCES,
        output_binding: 1,
        workgroup_size: [64, 1, 1],
    },
    GpuSupportedSubsetCase {
        id: "f32x4.subtract",
        shape: "f32x4",
        operation: "subtract",
        entry_name: "global_subtract_dynamic_f32x4",
        word_count: 247,
        word_hash: 46_842_235_511_820_257,
        resources: &GPU_SUBSET_F32X4_DYNAMIC_RESOURCES,
        output_binding: 1,
        workgroup_size: [64, 1, 1],
    },
    GpuSupportedSubsetCase {
        id: "f32x4.multiply",
        shape: "f32x4",
        operation: "multiply",
        entry_name: "global_multiply_dynamic_f32x4",
        word_count: 247,
        word_hash: 12_070_409_480_524_215_210,
        resources: &GPU_SUBSET_F32X4_DYNAMIC_RESOURCES,
        output_binding: 1,
        workgroup_size: [64, 1, 1],
    },
];

pub const JADREN_GPU_SUPPORTED_SUBSET_V0_2: GpuSupportedSubsetManifest =
    GpuSupportedSubsetManifest {
        schema: JADREN_GPU_SUPPORTED_SUBSET_SCHEMA,
        case_count: 7,
        cases: &GPU_SUPPORTED_SUBSET_CASES_V0_2,
        claim_scope: "exact seven-case Jadren GPU SPIR-V allowlist; no general SPIR-V compatibility claim",
    };

const GPU_SUPPORTED_SUBSET_CASES_V0_3: [GpuSupportedSubsetCase; 28] = [
    gpu_subset_case!(
        "u32.add",
        "u32",
        "add",
        "global_add_dynamic_u32",
        205,
        2_219_801_068_313_404_206,
        &GPU_SUBSET_U32_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "u32.subtract",
        "u32",
        "subtract",
        "global_subtract_dynamic_u32",
        206,
        12_271_741_692_180_461_446,
        &GPU_SUBSET_U32_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "u32.multiply",
        "u32",
        "multiply",
        "global_multiply_dynamic_u32",
        206,
        13_781_217_453_249_545_831,
        &GPU_SUBSET_U32_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "u32.divide",
        "u32",
        "divide",
        "global_divide_dynamic_u32",
        206,
        11_338_163_566_336_487_842,
        &GPU_SUBSET_U32_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "u32.remainder",
        "u32",
        "remainder",
        "global_remainder_dynamic_u32",
        207,
        11_029_024_476_988_489_218,
        &GPU_SUBSET_U32_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "u32.bitand",
        "u32",
        "bitand",
        "global_bitand_dynamic_u32",
        206,
        1_156_939_840_803_408_275,
        &GPU_SUBSET_U32_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "u32.bitor",
        "u32",
        "bitor",
        "global_bitor_dynamic_u32",
        206,
        10_419_306_245_517_149_885,
        &GPU_SUBSET_U32_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "u32.bitxor",
        "u32",
        "bitxor",
        "global_bitxor_dynamic_u32",
        206,
        15_671_622_549_172_684_298,
        &GPU_SUBSET_U32_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "u32.shift-left",
        "u32",
        "shift-left",
        "global_shift_left_dynamic_u32",
        207,
        11_496_518_376_843_337_121,
        &GPU_SUBSET_U32_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "u32.shift-right",
        "u32",
        "shift-right",
        "global_shift_right_dynamic_u32",
        207,
        9_886_083_063_640_459_764,
        &GPU_SUBSET_U32_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "f32.add",
        "f32",
        "add",
        "global_add_dynamic_f32",
        234,
        14_373_886_480_406_866_037,
        &GPU_SUBSET_F32_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "f32.subtract",
        "f32",
        "subtract",
        "global_subtract_dynamic_f32",
        235,
        16_121_106_323_021_775_875,
        &GPU_SUBSET_F32_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "f32.multiply",
        "f32",
        "multiply",
        "global_multiply_dynamic_f32",
        235,
        4_313_561_119_742_553_448,
        &GPU_SUBSET_F32_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "f32x2.add",
        "f32x2",
        "add",
        "global_add_dynamic_f32x2",
        244,
        11_655_716_164_372_001_499,
        &GPU_SUBSET_F32X2_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "f32x2.subtract",
        "f32x2",
        "subtract",
        "global_subtract_dynamic_f32x2",
        245,
        11_254_852_716_316_632_427,
        &GPU_SUBSET_F32X2_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "f32x2.multiply",
        "f32x2",
        "multiply",
        "global_multiply_dynamic_f32x2",
        245,
        5_113_283_667_968_636_784,
        &GPU_SUBSET_F32X2_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "f32x3.add",
        "f32x3",
        "add",
        "global_add_dynamic_f32x3",
        245,
        9_775_786_045_145_014_126,
        &GPU_SUBSET_F32X3_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "f32x3.subtract",
        "f32x3",
        "subtract",
        "global_subtract_dynamic_f32x3",
        246,
        5_827_949_122_965_129_492,
        &GPU_SUBSET_F32X3_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "f32x3.multiply",
        "f32x3",
        "multiply",
        "global_multiply_dynamic_f32x3",
        246,
        5_800_665_520_213_002_591,
        &GPU_SUBSET_F32X3_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "f32x4.add",
        "f32x4",
        "add",
        "global_add_dynamic_f32x4",
        246,
        6_890_100_016_184_533_701,
        &GPU_SUBSET_F32X4_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "f32x4.subtract",
        "f32x4",
        "subtract",
        "global_subtract_dynamic_f32x4",
        247,
        46_842_235_511_820_257,
        &GPU_SUBSET_F32X4_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "f32x4.multiply",
        "f32x4",
        "multiply",
        "global_multiply_dynamic_f32x4",
        247,
        12_070_409_480_524_215_210,
        &GPU_SUBSET_F32X4_DYNAMIC_RESOURCES,
        1,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "u32.write.1d",
        "u32-1d",
        "write",
        "global_write_u32",
        151,
        708_152_213_268_147_838,
        &GPU_SUBSET_WRITE_1_RESOURCES,
        0,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "u32.write.1d-strided",
        "u32-1d-strided",
        "write",
        "global_strided_write_u32",
        250,
        10_654_542_140_158_150_823,
        &GPU_SUBSET_WRITE_4_RESOURCES,
        0,
        [64, 1, 1]
    ),
    gpu_subset_case!(
        "u32.write.2d",
        "u32-2d",
        "write",
        "global_2d_write_u32",
        268,
        7_346_394_778_153_296_564,
        &GPU_SUBSET_WRITE_4_RESOURCES,
        0,
        [8, 8, 1]
    ),
    gpu_subset_case!(
        "u32.write.2d-strided",
        "u32-2d-strided",
        "write",
        "global_2d_strided_write_u32",
        327,
        11_856_361_274_972_770_542,
        &GPU_SUBSET_WRITE_6_RESOURCES,
        0,
        [4, 4, 1]
    ),
    gpu_subset_case!(
        "u32.write.3d",
        "u32-3d",
        "write",
        "global_3d_write_u32",
        319,
        15_346_378_772_262_315_015,
        &GPU_SUBSET_WRITE_5_RESOURCES,
        0,
        [4, 4, 2]
    ),
    gpu_subset_case!(
        "u32.write.3d-strided",
        "u32-3d-strided",
        "write",
        "global_3d_strided_write_u32",
        404,
        15_787_395_013_528_098_493,
        &GPU_SUBSET_WRITE_8_RESOURCES,
        0,
        [4, 4, 2]
    ),
];

/// Candidate full-family manifest used by JAD-1314IF conformance work. The
/// active release gate remains V0_2 until DX12 and Metal execution reports
/// cover this entire list.
pub const JADREN_GPU_SUPPORTED_SUBSET_V0_3: GpuSupportedSubsetManifest =
    GpuSupportedSubsetManifest {
        schema: JADREN_GPU_SUPPORTED_SUBSET_SCHEMA_V0_3,
        case_count: 28,
        cases: &GPU_SUPPORTED_SUBSET_CASES_V0_3,
        claim_scope: "exact 28-case Jadren GPU SPIR-V allowlist candidate; no general SPIR-V compatibility claim",
    };

/// Why an input cannot enter the exact supported-subset execution boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GpuSupportedSubsetAdmissionError {
    /// The requested case ID is outside the versioned manifest.
    UnknownCase(String),
    /// The requested entry differs from the manifest entry.
    EntryMismatch {
        expected: &'static str,
        actual: String,
    },
    /// The raw SPIR-V structural/reflection boundary rejected the module.
    Source(SpirvSourceTranslationError),
    /// The selected entry is not a compute entry point.
    ExecutionModelMismatch { actual: u32 },
    /// Literal local size differs from the exact manifest rule.
    WorkgroupMismatch {
        expected: [u32; 3],
        actual: Option<[u32; 3]>,
    },
    /// Dense resource type/stride/access metadata differs from the manifest.
    ResourceContractMismatch,
    /// Canonical SPIR-V word count differs from the manifest.
    WordCountMismatch { expected: usize, actual: usize },
    /// Canonical SPIR-V word hash differs from the manifest.
    WordHashMismatch { expected: u64, actual: u64 },
}

impl fmt::Display for GpuSupportedSubsetAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCase(case) => write!(formatter, "GPU subset case `{case}` is unknown"),
            Self::EntryMismatch { expected, actual } => write!(
                formatter,
                "GPU subset expected entry `{expected}`, got `{actual}`"
            ),
            Self::Source(error) => write!(formatter, "GPU subset source contract failed: {error}"),
            Self::ExecutionModelMismatch { actual } => {
                write!(
                    formatter,
                    "GPU subset execution model {actual} is not compute"
                )
            }
            Self::WorkgroupMismatch { expected, actual } => write!(
                formatter,
                "GPU subset workgroup mismatch: expected {expected:?}, got {actual:?}"
            ),
            Self::ResourceContractMismatch => {
                formatter.write_str("GPU subset resource contract mismatch")
            }
            Self::WordCountMismatch { expected, actual } => write!(
                formatter,
                "GPU subset word count mismatch: expected {expected}, got {actual}"
            ),
            Self::WordHashMismatch { expected, actual } => write!(
                formatter,
                "GPU subset word hash mismatch: expected {expected}, got {actual}"
            ),
        }
    }
}

impl Error for GpuSupportedSubsetAdmissionError {}

/// Admits only one exact case from the versioned Jadren GPU supported subset.
/// Structural, workgroup and resource checks run before the final exact-word
/// identity check so malformed modules retain a useful rejection reason.
pub fn admit_gpu_supported_subset_v0_2(
    case_id: &str,
    words: &[u32],
    entry_name: &str,
) -> Result<GpuSupportedSubsetCase, GpuSupportedSubsetAdmissionError> {
    let expected = JADREN_GPU_SUPPORTED_SUBSET_V0_2
        .cases
        .iter()
        .find(|case| case.id == case_id)
        .copied()
        .ok_or_else(|| GpuSupportedSubsetAdmissionError::UnknownCase(case_id.to_owned()))?;
    admit_gpu_supported_subset_case(expected, words, entry_name)
}

/// Admits one exact case from the 28-case JAD-1314IF candidate manifest.
/// This API is a conformance boundary until V0_3 becomes the active release
/// gate after complete DX12 and Metal execution evidence exists.
pub fn admit_gpu_supported_subset_v0_3(
    case_id: &str,
    words: &[u32],
    entry_name: &str,
) -> Result<GpuSupportedSubsetCase, GpuSupportedSubsetAdmissionError> {
    let expected = JADREN_GPU_SUPPORTED_SUBSET_V0_3
        .cases
        .iter()
        .find(|case| case.id == case_id)
        .copied()
        .ok_or_else(|| GpuSupportedSubsetAdmissionError::UnknownCase(case_id.to_owned()))?;
    admit_gpu_supported_subset_case(expected, words, entry_name)
}

fn admit_gpu_supported_subset_case(
    expected: GpuSupportedSubsetCase,
    words: &[u32],
    entry_name: &str,
) -> Result<GpuSupportedSubsetCase, GpuSupportedSubsetAdmissionError> {
    if entry_name != expected.entry_name {
        return Err(GpuSupportedSubsetAdmissionError::EntryMismatch {
            expected: expected.entry_name,
            actual: entry_name.to_owned(),
        });
    }
    let contract = inspect_spirv_source_module(words, entry_name)
        .map_err(GpuSupportedSubsetAdmissionError::Source)?;
    if contract.execution_model != 5 {
        return Err(GpuSupportedSubsetAdmissionError::ExecutionModelMismatch {
            actual: contract.execution_model,
        });
    }
    if contract.workgroup_size != Some(expected.workgroup_size)
        || contract.workgroup_size_ids.is_some()
    {
        return Err(GpuSupportedSubsetAdmissionError::WorkgroupMismatch {
            expected: expected.workgroup_size,
            actual: contract.workgroup_size,
        });
    }
    let resources_match = contract.resources.len() == expected.resources.len()
        && contract.resources.iter().zip(expected.resources).all(
            |(resource, expected_resource)| {
                resource.binding == expected_resource.binding
                    && resource.descriptor_set == expected_resource.descriptor_set
                    && resource.storage_class == Some(expected_resource.storage_class)
                    && resource.element_type == Some(expected_resource.element_type)
                    && resource.element_stride == Some(expected_resource.element_stride)
                    && resource.access == Some(expected_resource.access)
            },
        );
    if !resources_match {
        return Err(GpuSupportedSubsetAdmissionError::ResourceContractMismatch);
    }
    if contract
        .resources
        .get(expected.output_binding as usize)
        .is_none_or(|resource| {
            resource.binding != expected.output_binding
                || !resource.access.is_some_and(ResourceAccess::can_write)
        })
    {
        return Err(GpuSupportedSubsetAdmissionError::ResourceContractMismatch);
    }
    if contract.word_count != expected.word_count {
        return Err(GpuSupportedSubsetAdmissionError::WordCountMismatch {
            expected: expected.word_count,
            actual: contract.word_count,
        });
    }
    if contract.word_hash != expected.word_hash {
        return Err(GpuSupportedSubsetAdmissionError::WordHashMismatch {
            expected: expected.word_hash,
            actual: contract.word_hash,
        });
    }
    Ok(expected)
}

/// Native adapter capability snapshot for a raw compute module.
///
/// This is a pre-API gate shared by future Vulkan/DX12/Metal adapters. It
/// accepts only compute execution, dense set-zero bindings and resource
/// capabilities that have a known scalar/vector type, byte stride and access
/// policy. It does not allocate, translate or submit work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpirvRawNativeAdapterPlan {
    /// Explicit compute entry point selected for the adapter.
    pub entry_name: String,
    /// SPIR-V execution model, guaranteed to be `GLCompute` (5).
    pub execution_model: u32,
    /// Literal local workgroup dimensions retained for a future encoder.
    pub workgroup_size: Option<[u32; 3]>,
    /// Specialization-constant IDs retained for a future specialization step.
    pub workgroup_size_ids: Option<[u32; 3]>,
    /// External specialization IDs retained for a future specialization step.
    pub workgroup_size_spec_ids: Option<[u32; 3]>,
    /// Dense resource capabilities forwarded to a native descriptor encoder.
    pub resources: Vec<SpirvRawResourceBinding>,
    /// Number of words in the validated module.
    pub word_count: usize,
    /// Stable module identity hash.
    pub word_hash: u64,
}

/// Explicit caller-selected output/readback binding for a validated raw plan.
///
/// Writable capability and output selection are intentionally separate: a
/// shader may expose multiple writable resources, while one execution call
/// chooses exactly one buffer to clear/read back.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpirvRawOutputSelection {
    /// Descriptor binding selected by the caller.
    pub binding: u32,
    /// Dense resource index used by backend payload arrays.
    pub resource_index: usize,
    /// Proven writable access capability of the selected resource.
    pub access: ResourceAccess,
}

/// Invalid explicit output/readback selection for a raw native plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpirvRawOutputSelectionError {
    /// No reflected resource uses the requested binding.
    MissingBinding { binding: u32 },
    /// The validated plan unexpectedly lost access metadata.
    MissingAccess { binding: u32 },
    /// The requested resource cannot be written by the shader.
    NotWritable {
        binding: u32,
        access: ResourceAccess,
    },
}

impl fmt::Display for SpirvRawOutputSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingBinding { binding } => {
                write!(formatter, "raw output binding {binding} is missing")
            }
            Self::MissingAccess { binding } => {
                write!(
                    formatter,
                    "raw output binding {binding} has no access metadata"
                )
            }
            Self::NotWritable { binding, access } => write!(
                formatter,
                "raw output binding {binding} is not writable ({access:?})"
            ),
        }
    }
}

impl Error for SpirvRawOutputSelectionError {}

/// Validates one explicit output/readback binding independently from the
/// resource capability reflection.
pub fn select_spirv_raw_output_binding(
    plan: &SpirvRawNativeAdapterPlan,
    binding: u32,
) -> Result<SpirvRawOutputSelection, SpirvRawOutputSelectionError> {
    let (resource_index, resource) = plan
        .resources
        .iter()
        .enumerate()
        .find(|(_, resource)| resource.binding == binding)
        .ok_or(SpirvRawOutputSelectionError::MissingBinding { binding })?;
    let access = resource
        .access
        .ok_or(SpirvRawOutputSelectionError::MissingAccess { binding })?;
    if !access.can_write() {
        return Err(SpirvRawOutputSelectionError::NotWritable { binding, access });
    }
    Ok(SpirvRawOutputSelection {
        binding,
        resource_index,
        access,
    })
}

/// Backend-specific resource view selected from a validated raw capability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpirvRawNativeResourceView {
    /// Vulkan storage-buffer descriptor.
    VulkanStorageBuffer,
    /// DirectX12 read-only SRV.
    DirectX12ShaderResource,
    /// DirectX12 writable UAV.
    DirectX12UnorderedAccess,
    /// Metal read-only `const device` pointer.
    MetalConstDevicePointer,
    /// Metal writable `device` pointer.
    MetalDevicePointer,
}

/// One resource binding after backend view selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpirvRawBackendResourceBinding {
    /// Dense descriptor binding ordinal.
    pub binding: u32,
    /// Descriptor set/space, currently guaranteed to be zero.
    pub descriptor_set: u32,
    /// Proven scalar/vector element type.
    pub element_type: ResourceElementType,
    /// Proven byte stride.
    pub element_stride: u32,
    /// Conservative read/write policy.
    pub access: ResourceAccess,
    /// Backend-specific descriptor/pointer view.
    pub view: SpirvRawNativeResourceView,
}

/// Backend projection consumed by a future native descriptor encoder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpirvRawBackendAdapterPlan {
    /// Backend receiving the projected view list.
    pub backend: GpuBackend,
    /// Selected compute entry point.
    pub entry_name: String,
    /// SPIR-V compute execution model.
    pub execution_model: u32,
    /// Literal local workgroup dimensions retained for a future encoder.
    pub workgroup_size: Option<[u32; 3]>,
    /// Specialization-constant IDs retained for a future specialization step.
    pub workgroup_size_ids: Option<[u32; 3]>,
    /// External specialization IDs retained for a future specialization step.
    pub workgroup_size_spec_ids: Option<[u32; 3]>,
    /// Resource views in dense binding order.
    pub resources: Vec<SpirvRawBackendResourceBinding>,
    /// Number of words in the validated module.
    pub word_count: usize,
    /// Stable module identity hash.
    pub word_hash: u64,
}

impl SpirvRawNativeAdapterPlan {
    /// Projects the validated raw capabilities to one backend's native view
    /// vocabulary without creating API handles or touching a device.
    #[must_use]
    pub fn project_backend(&self, backend: GpuBackend) -> SpirvRawBackendAdapterPlan {
        let resources = self
            .resources
            .iter()
            .map(|resource| {
                let access = resource.access.expect("native plan proves resource access");
                let view = match (backend, access) {
                    (GpuBackend::Vulkan, _) => SpirvRawNativeResourceView::VulkanStorageBuffer,
                    (GpuBackend::DirectX12, ResourceAccess::ReadOnly) => {
                        SpirvRawNativeResourceView::DirectX12ShaderResource
                    }
                    (
                        GpuBackend::DirectX12,
                        ResourceAccess::WriteOnly | ResourceAccess::ReadWrite,
                    ) => SpirvRawNativeResourceView::DirectX12UnorderedAccess,
                    (GpuBackend::Metal, ResourceAccess::ReadOnly) => {
                        SpirvRawNativeResourceView::MetalConstDevicePointer
                    }
                    (GpuBackend::Metal, ResourceAccess::WriteOnly | ResourceAccess::ReadWrite) => {
                        SpirvRawNativeResourceView::MetalDevicePointer
                    }
                };
                SpirvRawBackendResourceBinding {
                    binding: resource.binding,
                    descriptor_set: resource.descriptor_set,
                    element_type: resource
                        .element_type
                        .expect("native plan proves element type"),
                    element_stride: resource
                        .element_stride
                        .expect("native plan proves element stride"),
                    access,
                    view,
                }
            })
            .collect();
        SpirvRawBackendAdapterPlan {
            backend,
            entry_name: self.entry_name.clone(),
            execution_model: self.execution_model,
            workgroup_size: self.workgroup_size,
            workgroup_size_ids: self.workgroup_size_ids,
            workgroup_size_spec_ids: self.workgroup_size_spec_ids,
            resources,
            word_count: self.word_count,
            word_hash: self.word_hash,
        }
    }

    /// Resolves the local workgroup dimensions required by a native encoder.
    ///
    /// Literal `LocalSize` metadata needs no specialization values. For
    /// `LocalSizeId`, the caller must provide values together with the exact
    /// three IDs reflected from the module. This is only a geometry contract;
    /// it does not rewrite SPIR-V or compile a specialized backend artifact.
    pub fn resolve_workgroup_size(
        &self,
        specialization: Option<&SpirvRawWorkgroupSpecialization>,
    ) -> Result<[u32; 3], SpirvRawNativeAdapterError> {
        match (self.workgroup_size, self.workgroup_size_ids, specialization) {
            (Some(size), None, None) => validate_workgroup_dimensions(size),
            (Some(_), None, Some(_)) => {
                Err(SpirvRawNativeAdapterError::UnexpectedWorkgroupSpecialization)
            }
            (None, Some(_), None) => {
                Err(SpirvRawNativeAdapterError::MissingWorkgroupSpecialization)
            }
            (None, Some(ids), Some(values)) => {
                if values.ids != ids {
                    return Err(
                        SpirvRawNativeAdapterError::WorkgroupSpecializationIdsMismatch {
                            expected: ids,
                            actual: values.ids,
                        },
                    );
                }
                if values.spec_ids != self.workgroup_size_spec_ids {
                    return Err(
                        SpirvRawNativeAdapterError::WorkgroupSpecializationSpecIdsMismatch {
                            expected: self.workgroup_size_spec_ids,
                            actual: values.spec_ids,
                        },
                    );
                }
                validate_workgroup_dimensions(values.values)
            }
            (Some(_), Some(_), _) => Err(SpirvRawNativeAdapterError::ConflictingWorkgroupMetadata),
            (None, None, _) => Err(SpirvRawNativeAdapterError::MissingWorkgroupMetadata),
        }
    }

    /// Resolves `LocalSizeId` dimensions from an external `SpecId`→value map.
    ///
    /// The map may contain specialization values for other shader constants;
    /// only the three reflected local-size `SpecId`s are consumed. A complete
    /// three-way `SpecId` reflection is required so the mapping cannot guess
    /// which external value belongs to which `LocalSizeId` operand. This
    /// materializes direct scalar values only; it does not evaluate
    /// `OpSpecConstantOp` expressions or rewrite the SPIR-V module.
    pub fn resolve_workgroup_size_from_spec_map(
        &self,
        spec_values: &BTreeMap<u32, u32>,
    ) -> Result<[u32; 3], SpirvRawNativeAdapterError> {
        if self.workgroup_size.is_some() && self.workgroup_size_ids.is_some() {
            return Err(SpirvRawNativeAdapterError::ConflictingWorkgroupMetadata);
        }
        if self.workgroup_size.is_some() {
            if spec_values.is_empty() {
                return self.resolve_workgroup_size(None);
            }
            return Err(SpirvRawNativeAdapterError::UnexpectedWorkgroupSpecialization);
        }
        let Some(ids) = self.workgroup_size_ids else {
            return self.resolve_workgroup_size(None);
        };
        let Some(spec_ids) = self.workgroup_size_spec_ids else {
            return Err(SpirvRawNativeAdapterError::MissingWorkgroupSpecializationSpecIds);
        };
        let mut values = [0_u32; 3];
        for (index, spec_id) in spec_ids.iter().copied().enumerate() {
            let Some(value) = spec_values.get(&spec_id).copied() else {
                return Err(
                    SpirvRawNativeAdapterError::MissingWorkgroupSpecializationValue { spec_id },
                );
            };
            values[index] = value;
        }
        self.resolve_workgroup_size(Some(&SpirvRawWorkgroupSpecialization {
            ids,
            spec_ids: Some(spec_ids),
            values,
        }))
    }

    /// Resolves workgroup dimensions from the exact raw SPIR-V module behind
    /// this plan. The word count, stable hash and selected entry are checked
    /// before the bounded `OpSpecConstantOp` evaluator runs.
    pub fn resolve_workgroup_size_from_spirv_words(
        &self,
        words: &[u32],
        entry_name: &str,
        spec_values: &BTreeMap<u32, u32>,
    ) -> Result<[u32; 3], SpirvRawNativeAdapterError> {
        if entry_name != self.entry_name {
            return Err(SpirvRawNativeAdapterError::SpirvEntryMismatch {
                expected: self.entry_name.clone(),
                actual: entry_name.to_owned(),
            });
        }
        if words.len() != self.word_count {
            return Err(SpirvRawNativeAdapterError::SpirvWordCountMismatch {
                expected: self.word_count,
                actual: words.len(),
            });
        }
        let word_hash = stable_spirv_word_hash(words);
        if word_hash != self.word_hash {
            return Err(SpirvRawNativeAdapterError::SpirvWordHashMismatch {
                expected: self.word_hash,
                actual: word_hash,
            });
        }
        if self.workgroup_size.is_none()
            && self.workgroup_size_ids.is_none()
            && !spec_values.is_empty()
        {
            return Err(SpirvRawNativeAdapterError::UnexpectedWorkgroupSpecialization);
        }
        if self.workgroup_size_ids.is_some()
            && self.workgroup_size_spec_ids.is_none()
            && !spec_values.is_empty()
        {
            return Err(SpirvRawNativeAdapterError::MissingWorkgroupSpecializationSpecIds);
        }
        let dimensions = resolve_spirv_source_workgroup_size(words, entry_name, spec_values)
            .map_err(SpirvRawNativeAdapterError::WorkgroupSpecializationEvaluation)?;
        if let Some(expected) = self.workgroup_size {
            if !spec_values.is_empty() {
                return Err(SpirvRawNativeAdapterError::UnexpectedWorkgroupSpecialization);
            }
            if dimensions != expected {
                return Err(SpirvRawNativeAdapterError::WorkgroupDimensionsMismatch {
                    expected,
                    actual: dimensions,
                });
            }
        }
        Ok(dimensions)
    }
}

/// Explicit values for one raw `OpExecutionModeId LocalSizeId` contract.
///
/// The IDs must be copied from [`SpirvRawNativeAdapterPlan::workgroup_size_ids`]
/// in the same x/y/z order. When the module reflects `SpecId` decorations,
/// `spec_ids` must carry the matching external IDs in x/y/z order as well.
/// Values are checked for non-zero dimensions before a future native encoder
/// can consume them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpirvRawWorkgroupSpecialization {
    /// SPIR-V result IDs being specialized, in x/y/z order.
    pub ids: [u32; 3],
    /// External `SpecId` decorations for the specialized IDs, when reflected.
    pub spec_ids: Option<[u32; 3]>,
    /// Resolved unsigned local dimensions, in x/y/z order.
    pub values: [u32; 3],
}

fn validate_workgroup_dimensions(
    dimensions: [u32; 3],
) -> Result<[u32; 3], SpirvRawNativeAdapterError> {
    if let Some((index, value)) = dimensions
        .iter()
        .copied()
        .enumerate()
        .find(|(_, value)| *value == 0)
    {
        return Err(SpirvRawNativeAdapterError::ZeroWorkgroupDimension { index, value });
    }
    Ok(dimensions)
}

/// Why a raw module cannot cross the common native-adapter capability gate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpirvRawNativeAdapterError {
    /// Only SPIR-V compute entry points are accepted by this adapter boundary.
    UnsupportedExecutionModel { actual: u32 },
    /// Resource bindings must be dense and declaration ordered.
    NonCanonicalBinding { expected: u32, actual: u32 },
    /// A descriptor variable did not retain its storage class.
    MissingStorageClass { binding: u32 },
    /// Storage class is not in the portable storage-buffer/uniform subset.
    UnsupportedStorageClass { binding: u32, storage_class: u32 },
    /// The first native adapter revision accepts descriptor set zero only.
    UnsupportedDescriptorSet { binding: u32, descriptor_set: u32 },
    /// Type reflection could not prove a scalar/vector element.
    MissingElementType { binding: u32 },
    /// Type reflection could not prove a byte stride.
    MissingElementStride { binding: u32 },
    /// Access reflection could not prove a conservative policy.
    MissingAccess { binding: u32 },
    /// A raw contract had no literal or specialization workgroup metadata.
    MissingWorkgroupMetadata,
    /// LocalSizeId metadata exists but no explicit values were supplied.
    MissingWorkgroupSpecialization,
    /// Literal LocalSize metadata was paired with specialization values.
    UnexpectedWorkgroupSpecialization,
    /// Both literal and LocalSizeId metadata were present.
    ConflictingWorkgroupMetadata,
    /// Specialization IDs do not match the reflected LocalSizeId operands.
    WorkgroupSpecializationIdsMismatch {
        expected: [u32; 3],
        actual: [u32; 3],
    },
    /// External specialization IDs do not match the reflected `SpecId` map.
    WorkgroupSpecializationSpecIdsMismatch {
        expected: Option<[u32; 3]>,
        actual: Option<[u32; 3]>,
    },
    /// LocalSizeId has no complete external SpecId map to use for lookup.
    MissingWorkgroupSpecializationSpecIds,
    /// A reflected external SpecId has no caller-supplied value.
    MissingWorkgroupSpecializationValue { spec_id: u32 },
    /// The caller supplied a different SPIR-V word count than the plan.
    SpirvWordCountMismatch { expected: usize, actual: usize },
    /// The caller supplied words with a different stable identity hash.
    SpirvWordHashMismatch { expected: u64, actual: u64 },
    /// The caller selected a different entry point than the plan.
    SpirvEntryMismatch { expected: String, actual: String },
    /// The raw evaluator rejected the selected LocalSizeId expression graph.
    WorkgroupSpecializationEvaluation(SpirvRawWorkgroupEvaluationError),
    /// Evaluated dimensions differ from literal plan metadata.
    WorkgroupDimensionsMismatch {
        expected: [u32; 3],
        actual: [u32; 3],
    },
    /// A resolved local dimension is zero and cannot be dispatched.
    ZeroWorkgroupDimension { index: usize, value: u32 },
}

impl fmt::Display for SpirvRawNativeAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedExecutionModel { actual } => {
                write!(
                    formatter,
                    "raw module execution model {actual} is not compute"
                )
            }
            Self::NonCanonicalBinding { expected, actual } => write!(
                formatter,
                "raw native adapter expected binding {expected}, got {actual}"
            ),
            Self::MissingStorageClass { binding } => {
                write!(
                    formatter,
                    "raw resource binding {binding} has no storage class"
                )
            }
            Self::UnsupportedStorageClass {
                binding,
                storage_class,
            } => write!(
                formatter,
                "raw resource binding {binding} has unsupported storage class {storage_class}"
            ),
            Self::UnsupportedDescriptorSet {
                binding,
                descriptor_set,
            } => write!(
                formatter,
                "raw resource binding {binding} has unsupported descriptor set {descriptor_set}"
            ),
            Self::MissingElementType { binding } => {
                write!(
                    formatter,
                    "raw resource binding {binding} has unknown element type"
                )
            }
            Self::MissingElementStride { binding } => {
                write!(
                    formatter,
                    "raw resource binding {binding} has unknown byte stride"
                )
            }
            Self::MissingAccess { binding } => {
                write!(
                    formatter,
                    "raw resource binding {binding} has unknown access policy"
                )
            }
            Self::MissingWorkgroupMetadata => {
                formatter.write_str("raw module has no LocalSize or LocalSizeId metadata")
            }
            Self::MissingWorkgroupSpecialization => {
                formatter.write_str("raw LocalSizeId requires explicit specialization values")
            }
            Self::UnexpectedWorkgroupSpecialization => formatter
                .write_str("raw literal LocalSize cannot be paired with specialization values"),
            Self::ConflictingWorkgroupMetadata => {
                formatter.write_str("raw module has conflicting LocalSize and LocalSizeId metadata")
            }
            Self::WorkgroupSpecializationIdsMismatch { expected, actual } => write!(
                formatter,
                "raw LocalSizeId specialization IDs mismatch: expected {expected:?}, got {actual:?}"
            ),
            Self::WorkgroupSpecializationSpecIdsMismatch { expected, actual } => write!(
                formatter,
                "raw LocalSizeId SpecId map mismatch: expected {expected:?}, got {actual:?}"
            ),
            Self::MissingWorkgroupSpecializationSpecIds => formatter.write_str(
                "raw LocalSizeId has no complete SpecId map for external specialization lookup",
            ),
            Self::MissingWorkgroupSpecializationValue { spec_id } => write!(
                formatter,
                "raw LocalSizeId specialization value is missing for SpecId {spec_id}"
            ),
            Self::SpirvWordCountMismatch { expected, actual } => write!(
                formatter,
                "raw LocalSizeId word count mismatch: expected {expected}, got {actual}"
            ),
            Self::SpirvWordHashMismatch { expected, actual } => write!(
                formatter,
                "raw LocalSizeId word hash mismatch: expected {expected}, got {actual}"
            ),
            Self::SpirvEntryMismatch { expected, actual } => write!(
                formatter,
                "raw LocalSizeId entry mismatch: expected {expected}, got {actual}"
            ),
            Self::WorkgroupSpecializationEvaluation(error) => {
                write!(
                    formatter,
                    "raw LocalSizeId specialization evaluation failed: {error}"
                )
            }
            Self::WorkgroupDimensionsMismatch { expected, actual } => write!(
                formatter,
                "raw workgroup dimensions mismatch: expected {expected:?}, got {actual:?}"
            ),
            Self::ZeroWorkgroupDimension { index, value } => write!(
                formatter,
                "raw workgroup dimension {index} has invalid zero value {value}"
            ),
        }
    }
}

impl Error for SpirvRawNativeAdapterError {}

/// Validates the common raw capability boundary before a native adapter.
pub fn validate_spirv_raw_native_adapter(
    contract: &SpirvRawModuleContract,
) -> Result<SpirvRawNativeAdapterPlan, SpirvRawNativeAdapterError> {
    if contract.execution_model != 5 {
        return Err(SpirvRawNativeAdapterError::UnsupportedExecutionModel {
            actual: contract.execution_model,
        });
    }
    for (index, resource) in contract.resources.iter().enumerate() {
        let expected = u32::try_from(index).expect("raw resource count fits u32");
        if resource.binding != expected {
            return Err(SpirvRawNativeAdapterError::NonCanonicalBinding {
                expected,
                actual: resource.binding,
            });
        }
        let Some(storage_class) = resource.storage_class else {
            return Err(SpirvRawNativeAdapterError::MissingStorageClass {
                binding: resource.binding,
            });
        };
        if !matches!(storage_class, 2 | 12) {
            return Err(SpirvRawNativeAdapterError::UnsupportedStorageClass {
                binding: resource.binding,
                storage_class,
            });
        }
        if resource.descriptor_set != 0 {
            return Err(SpirvRawNativeAdapterError::UnsupportedDescriptorSet {
                binding: resource.binding,
                descriptor_set: resource.descriptor_set,
            });
        }
        if resource.element_type.is_none() {
            return Err(SpirvRawNativeAdapterError::MissingElementType {
                binding: resource.binding,
            });
        }
        if resource.element_stride.is_none() {
            return Err(SpirvRawNativeAdapterError::MissingElementStride {
                binding: resource.binding,
            });
        }
        if resource.access.is_none() {
            return Err(SpirvRawNativeAdapterError::MissingAccess {
                binding: resource.binding,
            });
        }
    }
    Ok(SpirvRawNativeAdapterPlan {
        entry_name: contract.entry_name.clone(),
        execution_model: contract.execution_model,
        workgroup_size: contract.workgroup_size,
        workgroup_size_ids: contract.workgroup_size_ids,
        workgroup_size_spec_ids: contract.workgroup_size_spec_ids,
        resources: contract.resources.clone(),
        word_count: contract.word_count,
        word_hash: contract.word_hash,
    })
}

/// Metadata-only dispatch route for a raw SPIR-V module.
///
/// The route combines the bounded specialization/integrity bridge with the
/// existing backend capability planner and checked dispatch geometry. It does
/// not translate shader bytes, allocate descriptors, create a device, or
/// submit native work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpirvRawDispatchPlan {
    /// Backend capability route selected before any native API call.
    pub backend: BackendPlan,
    /// Backend-specific resource view projection for the raw module.
    pub adapter: SpirvRawBackendAdapterPlan,
    /// Resolved local workgroup dimensions in x/y/z order.
    pub workgroup_size: [u32; 3],
    /// Number of local workgroups in x/y/z order.
    pub workgroups: [u32; 3],
    /// Checked total invocation count for this dispatch.
    pub invocation_count: u64,
}

/// Common source-backed execution preparation for translated backends.
///
/// This is the shared hand-off consumed by DX12 and Metal adapters after the
/// raw SPIR-V integrity/capability gate. It deliberately contains no native
/// device, queue, shader module or descriptor handle; those lifetimes remain
/// owned by the backend-specific executor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpirvSourceExecutionPlan {
    /// Raw module/backend/geometry plan that was validated first.
    pub raw_dispatch: SpirvRawDispatchPlan,
    /// Source report produced through the canonical HLSL/MSL route.
    pub source: SpirvSourceTranslationReport,
}

/// Why a source-backed execution preparation was rejected before native work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpirvSourceExecutionPlanError {
    /// Vulkan uses native SPIR-V and therefore has no source translation
    /// report in this API.
    NativeSpirvTransport { backend: GpuBackend },
    /// The raw module, capability route or dispatch geometry failed.
    Raw(SpirvRawDispatchPlanError),
    /// The external source translator failed or was unavailable.
    Translation(SpirvSourceTranslationError),
    /// The translated report no longer describes the exact raw module/route.
    TranslationIdentityMismatch,
}

impl fmt::Display for SpirvSourceExecutionPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NativeSpirvTransport { backend } => write!(
                formatter,
                "backend {backend:?} uses native SPIR-V transport, not a source report"
            ),
            Self::Raw(error) => write!(formatter, "raw source execution plan rejected: {error}"),
            Self::Translation(error) => {
                write!(formatter, "source translation rejected: {error}")
            }
            Self::TranslationIdentityMismatch => formatter.write_str(
                "translated source report does not match the exact raw SPIR-V execution plan",
            ),
        }
    }
}

impl Error for SpirvSourceExecutionPlanError {}

/// Caller-owned inputs for [`plan_spirv_source_execution`].
#[derive(Clone, Copy, Debug)]
pub struct SpirvSourceExecutionRequest<'a> {
    /// Backend route to prepare.
    pub backend: GpuBackend,
    /// Host capability snapshot used before any native lifecycle.
    pub probe: BackendProbe,
    /// Portable kernel requirements.
    pub request: ArtifactDispatchRequest,
    /// Exact SPIR-V words handed to both reflection and translation.
    pub words: &'a [u32],
    /// Entry point selected in the module.
    pub entry_name: &'a str,
    /// External SPIRV-Cross-compatible translator executable.
    pub tool: &'a Path,
    /// Caller-supplied `SpecId` values for `LocalSizeId` dimensions.
    pub spec_values: &'a BTreeMap<u32, u32>,
    /// Non-zero dispatch workgroup grid.
    pub workgroups: [u32; 3],
}

/// Why a raw SPIR-V dispatch route was rejected before native work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpirvRawDispatchPlanError {
    /// The shared raw module contract failed.
    Source(SpirvSourceTranslationError),
    /// The common raw native capability gate failed.
    Native(SpirvRawNativeAdapterError),
    /// The selected backend capability route failed.
    Backend(BackendPlanError),
    /// The requested workgroup grid or invocation count overflowed/rejected.
    Geometry(DispatchGeometryError),
}

impl fmt::Display for SpirvRawDispatchPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(error) => {
                write!(formatter, "raw SPIR-V source contract rejected: {error}")
            }
            Self::Native(error) => write!(formatter, "raw native adapter rejected: {error}"),
            Self::Backend(error) => write!(formatter, "raw backend route rejected: {error}"),
            Self::Geometry(error) => write!(formatter, "raw dispatch geometry rejected: {error}"),
        }
    }
}

impl Error for SpirvRawDispatchPlanError {}

/// Builds a capability-only dispatch plan for an exact raw SPIR-V module.
///
/// The selected entry, word count and stable word hash are checked before the
/// bounded `LocalSizeId` evaluator is used. `spec_values` therefore cannot be
/// applied to a different module, and the resulting plan remains metadata
/// only until a backend-specific native adapter is implemented.
pub fn plan_spirv_raw_dispatch(
    backend: GpuBackend,
    probe: BackendProbe,
    request: ArtifactDispatchRequest,
    words: &[u32],
    entry_name: &str,
    spec_values: &BTreeMap<u32, u32>,
    workgroups: [u32; 3],
) -> Result<SpirvRawDispatchPlan, SpirvRawDispatchPlanError> {
    let contract = inspect_spirv_source_module(words, entry_name)
        .map_err(SpirvRawDispatchPlanError::Source)?;
    let native =
        validate_spirv_raw_native_adapter(&contract).map_err(SpirvRawDispatchPlanError::Native)?;
    let workgroup_size = native
        .resolve_workgroup_size_from_spirv_words(words, entry_name, spec_values)
        .map_err(SpirvRawDispatchPlanError::Native)?;
    let product = workgroup_size
        .into_iter()
        .try_fold(1_u64, |product, dimension| {
            product.checked_mul(u64::from(dimension))
        })
        .unwrap_or(u64::MAX);
    if product > u64::from(probe.max_workgroup_size) {
        return Err(SpirvRawDispatchPlanError::Backend(
            BackendPlanError::WorkgroupSizeUnsupported,
        ));
    }
    let backend_plan = plan_backend(
        backend,
        probe,
        BackendRequest {
            fp: request.fp,
            workgroup_size: workgroup_size[0],
            require_bounded_global_u32_array: request.require_bounded_global_u32_array,
            require_async_completion: request.require_async_completion,
        },
    )
    .map_err(SpirvRawDispatchPlanError::Backend)?;
    let geometry =
        DispatchGeometry::new(workgroups).map_err(SpirvRawDispatchPlanError::Geometry)?;
    let invocation_count = geometry
        .invocation_count(workgroup_size)
        .map_err(SpirvRawDispatchPlanError::Geometry)?;
    Ok(SpirvRawDispatchPlan {
        backend: backend_plan,
        adapter: native.project_backend(backend),
        workgroup_size,
        workgroups,
        invocation_count,
    })
}

/// Builds the shared DX12/Metal source-backed execution hand-off.
///
/// The raw words are inspected and planned before the external translator is
/// invoked. The resulting report must preserve the same entry, execution
/// metadata, resources, word count/hash and resolved workgroup dimensions.
/// This function does not create a device or submit a dispatch.
pub fn plan_spirv_source_execution(
    input: SpirvSourceExecutionRequest<'_>,
) -> Result<SpirvSourceExecutionPlan, SpirvSourceExecutionPlanError> {
    if ShaderTranslationRoute::for_backend(input.backend)
        .source_backend
        .is_none()
    {
        return Err(SpirvSourceExecutionPlanError::NativeSpirvTransport {
            backend: input.backend,
        });
    }
    let raw_dispatch = plan_spirv_raw_dispatch(
        input.backend,
        input.probe,
        input.request,
        input.words,
        input.entry_name,
        input.spec_values,
        input.workgroups,
    )
    .map_err(SpirvSourceExecutionPlanError::Raw)?;
    let source = translate_spirv_source_report_for_backend(
        input.words,
        input.entry_name,
        input.tool,
        input.backend,
    )
    .map_err(SpirvSourceExecutionPlanError::Translation)?;
    let identity = &source.identity;
    let adapter = &raw_dispatch.adapter;
    let literal_workgroup_matches = identity
        .workgroup_size
        .is_none_or(|dimensions| dimensions == raw_dispatch.workgroup_size);
    let resources_match = identity.resources.len() == adapter.resources.len()
        && identity
            .resources
            .iter()
            .zip(&adapter.resources)
            .all(|(raw, projected)| {
                raw.binding == projected.binding
                    && raw.descriptor_set == projected.descriptor_set
                    && raw.element_type == Some(projected.element_type)
                    && raw.element_stride == Some(projected.element_stride)
                    && raw.access == Some(projected.access)
            });
    if identity.entry_name != adapter.entry_name
        || identity.execution_model != adapter.execution_model
        || !literal_workgroup_matches
        || identity.workgroup_size_ids != adapter.workgroup_size_ids
        || identity.workgroup_size_spec_ids != adapter.workgroup_size_spec_ids
        || !resources_match
        || identity.word_count != adapter.word_count
        || identity.word_hash != adapter.word_hash
    {
        return Err(SpirvSourceExecutionPlanError::TranslationIdentityMismatch);
    }
    Ok(SpirvSourceExecutionPlan {
        raw_dispatch,
        source,
    })
}

/// Metadata-only differential report for two or more source-backed backend
/// plans. It proves that the same raw module and dispatch geometry reached
/// each route; it does not prove native compilation, completion or readback.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpirvSourceExecutionParityReport {
    /// Shared selected entry point.
    pub entry_name: String,
    /// Shared SPIR-V execution model.
    pub execution_model: u32,
    /// Resolved local workgroup dimensions.
    pub workgroup_size: [u32; 3],
    /// Shared dispatch workgroup grid.
    pub workgroups: [u32; 3],
    /// Shared checked invocation count.
    pub invocation_count: u64,
    /// Stable raw SPIR-V word count/hash shared by every plan.
    pub word_count: usize,
    pub word_hash: u64,
    /// Portable raw resource metadata shared by every plan.
    pub resources: Vec<SpirvRawResourceBinding>,
    /// Native backend routes in caller order.
    pub backends: Vec<GpuBackend>,
    /// Shader transports in caller order.
    pub transports: Vec<ShaderTransport>,
    /// Source languages in caller order.
    pub source_backends: Vec<ArtifactSourceBackend>,
    /// Source hashes in caller order; source text is allowed to differ.
    pub source_hashes: Vec<u64>,
}

/// Why source-backed execution plans cannot form one differential report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpirvSourceExecutionParityError {
    /// At least two distinct backend plans are required.
    InsufficientPlans { actual: usize },
    /// Plans do not refer to the same raw module identity.
    ModuleIdentityMismatch,
    /// Plans use different resolved geometry or invocation counts.
    DispatchGeometryMismatch,
    /// The same native backend was supplied more than once.
    DuplicateBackend(GpuBackend),
    /// The same source language route was supplied more than once.
    DuplicateSourceBackend(ArtifactSourceBackend),
    /// A plan's transport does not match its canonical backend route.
    BackendRouteMismatch,
}

impl fmt::Display for SpirvSourceExecutionParityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InsufficientPlans { actual } => write!(
                formatter,
                "source execution parity requires at least two plans, got {actual}"
            ),
            Self::ModuleIdentityMismatch => {
                formatter.write_str("source execution plans have different module identities")
            }
            Self::DispatchGeometryMismatch => formatter
                .write_str("source execution plans have different dispatch geometry or count"),
            Self::DuplicateBackend(backend) => {
                write!(
                    formatter,
                    "source execution parity repeats backend {backend:?}"
                )
            }
            Self::DuplicateSourceBackend(backend) => write!(
                formatter,
                "source execution parity repeats source backend {backend:?}"
            ),
            Self::BackendRouteMismatch => {
                formatter.write_str("source execution plan transport does not match backend route")
            }
        }
    }
}

impl Error for SpirvSourceExecutionParityError {}

/// Compares source-backed execution plans without claiming native execution.
pub fn compare_spirv_source_execution_plans(
    plans: &[SpirvSourceExecutionPlan],
) -> Result<SpirvSourceExecutionParityReport, SpirvSourceExecutionParityError> {
    if plans.len() < 2 {
        return Err(SpirvSourceExecutionParityError::InsufficientPlans {
            actual: plans.len(),
        });
    }
    let first = &plans[0];
    let first_identity = &first.source.identity;
    let first_raw = &first.raw_dispatch;
    let mut backends = Vec::with_capacity(plans.len());
    let mut transports = Vec::with_capacity(plans.len());
    let mut source_backends = Vec::with_capacity(plans.len());
    let mut source_hashes = Vec::with_capacity(plans.len());
    for plan in plans {
        let backend = plan.raw_dispatch.backend.backend;
        let expected_route = ShaderTranslationRoute::for_backend(backend);
        if expected_route.transport != plan.raw_dispatch.backend.shader_transport
            || expected_route.source_backend != Some(plan.source.identity.backend)
        {
            return Err(SpirvSourceExecutionParityError::BackendRouteMismatch);
        }
        let identity = &plan.source.identity;
        if identity.entry_name != first_identity.entry_name
            || identity.execution_model != first_identity.execution_model
            || identity.workgroup_size != first_identity.workgroup_size
            || identity.workgroup_size_ids != first_identity.workgroup_size_ids
            || identity.workgroup_size_spec_ids != first_identity.workgroup_size_spec_ids
            || identity.resources != first_identity.resources
            || identity.word_count != first_identity.word_count
            || identity.word_hash != first_identity.word_hash
        {
            return Err(SpirvSourceExecutionParityError::ModuleIdentityMismatch);
        }
        if plan.raw_dispatch.workgroup_size != first_raw.workgroup_size
            || plan.raw_dispatch.workgroups != first_raw.workgroups
            || plan.raw_dispatch.invocation_count != first_raw.invocation_count
        {
            return Err(SpirvSourceExecutionParityError::DispatchGeometryMismatch);
        }
        if backends.contains(&backend) {
            return Err(SpirvSourceExecutionParityError::DuplicateBackend(backend));
        }
        if source_backends.contains(&identity.backend) {
            return Err(SpirvSourceExecutionParityError::DuplicateSourceBackend(
                identity.backend,
            ));
        }
        backends.push(backend);
        transports.push(plan.raw_dispatch.backend.shader_transport);
        source_backends.push(identity.backend);
        source_hashes.push(plan.source.source_hash);
    }
    Ok(SpirvSourceExecutionParityReport {
        entry_name: first_identity.entry_name.clone(),
        execution_model: first_identity.execution_model,
        workgroup_size: first_raw.workgroup_size,
        workgroups: first_raw.workgroups,
        invocation_count: first_raw.invocation_count,
        word_count: first_identity.word_count,
        word_hash: first_identity.word_hash,
        resources: first_identity.resources.clone(),
        backends,
        transports,
        source_backends,
        source_hashes,
    })
}

/// Audit report for a raw SPIR-V→source translation.
///
/// Unlike [`ArtifactSourceTranslationReport`], this report intentionally does
/// not invent resource metadata. Callers remain responsible for structural
/// validation and backend-specific resource reflection before native use.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpirvSourceTranslationReport {
    /// Identity of the raw module and selected source backend.
    pub identity: SpirvSourceTranslationIdentity,
    /// UTF-8 source returned by the external translator.
    pub source: String,
    /// UTF-8 byte length of `source`.
    pub source_byte_count: usize,
    /// Stable FNV-1a hash of the UTF-8 source bytes.
    pub source_hash: u64,
}

/// Failure while creating an audit report for a raw source translation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpirvSourceTranslationError {
    /// The caller supplied no words or an unsafe entry identifier.
    InvalidInput(&'static str),
    /// The raw SPIR-V stream is structurally invalid or does not contain the
    /// selected entry point.
    InvalidSpirv(&'static str),
    /// The selected entry point is not present in the raw module.
    EntryPointNotFound(String),
    /// The external SPIRV-Cross process failed or was unavailable.
    Tool(SpirvCrossError),
    /// The external process returned no source text.
    EmptySource,
}

impl fmt::Display for SpirvSourceTranslationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(reason) => {
                write!(formatter, "invalid source translation input: {reason}")
            }
            Self::InvalidSpirv(reason) => write!(formatter, "invalid raw SPIR-V module: {reason}"),
            Self::EntryPointNotFound(entry_name) => {
                write!(
                    formatter,
                    "raw SPIR-V entry point `{entry_name}` was not found"
                )
            }
            Self::Tool(error) => write!(formatter, "source translation tool failed: {error}"),
            Self::EmptySource => formatter.write_str("source translation returned empty output"),
        }
    }
}

impl Error for SpirvSourceTranslationError {}

/// Snapshots the shared raw SPIR-V module/resource contract before any
/// external toolchain or native API work.
pub fn inspect_spirv_source_module(
    words: &[u32],
    entry_name: &str,
) -> Result<SpirvRawModuleContract, SpirvSourceTranslationError> {
    if words.is_empty() {
        return Err(SpirvSourceTranslationError::InvalidInput(
            "empty SPIR-V word stream",
        ));
    }
    if entry_name.is_empty() || entry_name.contains('\0') {
        return Err(SpirvSourceTranslationError::InvalidInput(
            "entry name must be non-empty and NUL-free",
        ));
    }
    let metadata = validate_raw_spirv_entry(words, entry_name)?;
    Ok(SpirvRawModuleContract {
        entry_name: entry_name.to_owned(),
        execution_model: metadata.execution_model,
        workgroup_size: metadata.workgroup_size,
        workgroup_size_ids: metadata.workgroup_size_ids,
        workgroup_size_spec_ids: metadata.workgroup_size_spec_ids,
        resources: metadata.resources,
        word_count: words.len(),
        word_hash: stable_spirv_word_hash(words),
    })
}

/// Runs the shared raw SPIR-V→source process boundary and returns an audit
/// report after the common module contract has passed.
pub fn translate_spirv_source_report(
    words: &[u32],
    entry_name: &str,
    tool: &Path,
    backend: ArtifactSourceBackend,
) -> Result<SpirvSourceTranslationReport, SpirvSourceTranslationError> {
    let contract = inspect_spirv_source_module(words, entry_name)?;
    let source = translate_spirv_with_spirv_cross(tool, words, entry_name, backend)
        .map_err(SpirvSourceTranslationError::Tool)?;
    if source.trim().is_empty() {
        return Err(SpirvSourceTranslationError::EmptySource);
    }
    Ok(SpirvSourceTranslationReport {
        identity: SpirvSourceTranslationIdentity {
            backend,
            entry_name: contract.entry_name,
            execution_model: contract.execution_model,
            workgroup_size: contract.workgroup_size,
            workgroup_size_ids: contract.workgroup_size_ids,
            workgroup_size_spec_ids: contract.workgroup_size_spec_ids,
            resources: contract.resources,
            word_count: contract.word_count,
            word_hash: contract.word_hash,
        },
        source_byte_count: source.len(),
        source_hash: stable_source_hash(&source),
        source,
    })
}

/// Translates a raw SPIR-V module using the canonical source route for a
/// backend. Vulkan is intentionally rejected here because its route submits
/// native SPIR-V; DX12 selects HLSL and Metal selects MSL. The returned report
/// remains source/identity evidence and does not create a native device or
/// execute the module.
pub fn translate_spirv_source_report_for_backend(
    words: &[u32],
    entry_name: &str,
    tool: &Path,
    backend: GpuBackend,
) -> Result<SpirvSourceTranslationReport, SpirvSourceTranslationError> {
    let Some(source_backend) = ShaderTranslationRoute::for_backend(backend).source_backend else {
        return Err(SpirvSourceTranslationError::InvalidInput(
            "backend uses native SPIR-V transport",
        ));
    };
    translate_spirv_source_report(words, entry_name, tool, source_backend)
}

/// Failure while validating a translated source report against its exact raw
/// SPIR-V words before a backend native lifecycle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpirvSourceReportWordsError {
    /// The selected backend submits native SPIR-V and cannot consume a source
    /// report through this helper.
    NativeSpirvTransport { backend: GpuBackend },
    /// The report was generated for a different source language route.
    SourceBackendMismatch {
        expected: ArtifactSourceBackend,
        actual: ArtifactSourceBackend,
    },
    /// The caller supplied a different number of words than the report.
    WordCountMismatch { expected: usize, actual: usize },
    /// The caller supplied words with a different stable identity hash.
    WordHashMismatch { expected: u64, actual: u64 },
    /// The raw module cannot be inspected at this common boundary.
    Source(SpirvSourceTranslationError),
    /// Reflected raw metadata differs from the report identity.
    IdentityMismatch,
    /// The raw module fails the shared native capability gate.
    Native(SpirvRawNativeAdapterError),
    /// The bounded LocalSize/SpecId resolution fails after the native plan.
    Specialization(SpirvRawNativeAdapterError),
}

impl fmt::Display for SpirvSourceReportWordsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NativeSpirvTransport { backend } => write!(
                formatter,
                "backend {backend:?} uses native SPIR-V transport, not a source report"
            ),
            Self::SourceBackendMismatch { expected, actual } => write!(
                formatter,
                "source report backend mismatch: expected {expected:?}, got {actual:?}"
            ),
            Self::WordCountMismatch { expected, actual } => write!(
                formatter,
                "source report word count mismatch: expected {expected}, got {actual}"
            ),
            Self::WordHashMismatch { expected, actual } => write!(
                formatter,
                "source report word hash mismatch: expected {expected:#x}, got {actual:#x}"
            ),
            Self::Source(error) => write!(formatter, "source report raw module invalid: {error}"),
            Self::IdentityMismatch => {
                formatter.write_str("source report identity differs from raw SPIR-V metadata")
            }
            Self::Native(error) => write!(formatter, "source report native plan rejected: {error}"),
            Self::Specialization(error) => {
                write!(formatter, "source report specialization rejected: {error}")
            }
        }
    }
}

impl Error for SpirvSourceReportWordsError {}

/// Validates a source report against the exact SPIR-V words that produced it.
///
/// DX12 and Metal call this before creating a device/queue or compiling a
/// native shader. It is intentionally backend-neutral and returns the common
/// raw native plan; resource-view and payload checks remain backend-specific.
pub fn validate_spirv_source_report_words(
    report: &SpirvSourceTranslationReport,
    words: &[u32],
    backend: GpuBackend,
    spec_values: &BTreeMap<u32, u32>,
) -> Result<SpirvRawNativeAdapterPlan, SpirvSourceReportWordsError> {
    let Some(expected_backend) = ShaderTranslationRoute::for_backend(backend).source_backend else {
        return Err(SpirvSourceReportWordsError::NativeSpirvTransport { backend });
    };
    if report.identity.backend != expected_backend {
        return Err(SpirvSourceReportWordsError::SourceBackendMismatch {
            expected: expected_backend,
            actual: report.identity.backend,
        });
    }
    if words.len() != report.identity.word_count {
        return Err(SpirvSourceReportWordsError::WordCountMismatch {
            expected: report.identity.word_count,
            actual: words.len(),
        });
    }
    let actual_hash = stable_spirv_word_hash(words);
    if actual_hash != report.identity.word_hash {
        return Err(SpirvSourceReportWordsError::WordHashMismatch {
            expected: report.identity.word_hash,
            actual: actual_hash,
        });
    }
    let contract = inspect_spirv_source_module(words, &report.identity.entry_name)
        .map_err(SpirvSourceReportWordsError::Source)?;
    if contract.execution_model != report.identity.execution_model
        || contract.workgroup_size != report.identity.workgroup_size
        || contract.workgroup_size_ids != report.identity.workgroup_size_ids
        || contract.workgroup_size_spec_ids != report.identity.workgroup_size_spec_ids
        || contract.resources != report.identity.resources
    {
        return Err(SpirvSourceReportWordsError::IdentityMismatch);
    }
    let native_plan = validate_spirv_raw_native_adapter(&contract)
        .map_err(SpirvSourceReportWordsError::Native)?;
    native_plan
        .resolve_workgroup_size_from_spirv_words(words, &report.identity.entry_name, spec_values)
        .map_err(SpirvSourceReportWordsError::Specialization)?;
    Ok(native_plan)
}

/// Returns the execution model of a selected raw SPIR-V entry point after a
/// backend-neutral structural pass. This deliberately validates only the
/// module envelope and `OpEntryPoint` identity; full instruction semantics and
/// resource reflection remain responsibilities of SPIRV-Cross and consumers.
#[derive(Clone, Debug, Eq, PartialEq)]
enum RawSpirvType {
    Integer { bits: u16, signed: bool },
    Float { bits: u16 },
    Vector { component: u32, lanes: u16 },
    Pointer { pointee: u32 },
    RuntimeArray { element: u32 },
    Struct { members: Vec<u32> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RawSpirvMetadata {
    execution_model: u32,
    workgroup_size: Option<[u32; 3]>,
    workgroup_size_ids: Option<[u32; 3]>,
    workgroup_size_spec_ids: Option<[u32; 3]>,
    resources: Vec<SpirvRawResourceBinding>,
}

#[derive(Default)]
struct RawSpirvDecorations {
    binding: Option<u32>,
    descriptor_set: Option<u32>,
    spec_id: Option<u32>,
    non_writable: bool,
    non_readable: bool,
}

fn validate_raw_spirv_entry(
    words: &[u32],
    entry_name: &str,
) -> Result<RawSpirvMetadata, SpirvSourceTranslationError> {
    if words.len() < 5 {
        return Err(SpirvSourceTranslationError::InvalidSpirv(
            "module header is incomplete",
        ));
    }
    if words[0] != 0x0723_0203 {
        return Err(SpirvSourceTranslationError::InvalidSpirv(
            "invalid SPIR-V magic",
        ));
    }
    if words[3] == 0 {
        return Err(SpirvSourceTranslationError::InvalidSpirv(
            "module id bound must be non-zero",
        ));
    }
    let mut offset = 5_usize;
    let mut decorations = BTreeMap::<u32, RawSpirvDecorations>::new();
    let mut variable_storage_classes = BTreeMap::<u32, u32>::new();
    let mut variable_types = BTreeMap::<u32, u32>::new();
    let mut types = BTreeMap::<u32, RawSpirvType>::new();
    let mut constant_result_types = BTreeMap::<u32, u32>::new();
    let mut zero_literal_constants = BTreeSet::<u32>::new();
    let mut selected_execution_model = None;
    let mut selected_entry_id = None;
    let mut literal_workgroups = BTreeMap::<u32, [u32; 3]>::new();
    let mut id_workgroups = BTreeMap::<u32, [u32; 3]>::new();
    while offset < words.len() {
        let instruction = words[offset];
        let word_count = usize::from((instruction >> 16) as u16);
        let opcode = (instruction & 0xffff) as u16;
        if word_count == 0 || offset + word_count > words.len() {
            return Err(SpirvSourceTranslationError::InvalidSpirv(
                "instruction word count is out of bounds",
            ));
        }
        if opcode == 71 {
            let operands = &words[offset + 1..offset + word_count];
            if operands.len() < 2 || operands[0] == 0 {
                return Err(SpirvSourceTranslationError::InvalidSpirv(
                    "OpDecorate has invalid operands",
                ));
            }
            let decoration = decorations.entry(operands[0]).or_default();
            match operands[1] {
                33 => {
                    if operands.len() < 3 {
                        return Err(SpirvSourceTranslationError::InvalidSpirv(
                            "Binding decoration has no literal",
                        ));
                    }
                    if decoration
                        .binding
                        .replace(operands[2])
                        .is_some_and(|value| value != operands[2])
                    {
                        return Err(SpirvSourceTranslationError::InvalidSpirv(
                            "resource binding decoration is inconsistent",
                        ));
                    }
                }
                34 => {
                    if operands.len() < 3 {
                        return Err(SpirvSourceTranslationError::InvalidSpirv(
                            "DescriptorSet decoration has no literal",
                        ));
                    }
                    if decoration
                        .descriptor_set
                        .replace(operands[2])
                        .is_some_and(|value| value != operands[2])
                    {
                        return Err(SpirvSourceTranslationError::InvalidSpirv(
                            "descriptor set decoration is inconsistent",
                        ));
                    }
                }
                1 => {
                    if operands.len() < 3 {
                        return Err(SpirvSourceTranslationError::InvalidSpirv(
                            "SpecId decoration has no literal",
                        ));
                    }
                    if decoration
                        .spec_id
                        .replace(operands[2])
                        .is_some_and(|value| value != operands[2])
                    {
                        return Err(SpirvSourceTranslationError::InvalidSpirv(
                            "SpecId decoration is inconsistent",
                        ));
                    }
                }
                24 => {
                    if operands.len() != 2 {
                        return Err(SpirvSourceTranslationError::InvalidSpirv(
                            "NonWritable decoration has unexpected operands",
                        ));
                    }
                    decoration.non_writable = true;
                }
                25 => {
                    if operands.len() != 2 {
                        return Err(SpirvSourceTranslationError::InvalidSpirv(
                            "NonReadable decoration has unexpected operands",
                        ));
                    }
                    decoration.non_readable = true;
                }
                _ => {}
            }
        }
        if opcode == 59 {
            let operands = &words[offset + 1..offset + word_count];
            if operands.len() < 3 || operands[1] == 0 {
                return Err(SpirvSourceTranslationError::InvalidSpirv(
                    "OpVariable has invalid operands",
                ));
            }
            if variable_storage_classes
                .insert(operands[1], operands[2])
                .is_some_and(|storage_class| storage_class != operands[2])
            {
                return Err(SpirvSourceTranslationError::InvalidSpirv(
                    "OpVariable storage class is inconsistent",
                ));
            }
            if variable_types
                .insert(operands[1], operands[0])
                .is_some_and(|type_id| type_id != operands[0])
            {
                return Err(SpirvSourceTranslationError::InvalidSpirv(
                    "OpVariable type is inconsistent",
                ));
            }
        }
        if let Some((type_id, ty)) =
            parse_raw_spirv_type(opcode, &words[offset + 1..offset + word_count])?
            && types.insert(type_id, ty).is_some()
        {
            return Err(SpirvSourceTranslationError::InvalidSpirv(
                "SPIR-V type id is declared more than once",
            ));
        }
        if matches!(opcode, 43 | 50 | 52) {
            let operands = &words[offset + 1..offset + word_count];
            if operands.len() < 3
                || operands[0] == 0
                || operands[0] >= words[3]
                || operands[1] == 0
                || operands[1] >= words[3]
                || (opcode == 52 && operands[2] == 0)
            {
                return Err(SpirvSourceTranslationError::InvalidSpirv(
                    "SPIR-V constant instruction has invalid operands",
                ));
            }
            if constant_result_types
                .insert(operands[1], operands[0])
                .is_some_and(|type_id| type_id != operands[0])
            {
                return Err(SpirvSourceTranslationError::InvalidSpirv(
                    "SPIR-V constant result type is inconsistent",
                ));
            }
            if opcode == 43 && operands[2..].iter().all(|word| *word == 0) {
                zero_literal_constants.insert(operands[1]);
            }
        }
        if opcode == 15 {
            let operands = &words[offset + 1..offset + word_count];
            if operands.len() < 3 {
                return Err(SpirvSourceTranslationError::InvalidSpirv(
                    "OpEntryPoint has too few operands",
                ));
            }
            let mut bytes = Vec::new();
            let mut string_end = None;
            for (index, word) in operands[2..].iter().enumerate() {
                for byte in word.to_le_bytes() {
                    if byte == 0 {
                        string_end = Some(index);
                        break;
                    }
                    bytes.push(byte);
                }
                if string_end.is_some() {
                    break;
                }
            }
            let Some(_) = string_end else {
                return Err(SpirvSourceTranslationError::InvalidSpirv(
                    "OpEntryPoint name is unterminated",
                ));
            };
            let name = String::from_utf8(bytes).map_err(|_| {
                SpirvSourceTranslationError::InvalidSpirv("OpEntryPoint name is not UTF-8")
            })?;
            if name == entry_name
                && selected_execution_model
                    .replace(operands[0])
                    .is_some_and(|model| model != operands[0])
            {
                return Err(SpirvSourceTranslationError::InvalidSpirv(
                    "selected entry point is duplicated with different execution models",
                ));
            }
            if name == entry_name
                && selected_entry_id
                    .replace(operands[1])
                    .is_some_and(|entry_id| entry_id != operands[1])
            {
                return Err(SpirvSourceTranslationError::InvalidSpirv(
                    "selected entry point is duplicated with different ids",
                ));
            }
        }
        if opcode == 16 {
            let operands = &words[offset + 1..offset + word_count];
            if operands.len() >= 2 && operands[1] == 17 {
                if operands.len() != 5
                    || operands[0] == 0
                    || operands[2] == 0
                    || operands[3] == 0
                    || operands[4] == 0
                {
                    return Err(SpirvSourceTranslationError::InvalidSpirv(
                        "OpExecutionMode LocalSize has invalid operands",
                    ));
                }
                let workgroup = [operands[2], operands[3], operands[4]];
                if literal_workgroups
                    .insert(operands[0], workgroup)
                    .is_some_and(|previous| previous != workgroup)
                {
                    return Err(SpirvSourceTranslationError::InvalidSpirv(
                        "LocalSize execution mode is inconsistent",
                    ));
                }
            }
        }
        if opcode == 331 {
            let operands = &words[offset + 1..offset + word_count];
            if operands.len() >= 2 && operands[1] == 38 {
                if operands.len() != 5
                    || operands[0] == 0
                    || operands[0] >= words[3]
                    || operands[2] == 0
                    || operands[2] >= words[3]
                    || operands[3] == 0
                    || operands[3] >= words[3]
                    || operands[4] == 0
                    || operands[4] >= words[3]
                {
                    return Err(SpirvSourceTranslationError::InvalidSpirv(
                        "OpExecutionModeId LocalSizeId has invalid operands",
                    ));
                }
                let workgroup_ids = [operands[2], operands[3], operands[4]];
                if id_workgroups
                    .insert(operands[0], workgroup_ids)
                    .is_some_and(|previous| previous != workgroup_ids)
                {
                    return Err(SpirvSourceTranslationError::InvalidSpirv(
                        "LocalSizeId execution mode is inconsistent",
                    ));
                }
            }
        }
        offset += word_count;
    }
    let Some(execution_model) = selected_execution_model else {
        return Err(SpirvSourceTranslationError::EntryPointNotFound(
            entry_name.to_owned(),
        ));
    };
    let workgroup_size =
        selected_entry_id.and_then(|entry_id| literal_workgroups.get(&entry_id).copied());
    let workgroup_size_ids =
        selected_entry_id.and_then(|entry_id| id_workgroups.get(&entry_id).copied());
    if workgroup_size.is_some() && workgroup_size_ids.is_some() {
        return Err(SpirvSourceTranslationError::InvalidSpirv(
            "selected entry has both LocalSize and LocalSizeId",
        ));
    }
    if let Some(workgroup_ids) = workgroup_size_ids {
        if workgroup_ids.iter().any(|id| {
            !constant_result_types.get(id).is_some_and(|type_id| {
                matches!(types.get(type_id), Some(RawSpirvType::Integer { .. }))
            })
        }) {
            return Err(SpirvSourceTranslationError::InvalidSpirv(
                "LocalSizeId operands are not scalar integer constants",
            ));
        }
        if workgroup_ids
            .iter()
            .any(|id| zero_literal_constants.contains(id))
        {
            return Err(SpirvSourceTranslationError::InvalidSpirv(
                "LocalSizeId contains a zero literal dimension",
            ));
        }
    }
    let workgroup_size_spec_ids = workgroup_size_ids.and_then(|ids| {
        Some([
            decorations.get(&ids[0])?.spec_id?,
            decorations.get(&ids[1])?.spec_id?,
            decorations.get(&ids[2])?.spec_id?,
        ])
    });
    let mut resources = Vec::new();
    let mut bindings = BTreeMap::<u32, u32>::new();
    for (variable_id, decoration) in decorations {
        let Some(binding) = decoration.binding else {
            continue;
        };
        if bindings.insert(binding, variable_id).is_some() {
            return Err(SpirvSourceTranslationError::InvalidSpirv(
                "multiple variables use the same descriptor binding",
            ));
        }
        let storage_class = variable_storage_classes.get(&variable_id).copied();
        let access = storage_class
            .map(|storage_class| {
                raw_spirv_access(
                    storage_class,
                    decoration.non_writable,
                    decoration.non_readable,
                )
            })
            .transpose()?
            .flatten();
        resources.push(SpirvRawResourceBinding {
            variable_id,
            binding,
            descriptor_set: decoration.descriptor_set.unwrap_or(0),
            storage_class,
            element_type: variable_types.get(&variable_id).and_then(|type_id| {
                raw_spirv_element_type(*type_id, &types, 0).map(|value| value.0)
            }),
            element_stride: variable_types.get(&variable_id).and_then(|type_id| {
                raw_spirv_element_type(*type_id, &types, 0).and_then(|value| value.1)
            }),
            access,
        });
    }
    resources.sort_by_key(|resource| (resource.descriptor_set, resource.binding));
    Ok(RawSpirvMetadata {
        execution_model,
        workgroup_size,
        workgroup_size_ids,
        workgroup_size_spec_ids,
        resources,
    })
}

/// Failure while evaluating a bounded raw-SPIR-V `LocalSizeId` expression.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpirvRawWorkgroupEvaluationError {
    /// The module failed the shared raw-SPIR-V structural contract.
    InvalidSpirv(SpirvSourceTranslationError),
    /// The selected entry does not use `LocalSizeId` metadata.
    MissingLocalSizeId,
    /// A referenced constant result was not declared.
    MissingConstant { id: u32 },
    /// A constant result did not retain an integer type declaration.
    MissingConstantType { id: u32 },
    /// This evaluator currently accepts 32-bit integer dimensions only.
    UnsupportedIntegerWidth { id: u32, bits: u16 },
    /// The bounded evaluator does not implement this SPIR-V operation yet.
    UnsupportedOperation { opcode: u16 },
    /// An integer division or remainder used a zero divisor.
    DivisionByZero { opcode: u16 },
    /// A signed integer operation overflowed its 32-bit result domain.
    ArithmeticOverflow { opcode: u16 },
    /// A shift amount is outside the supported 32-bit scalar range.
    ShiftOutOfRange { opcode: u16, amount: u32 },
    /// The expression graph contains a cycle.
    CyclicExpression { id: u32 },
    /// A resolved local dimension is zero.
    ZeroDimension { index: usize },
    /// A signed local dimension resolved to a negative value.
    NegativeDimension { index: usize },
}

impl fmt::Display for SpirvRawWorkgroupEvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSpirv(error) => {
                write!(formatter, "raw LocalSizeId module is invalid: {error}")
            }
            Self::MissingLocalSizeId => formatter.write_str("selected entry has no LocalSizeId"),
            Self::MissingConstant { id } => {
                write!(formatter, "LocalSizeId constant {id} is undefined")
            }
            Self::MissingConstantType { id } => {
                write!(formatter, "LocalSizeId constant {id} has no integer type")
            }
            Self::UnsupportedIntegerWidth { id, bits } => write!(
                formatter,
                "LocalSizeId constant {id} uses unsupported integer width {bits}"
            ),
            Self::UnsupportedOperation { opcode } => {
                write!(
                    formatter,
                    "LocalSizeId specialization operation {opcode} is unsupported"
                )
            }
            Self::DivisionByZero { opcode } => {
                write!(
                    formatter,
                    "LocalSizeId specialization operation {opcode} divides by zero"
                )
            }
            Self::ArithmeticOverflow { opcode } => write!(
                formatter,
                "LocalSizeId specialization operation {opcode} overflowed 32-bit signed arithmetic"
            ),
            Self::ShiftOutOfRange { opcode, amount } => write!(
                formatter,
                "LocalSizeId specialization operation {opcode} shifts by invalid amount {amount}"
            ),
            Self::CyclicExpression { id } => {
                write!(
                    formatter,
                    "LocalSizeId specialization expression cycles at {id}"
                )
            }
            Self::ZeroDimension { index } => {
                write!(formatter, "resolved LocalSizeId dimension {index} is zero")
            }
            Self::NegativeDimension { index } => {
                write!(
                    formatter,
                    "resolved LocalSizeId dimension {index} is negative"
                )
            }
        }
    }
}

impl Error for SpirvRawWorkgroupEvaluationError {}

#[derive(Clone, Debug)]
enum RawWorkgroupConstant {
    Literal {
        type_id: u32,
        value: u32,
    },
    Spec {
        type_id: u32,
        default: u32,
    },
    Operation {
        type_id: u32,
        opcode: u16,
        operands: Vec<u32>,
    },
}

type RawWorkgroupConstantSet = (
    BTreeMap<u32, RawWorkgroupConstant>,
    BTreeMap<u32, u32>,
    BTreeMap<u32, (u16, bool)>,
);

/// Evaluates a bounded scalar `LocalSizeId` graph using caller overrides for
/// reflected `SpecId` decorations.
///
/// Direct `OpConstant`/`OpSpecConstant` values and a conservative integer
/// `OpSpecConstantOp` subset are supported. Missing caller overrides use the
/// SPIR-V default value. This helper is deliberately separate from source
/// translation and does not rewrite or recompile the module.
pub fn resolve_spirv_source_workgroup_size(
    words: &[u32],
    entry_name: &str,
    spec_values: &BTreeMap<u32, u32>,
) -> Result<[u32; 3], SpirvRawWorkgroupEvaluationError> {
    let contract = inspect_spirv_source_module(words, entry_name)
        .map_err(SpirvRawWorkgroupEvaluationError::InvalidSpirv)?;
    if let Some(size) = contract.workgroup_size {
        return validate_workgroup_dimensions(size).map_err(|error| match error {
            SpirvRawNativeAdapterError::ZeroWorkgroupDimension { index, .. } => {
                SpirvRawWorkgroupEvaluationError::ZeroDimension { index }
            }
            _ => unreachable!("literal dimensions only fail for zero values"),
        });
    }
    let ids = contract
        .workgroup_size_ids
        .ok_or(SpirvRawWorkgroupEvaluationError::MissingLocalSizeId)?;
    let (constants, spec_ids, types) = collect_raw_workgroup_constants(words)?;
    let mut memo = BTreeMap::new();
    let mut active = BTreeSet::new();
    let mut dimensions = [0_u32; 3];
    for (index, id) in ids.iter().copied().enumerate() {
        dimensions[index] = evaluate_raw_workgroup_constant(
            id,
            &constants,
            &spec_ids,
            spec_values,
            &types,
            &mut memo,
            &mut active,
        )?;
        let type_id = match constants
            .get(&id)
            .ok_or(SpirvRawWorkgroupEvaluationError::MissingConstant { id })?
        {
            RawWorkgroupConstant::Literal { type_id, .. }
            | RawWorkgroupConstant::Spec { type_id, .. }
            | RawWorkgroupConstant::Operation { type_id, .. } => *type_id,
        };
        if types.get(&type_id).is_some_and(|(_, signed)| *signed)
            && dimensions[index] > i32::MAX as u32
        {
            return Err(SpirvRawWorkgroupEvaluationError::NegativeDimension { index });
        }
        if dimensions[index] == 0 {
            return Err(SpirvRawWorkgroupEvaluationError::ZeroDimension { index });
        }
    }
    Ok(dimensions)
}

fn collect_raw_workgroup_constants(
    words: &[u32],
) -> Result<RawWorkgroupConstantSet, SpirvRawWorkgroupEvaluationError> {
    let mut constants = BTreeMap::new();
    let mut spec_ids = BTreeMap::new();
    let mut types = BTreeMap::new();
    let mut offset = 5_usize;
    while offset < words.len() {
        let word_count = usize::from((words[offset] >> 16) as u16);
        let opcode = (words[offset] & 0xffff) as u16;
        if word_count == 0 || offset + word_count > words.len() {
            return Err(SpirvRawWorkgroupEvaluationError::InvalidSpirv(
                SpirvSourceTranslationError::InvalidSpirv(
                    "instruction word count is out of bounds",
                ),
            ));
        }
        let operands = &words[offset + 1..offset + word_count];
        match opcode {
            21 if operands.len() >= 3 => {
                types.insert(operands[0], (operands[1] as u16, operands[2] != 0));
            }
            71 if operands.len() >= 3 && operands[1] == 1 => {
                spec_ids.insert(operands[0], operands[2]);
            }
            43 | 50 if operands.len() == 3 => {
                let value = if opcode == 43 {
                    RawWorkgroupConstant::Literal {
                        type_id: operands[0],
                        value: operands[2],
                    }
                } else {
                    RawWorkgroupConstant::Spec {
                        type_id: operands[0],
                        default: operands[2],
                    }
                };
                constants.insert(operands[1], value);
            }
            52 if operands.len() >= 3 => {
                constants.insert(
                    operands[1],
                    RawWorkgroupConstant::Operation {
                        type_id: operands[0],
                        opcode: operands[2] as u16,
                        operands: operands[3..].to_vec(),
                    },
                );
            }
            _ => {}
        }
        offset += word_count;
    }
    Ok((constants, spec_ids, types))
}

fn evaluate_raw_workgroup_constant(
    id: u32,
    constants: &BTreeMap<u32, RawWorkgroupConstant>,
    spec_ids: &BTreeMap<u32, u32>,
    spec_values: &BTreeMap<u32, u32>,
    types: &BTreeMap<u32, (u16, bool)>,
    memo: &mut BTreeMap<u32, u32>,
    active: &mut BTreeSet<u32>,
) -> Result<u32, SpirvRawWorkgroupEvaluationError> {
    if let Some(value) = memo.get(&id).copied() {
        return Ok(value);
    }
    if !active.insert(id) {
        return Err(SpirvRawWorkgroupEvaluationError::CyclicExpression { id });
    }
    let result = (|| {
        let constant = constants
            .get(&id)
            .ok_or(SpirvRawWorkgroupEvaluationError::MissingConstant { id })?;
        let type_id = match constant {
            RawWorkgroupConstant::Literal { type_id, .. }
            | RawWorkgroupConstant::Spec { type_id, .. }
            | RawWorkgroupConstant::Operation { type_id, .. } => *type_id,
        };
        let Some((bits, signed)) = types.get(&type_id).copied() else {
            return Err(SpirvRawWorkgroupEvaluationError::MissingConstantType { id });
        };
        if bits != 32 {
            return Err(SpirvRawWorkgroupEvaluationError::UnsupportedIntegerWidth { id, bits });
        }
        match constant {
            RawWorkgroupConstant::Literal { value, .. } => Ok(*value),
            RawWorkgroupConstant::Spec { default, .. } => Ok(spec_ids
                .get(&id)
                .and_then(|spec_id| spec_values.get(spec_id))
                .copied()
                .unwrap_or(*default)),
            RawWorkgroupConstant::Operation {
                opcode, operands, ..
            } => {
                if let Some(value) = spec_ids
                    .get(&id)
                    .and_then(|spec_id| spec_values.get(spec_id))
                    .copied()
                {
                    return Ok(value);
                }
                let mut values = Vec::with_capacity(operands.len());
                for operand in operands {
                    values.push(evaluate_raw_workgroup_constant(
                        *operand,
                        constants,
                        spec_ids,
                        spec_values,
                        types,
                        memo,
                        active,
                    )?);
                }
                evaluate_raw_workgroup_operation(*opcode, &values, signed)
            }
        }
    })();
    active.remove(&id);
    if let Ok(value) = result {
        memo.insert(id, value);
    }
    result
}

fn evaluate_raw_workgroup_operation(
    opcode: u16,
    operands: &[u32],
    signed: bool,
) -> Result<u32, SpirvRawWorkgroupEvaluationError> {
    let binary = |operation: fn(u32, u32) -> u32| {
        if operands.len() != 2 {
            return Err(SpirvRawWorkgroupEvaluationError::UnsupportedOperation { opcode });
        }
        Ok(operation(operands[0], operands[1]))
    };
    match opcode {
        126 if operands.len() == 1 => (operands[0] as i32)
            .checked_neg()
            .map(|value| value as u32)
            .ok_or(SpirvRawWorkgroupEvaluationError::ArithmeticOverflow { opcode }),
        128 if signed => {
            if operands.len() != 2 {
                return Err(SpirvRawWorkgroupEvaluationError::UnsupportedOperation { opcode });
            }
            (operands[0] as i32)
                .checked_add(operands[1] as i32)
                .map(|value| value as u32)
                .ok_or(SpirvRawWorkgroupEvaluationError::ArithmeticOverflow { opcode })
        }
        128 => binary(u32::wrapping_add),
        130 if signed => {
            if operands.len() != 2 {
                return Err(SpirvRawWorkgroupEvaluationError::UnsupportedOperation { opcode });
            }
            (operands[0] as i32)
                .checked_sub(operands[1] as i32)
                .map(|value| value as u32)
                .ok_or(SpirvRawWorkgroupEvaluationError::ArithmeticOverflow { opcode })
        }
        130 => binary(u32::wrapping_sub),
        132 if signed => {
            if operands.len() != 2 {
                return Err(SpirvRawWorkgroupEvaluationError::UnsupportedOperation { opcode });
            }
            (operands[0] as i32)
                .checked_mul(operands[1] as i32)
                .map(|value| value as u32)
                .ok_or(SpirvRawWorkgroupEvaluationError::ArithmeticOverflow { opcode })
        }
        132 => binary(u32::wrapping_mul),
        134 => {
            if operands.len() != 2 {
                return Err(SpirvRawWorkgroupEvaluationError::UnsupportedOperation { opcode });
            }
            if operands[1] == 0 {
                return Err(SpirvRawWorkgroupEvaluationError::DivisionByZero { opcode });
            }
            Ok(operands[0] / operands[1])
        }
        135 => {
            if operands.len() != 2 {
                return Err(SpirvRawWorkgroupEvaluationError::UnsupportedOperation { opcode });
            }
            let right = operands[1] as i32;
            if right == 0 {
                return Err(SpirvRawWorkgroupEvaluationError::DivisionByZero { opcode });
            }
            (operands[0] as i32)
                .checked_div(right)
                .map(|value| value as u32)
                .ok_or(SpirvRawWorkgroupEvaluationError::ArithmeticOverflow { opcode })
        }
        137 => {
            if operands.len() != 2 {
                return Err(SpirvRawWorkgroupEvaluationError::UnsupportedOperation { opcode });
            }
            if operands[1] == 0 {
                return Err(SpirvRawWorkgroupEvaluationError::DivisionByZero { opcode });
            }
            Ok(operands[0] % operands[1])
        }
        138 | 139 => {
            if operands.len() != 2 {
                return Err(SpirvRawWorkgroupEvaluationError::UnsupportedOperation { opcode });
            }
            let left = operands[0] as i32;
            let right = operands[1] as i32;
            if right == 0 {
                return Err(SpirvRawWorkgroupEvaluationError::DivisionByZero { opcode });
            }
            let remainder = left
                .checked_rem(right)
                .ok_or(SpirvRawWorkgroupEvaluationError::ArithmeticOverflow { opcode })?;
            let value = if opcode == 139 && remainder != 0 && (remainder < 0) != (right < 0) {
                remainder
                    .checked_add(right)
                    .ok_or(SpirvRawWorkgroupEvaluationError::ArithmeticOverflow { opcode })?
            } else {
                remainder
            };
            Ok(value as u32)
        }
        194..=196 => {
            if operands.len() != 2 || operands[1] >= 32 {
                return Err(SpirvRawWorkgroupEvaluationError::ShiftOutOfRange {
                    opcode,
                    amount: operands.get(1).copied().unwrap_or(u32::MAX),
                });
            }
            let amount = operands[1];
            Ok(match opcode {
                194 => operands[0] >> amount,
                195 => ((operands[0] as i32) >> amount) as u32,
                _ => operands[0] << amount,
            })
        }
        197 => binary(|left, right| left | right),
        198 => binary(|left, right| left ^ right),
        199 => binary(|left, right| left & right),
        200 if operands.len() == 1 => Ok(!operands[0]),
        169 if operands.len() == 3 => Ok(if operands[0] != 0 {
            operands[1]
        } else {
            operands[2]
        }),
        170 => binary(|left, right| u32::from(left == right)),
        171 => binary(|left, right| u32::from(left != right)),
        172 => binary(|left, right| u32::from(left > right)),
        173 => binary(|left, right| u32::from((left as i32) > (right as i32))),
        174 => binary(|left, right| u32::from(left >= right)),
        175 => binary(|left, right| u32::from((left as i32) >= (right as i32))),
        176 => binary(|left, right| u32::from(left < right)),
        177 => binary(|left, right| u32::from((left as i32) < (right as i32))),
        178 => binary(|left, right| u32::from(left <= right)),
        179 => binary(|left, right| u32::from((left as i32) <= (right as i32))),
        166 => binary(|left, right| u32::from(left != 0 && right != 0)),
        167 => binary(|left, right| u32::from(left != 0 || right != 0)),
        168 if operands.len() == 1 => Ok(u32::from(operands[0] == 0)),
        _ => Err(SpirvRawWorkgroupEvaluationError::UnsupportedOperation { opcode }),
    }
}

fn parse_raw_spirv_type(
    opcode: u16,
    operands: &[u32],
) -> Result<Option<(u32, RawSpirvType)>, SpirvSourceTranslationError> {
    let invalid = || {
        Err(SpirvSourceTranslationError::InvalidSpirv(
            "SPIR-V type instruction has invalid operands",
        ))
    };
    let parsed = match opcode {
        21 => {
            if operands.len() != 3 || operands[0] == 0 || operands[1] == 0 || operands[2] > 1 {
                return invalid();
            }
            Some((
                operands[0],
                RawSpirvType::Integer {
                    bits: u16::try_from(operands[1]).map_err(|_| {
                        SpirvSourceTranslationError::InvalidSpirv(
                            "SPIR-V integer width exceeds u16",
                        )
                    })?,
                    signed: operands[2] == 1,
                },
            ))
        }
        22 => {
            if operands.len() != 2 || operands[0] == 0 || operands[1] == 0 {
                return invalid();
            }
            Some((
                operands[0],
                RawSpirvType::Float {
                    bits: u16::try_from(operands[1]).map_err(|_| {
                        SpirvSourceTranslationError::InvalidSpirv("SPIR-V float width exceeds u16")
                    })?,
                },
            ))
        }
        23 => {
            if operands.len() != 3 || operands[0] == 0 || operands[1] == 0 || operands[2] == 0 {
                return invalid();
            }
            Some((
                operands[0],
                RawSpirvType::Vector {
                    component: operands[1],
                    lanes: u16::try_from(operands[2]).map_err(|_| {
                        SpirvSourceTranslationError::InvalidSpirv(
                            "SPIR-V vector lane count exceeds u16",
                        )
                    })?,
                },
            ))
        }
        29 => {
            if operands.len() != 2 || operands[0] == 0 || operands[1] == 0 {
                return invalid();
            }
            Some((
                operands[0],
                RawSpirvType::RuntimeArray {
                    element: operands[1],
                },
            ))
        }
        30 => {
            if operands.is_empty() || operands[0] == 0 {
                return invalid();
            }
            Some((
                operands[0],
                RawSpirvType::Struct {
                    members: operands[1..].to_vec(),
                },
            ))
        }
        32 => {
            if operands.len() != 3 || operands[0] == 0 || operands[2] == 0 {
                return invalid();
            }
            Some((
                operands[0],
                RawSpirvType::Pointer {
                    pointee: operands[2],
                },
            ))
        }
        _ => None,
    };
    Ok(parsed)
}

fn raw_spirv_element_type(
    type_id: u32,
    types: &BTreeMap<u32, RawSpirvType>,
    depth: u8,
) -> Option<(ResourceElementType, Option<u32>)> {
    if depth > 64 {
        return None;
    }
    match types.get(&type_id)? {
        RawSpirvType::Integer { bits, signed } => {
            let element = ResourceElementType::Integer {
                signed: *signed,
                bits: *bits,
                lanes: 1,
            };
            Some((element, element.byte_stride()))
        }
        RawSpirvType::Float { bits } => {
            let element = ResourceElementType::Float {
                bits: *bits,
                lanes: 1,
            };
            Some((element, element.byte_stride()))
        }
        RawSpirvType::Vector { component, lanes } => {
            if !(1..=4).contains(lanes) {
                return None;
            }
            let (scalar, _) = raw_spirv_element_type(*component, types, depth + 1)?;
            let element = match scalar {
                ResourceElementType::Integer { signed, bits, .. } => ResourceElementType::Integer {
                    signed,
                    bits,
                    lanes: *lanes,
                },
                ResourceElementType::Float { bits, .. } => ResourceElementType::Float {
                    bits,
                    lanes: *lanes,
                },
            };
            Some((element, element.byte_stride()))
        }
        RawSpirvType::Pointer { pointee } | RawSpirvType::RuntimeArray { element: pointee } => {
            raw_spirv_element_type(*pointee, types, depth + 1)
        }
        RawSpirvType::Struct { members } if members.len() == 1 => {
            raw_spirv_element_type(members[0], types, depth + 1)
        }
        RawSpirvType::Struct { .. } => None,
    }
}

fn raw_spirv_access(
    storage_class: u32,
    non_writable: bool,
    non_readable: bool,
) -> Result<Option<ResourceAccess>, SpirvSourceTranslationError> {
    if non_writable && non_readable {
        return Err(SpirvSourceTranslationError::InvalidSpirv(
            "resource cannot be both NonWritable and NonReadable",
        ));
    }
    match storage_class {
        2 if non_readable => Err(SpirvSourceTranslationError::InvalidSpirv(
            "read-only storage class cannot be NonReadable",
        )),
        2 => Ok(Some(ResourceAccess::ReadOnly)),
        12 if non_writable => Ok(Some(ResourceAccess::ReadOnly)),
        12 if non_readable => Ok(Some(ResourceAccess::WriteOnly)),
        12 => Ok(Some(ResourceAccess::ReadWrite)),
        _ => Ok(None),
    }
}

/// One backend's source evidence in a raw SPIR-V parity report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpirvSourceParitySource {
    /// Source backend represented by this evidence.
    pub backend: ArtifactSourceBackend,
    /// UTF-8 byte count of the translated source.
    pub source_byte_count: usize,
    /// Stable FNV-1a hash of the translated source.
    pub source_hash: u64,
}

/// Metadata-only parity report for raw MSL/HLSL source translations.
///
/// The source texts are allowed to differ by backend. Parity is established
/// only for the selected entry and input SPIR-V identity; source hashes remain
/// per-backend audit evidence rather than an equality claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpirvSourceParityReport {
    /// Explicit entry point shared by all source reports.
    pub entry_name: String,
    /// Execution model shared by all source reports.
    pub execution_model: u32,
    /// Literal local workgroup dimensions shared by the source reports.
    pub workgroup_size: Option<[u32; 3]>,
    /// LocalSizeId specialization IDs shared by the source reports.
    pub workgroup_size_ids: Option<[u32; 3]>,
    /// External SpecId decorations shared by the source reports.
    pub workgroup_size_spec_ids: Option<[u32; 3]>,
    /// Descriptor identities shared by all source reports.
    pub resources: Vec<SpirvRawResourceBinding>,
    /// Number of input SPIR-V words shared by all source reports.
    pub word_count: usize,
    /// Stable hash of the input SPIR-V words.
    pub word_hash: u64,
    /// Per-backend source audit evidence in caller-provided order.
    pub sources: Vec<SpirvSourceParitySource>,
}

/// Why raw source reports cannot form one parity audit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpirvSourceParityError {
    /// At least two distinct source backends are required.
    InsufficientReports { actual: usize },
    /// Reports were built from different entry points or SPIR-V words.
    ModuleIdentityMismatch,
    /// One backend was supplied more than once.
    DuplicateBackend(ArtifactSourceBackend),
}

impl fmt::Display for SpirvSourceParityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InsufficientReports { actual } => {
                write!(
                    formatter,
                    "raw source parity requires at least two reports, got {actual}"
                )
            }
            Self::ModuleIdentityMismatch => {
                formatter.write_str("raw source reports have different module identities")
            }
            Self::DuplicateBackend(backend) => {
                write!(
                    formatter,
                    "raw source parity contains duplicate backend {backend:?}"
                )
            }
        }
    }
}

impl Error for SpirvSourceParityError {}

/// Compares raw source reports without claiming source-text equality or
/// compilation/execution parity.
pub fn compare_spirv_source_reports(
    reports: &[SpirvSourceTranslationReport],
) -> Result<SpirvSourceParityReport, SpirvSourceParityError> {
    if reports.len() < 2 {
        return Err(SpirvSourceParityError::InsufficientReports {
            actual: reports.len(),
        });
    }
    let first = &reports[0];
    let mut sources = Vec::with_capacity(reports.len());
    for report in reports {
        if report.identity.entry_name != first.identity.entry_name
            || report.identity.execution_model != first.identity.execution_model
            || report.identity.workgroup_size != first.identity.workgroup_size
            || report.identity.workgroup_size_ids != first.identity.workgroup_size_ids
            || report.identity.workgroup_size_spec_ids != first.identity.workgroup_size_spec_ids
            || report.identity.resources != first.identity.resources
            || report.identity.word_count != first.identity.word_count
            || report.identity.word_hash != first.identity.word_hash
        {
            return Err(SpirvSourceParityError::ModuleIdentityMismatch);
        }
        if sources
            .iter()
            .any(|source: &SpirvSourceParitySource| source.backend == report.identity.backend)
        {
            return Err(SpirvSourceParityError::DuplicateBackend(
                report.identity.backend,
            ));
        }
        sources.push(SpirvSourceParitySource {
            backend: report.identity.backend,
            source_byte_count: report.source_byte_count,
            source_hash: report.source_hash,
        });
    }
    Ok(SpirvSourceParityReport {
        entry_name: first.identity.entry_name.clone(),
        execution_model: first.identity.execution_model,
        workgroup_size: first.identity.workgroup_size,
        workgroup_size_ids: first.identity.workgroup_size_ids,
        workgroup_size_spec_ids: first.identity.workgroup_size_spec_ids,
        resources: first.identity.resources.clone(),
        word_count: first.identity.word_count,
        word_hash: first.identity.word_hash,
        sources,
    })
}

/// Failure while translating a validated portable artifact to backend source.
///
/// The error keeps the common artifact contract separate from the external
/// SPIRV-Cross process boundary. Backend crates can then add their own source
/// reflection rules without duplicating the input-plan and report lifecycle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactSourceTranslationError {
    /// The artifact failed the shared portable identity/resource contract.
    Contract(SpirvArtifactContractError),
    /// The external SPIRV-Cross process could not produce source output.
    Tool(SpirvCrossError),
}

impl fmt::Display for ArtifactSourceTranslationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => write!(formatter, "artifact source contract failed: {error}"),
            Self::Tool(error) => write!(formatter, "artifact source translation failed: {error}"),
        }
    }
}

impl Error for ArtifactSourceTranslationError {}

/// Translates any artifact accepted by the shared portable contract through
/// SPIRV-Cross and returns the common source audit report.
///
/// This is a source-only boundary: callers must still validate backend
/// resource semantics and perform native compilation/execution separately.
pub fn translate_spirv_artifact_source(
    artifact: &SpirvArtifact,
    tool: &Path,
    backend: ArtifactSourceBackend,
) -> Result<ArtifactSourceTranslationReport, ArtifactSourceTranslationError> {
    let plan = ArtifactSourceTranslationPlan::from_artifact(artifact, backend)
        .map_err(ArtifactSourceTranslationError::Contract)?;
    let source_words = annotate_spirv_resource_names(artifact.words.clone(), &artifact.resources);
    let source =
        translate_spirv_with_spirv_cross(tool, &source_words, &artifact.entry_name, backend)
            .map_err(ArtifactSourceTranslationError::Tool)?;
    plan.into_report(source)
        .map_err(ArtifactSourceTranslationError::Contract)
}

/// Metadata-only comparison of two or more source translation reports for one
/// artifact. This does not claim that either source was compiled or executed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactSourceParityReport {
    /// Shared identity of the artifact represented by every source report.
    pub artifact: SpirvArtifactIdentity,
    /// Shared resource capabilities represented by every source report.
    pub resources: Vec<ArtifactResourceCapability>,
    /// Source backends included in declaration order.
    pub backends: Vec<ArtifactSourceBackend>,
    /// Source hashes in the same order as `backends`.
    pub source_hashes: Vec<u64>,
    /// UTF-8 source byte counts in the same order as `backends`.
    pub source_byte_counts: Vec<usize>,
}

/// Why source translation reports cannot form one cross-backend parity audit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactSourceParityError {
    /// A parity audit needs at least two distinct backend reports.
    InsufficientReports { actual: usize },
    /// Reports refer to different validated artifact identities.
    ArtifactIdentityMismatch,
    /// Reports expose different portable resource capability matrices.
    ResourceCapabilityMismatch,
    /// The same source backend was supplied more than once.
    DuplicateBackend(ArtifactSourceBackend),
}

impl fmt::Display for ArtifactSourceParityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InsufficientReports { actual } => write!(
                formatter,
                "source parity requires at least two reports, got {actual}"
            ),
            Self::ArtifactIdentityMismatch => {
                formatter.write_str("source reports refer to different artifact identities")
            }
            Self::ResourceCapabilityMismatch => {
                formatter.write_str("source reports expose different resource capabilities")
            }
            Self::DuplicateBackend(backend) => {
                write!(formatter, "source parity repeats backend {backend:?}")
            }
        }
    }
}

impl Error for ArtifactSourceParityError {}

/// Builds a deterministic metadata-only parity report for source backends.
pub fn compare_artifact_source_reports(
    reports: &[ArtifactSourceTranslationReport],
) -> Result<ArtifactSourceParityReport, ArtifactSourceParityError> {
    if reports.len() < 2 {
        return Err(ArtifactSourceParityError::InsufficientReports {
            actual: reports.len(),
        });
    }
    let first = &reports[0];
    let mut backends = Vec::with_capacity(reports.len());
    let mut source_hashes = Vec::with_capacity(reports.len());
    let mut source_byte_counts = Vec::with_capacity(reports.len());
    for report in reports {
        if report.artifact != first.artifact {
            return Err(ArtifactSourceParityError::ArtifactIdentityMismatch);
        }
        if report.resources != first.resources {
            return Err(ArtifactSourceParityError::ResourceCapabilityMismatch);
        }
        if backends.contains(&report.backend) {
            return Err(ArtifactSourceParityError::DuplicateBackend(report.backend));
        }
        backends.push(report.backend);
        source_hashes.push(report.source_hash);
        source_byte_counts.push(report.source_byte_count);
    }
    Ok(ArtifactSourceParityReport {
        artifact: first.artifact.clone(),
        resources: first.resources.clone(),
        backends,
        source_hashes,
        source_byte_counts,
    })
}

/// Validates the portable artifact and returns metadata shared by native
/// backend adapters.
pub fn validate_spirv_artifact_contract(
    artifact: &SpirvArtifact,
) -> Result<SpirvArtifactIdentity, SpirvArtifactContractError> {
    artifact
        .validate()
        .map_err(SpirvArtifactContractError::InvalidSpirv)?;
    if artifact.entry_name.is_empty() {
        return Err(SpirvArtifactContractError::EmptyEntryName);
    }
    if artifact.entry_name.contains('\0') {
        return Err(SpirvArtifactContractError::EntryNameContainsNul);
    }
    if artifact.workgroup_size.contains(&0) {
        return Err(SpirvArtifactContractError::ZeroWorkgroup(
            artifact.workgroup_size,
        ));
    }
    let product = u64::from(artifact.workgroup_size[0])
        .checked_mul(u64::from(artifact.workgroup_size[1]))
        .and_then(|product| product.checked_mul(u64::from(artifact.workgroup_size[2])))
        .ok_or(SpirvArtifactContractError::WorkgroupTooLarge(u64::MAX))?;
    if product > 1024 {
        return Err(SpirvArtifactContractError::WorkgroupTooLarge(product));
    }

    let mut names = BTreeSet::new();
    for (index, resource) in artifact.resources.iter().enumerate() {
        let expected = u32::try_from(index).unwrap_or(u32::MAX);
        if resource.binding != expected {
            return Err(SpirvArtifactContractError::NonCanonicalBinding {
                expected,
                actual: resource.binding,
            });
        }
        if resource.name.is_empty() {
            return Err(SpirvArtifactContractError::EmptyResourceName(
                resource.binding,
            ));
        }
        if resource.name.contains('\0') {
            return Err(SpirvArtifactContractError::ResourceNameContainsNul(
                resource.binding,
            ));
        }
        if let Some(stride) = resource.element_stride
            && stride == 0
        {
            return Err(SpirvArtifactContractError::InvalidResourceStride {
                binding: resource.binding,
                stride,
            });
        }
        if let Some(element_type) = resource.element_type_info {
            if !element_type.is_valid() {
                return Err(SpirvArtifactContractError::InvalidResourceTypeMetadata {
                    binding: resource.binding,
                });
            }
            if let (Some(type_stride), Some(stride)) =
                (element_type.byte_stride(), resource.element_stride)
                && type_stride != stride
            {
                return Err(SpirvArtifactContractError::ResourceTypeStrideMismatch {
                    binding: resource.binding,
                    type_stride,
                    stride,
                });
            }
        }
        if !names.insert(resource.name.as_str()) {
            return Err(SpirvArtifactContractError::DuplicateResourceName(
                resource.name.clone(),
            ));
        }
    }

    Ok(SpirvArtifactIdentity {
        entry_name: artifact.entry_name.clone(),
        workgroup_size: artifact.workgroup_size,
        resource_binding_count: artifact.resources.len(),
        word_count: artifact.words.len(),
        word_hash: stable_spirv_word_hash(&artifact.words),
    })
}

/// Requirements supplied when routing a validated artifact to a backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactDispatchRequest {
    /// Floating-point policy required by the kernel.
    pub fp: FpPolicy,
    /// Whether the artifact requires the bounded global-id array subset.
    pub require_bounded_global_u32_array: bool,
    /// Whether the host needs asynchronous completion.
    pub require_async_completion: bool,
}

/// Capability-gated artifact route selected before any native API call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactDispatchPlan {
    /// Backend capability plan that owns transport and completion semantics.
    pub backend: BackendPlan,
    /// Shared identity of the validated artifact being routed.
    pub artifact: SpirvArtifactIdentity,
}

/// Non-zero workgroup grid supplied to one compute dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DispatchGeometry {
    workgroups: [u32; 3],
}

impl DispatchGeometry {
    /// Creates a checked dispatch grid. Every dimension must be non-zero and
    /// the total workgroup count must fit in `u64`.
    pub fn new(workgroups: [u32; 3]) -> Result<Self, DispatchGeometryError> {
        for (axis, dimension) in workgroups.into_iter().enumerate() {
            if dimension == 0 {
                return Err(DispatchGeometryError::ZeroDimension {
                    axis: u8::try_from(axis).expect("dispatch axis count fits u8"),
                });
            }
        }
        workgroups
            .into_iter()
            .try_fold(1_u64, |product, dimension| {
                product
                    .checked_mul(u64::from(dimension))
                    .ok_or(DispatchGeometryError::WorkgroupCountOverflow)
            })?;
        Ok(Self { workgroups })
    }

    /// Returns the three-dimensional workgroup grid.
    #[must_use]
    pub const fn workgroups(self) -> [u32; 3] {
        self.workgroups
    }

    /// Returns total invocations for one artifact workgroup size.
    pub fn invocation_count(self, workgroup_size: [u32; 3]) -> Result<u64, DispatchGeometryError> {
        self.workgroups.into_iter().zip(workgroup_size).try_fold(
            1_u64,
            |product, (groups, local)| {
                product
                    .checked_mul(u64::from(groups))
                    .and_then(|product| product.checked_mul(u64::from(local)))
                    .ok_or(DispatchGeometryError::InvocationCountOverflow)
            },
        )
    }
}

/// Why a dispatch grid cannot be represented safely.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchGeometryError {
    /// One of x/y/z dimensions was zero.
    ZeroDimension { axis: u8 },
    /// Workgroup grid product exceeds `u64`.
    WorkgroupCountOverflow,
    /// Grid × local workgroup product exceeds `u64`.
    InvocationCountOverflow,
}

impl fmt::Display for DispatchGeometryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDimension { axis } => {
                write!(formatter, "dispatch dimension {axis} must be non-zero")
            }
            Self::WorkgroupCountOverflow => {
                formatter.write_str("dispatch workgroup count overflows u64")
            }
            Self::InvocationCountOverflow => {
                formatter.write_str("dispatch invocation count overflows u64")
            }
        }
    }
}

impl Error for DispatchGeometryError {}

/// One host buffer supplied for an artifact descriptor binding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactResourceRequest {
    /// Dense binding ordinal expected by the artifact.
    pub binding: u32,
    /// Host-side buffer identity to encode at this binding.
    pub buffer: BufferId,
    /// Minimum byte capacity required by the kernel contract.
    pub required_bytes: u64,
}

/// Validated binding information consumed by a native descriptor encoder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactResourceBinding {
    /// Dense artifact binding ordinal.
    pub binding: u32,
    /// Descriptor set/space carried by the artifact metadata.
    pub descriptor_set: u32,
    /// JIR address space preserved by the portable artifact contract.
    pub address_space: AddressSpace,
    /// Scalar/vector element classification when known.
    pub element_type_info: Option<jadren_codegen_spirv::ResourceElementType>,
    /// Conservative layout classification including any explicit byte stride.
    pub layout: ArtifactResourceLayout,
    /// Host-side buffer identity.
    pub buffer: BufferId,
    /// Conservative access required for the complete dispatch scope.
    pub access: AccessKind,
    /// Minimum byte capacity checked while creating the plan.
    pub required_bytes: u64,
}

/// Data-only resource binding plan for one validated SPIR-V artifact.
///
/// The plan checks artifact metadata and host buffer capacities but does not
/// mutate residency or create an API descriptor. Callers must acquire the
/// returned plan through [`ResourceTable::acquire_artifact_resources`] before
/// submitting native work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactResourcePlan {
    artifact: SpirvArtifactIdentity,
    bindings: Vec<ArtifactResourceBinding>,
}

impl ArtifactResourcePlan {
    /// Returns the validated artifact identity carried by this plan.
    #[must_use]
    pub const fn artifact(&self) -> &SpirvArtifactIdentity {
        &self.artifact
    }

    /// Returns bindings in the artifact's stable declaration order.
    #[must_use]
    pub fn bindings(&self) -> &[ArtifactResourceBinding] {
        &self.bindings
    }
}

/// A live residency/access scope for one artifact dispatch.
#[derive(Debug, Eq, PartialEq)]
pub struct ArtifactResourceLease {
    plan: ArtifactResourcePlan,
    tokens: Vec<AccessToken>,
}

impl ArtifactResourceLease {
    /// Returns the immutable plan covered by this lease.
    #[must_use]
    pub const fn plan(&self) -> &ArtifactResourcePlan {
        &self.plan
    }

    /// Returns the number of live access tokens held by the lease.
    #[must_use]
    pub const fn token_count(&self) -> usize {
        self.tokens.len()
    }
}

/// Why an artifact resource binding plan was rejected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactResourcePlanError {
    /// Artifact metadata or words failed the shared contract.
    Artifact(SpirvArtifactContractError),
    /// The host supplied a different number of bindings than the artifact.
    BindingCountMismatch { expected: usize, actual: usize },
    /// Binding requests must preserve dense artifact declaration order.
    NonCanonicalBinding { expected: u32, actual: u32 },
    /// Zero-sized requirements are not a meaningful GPU descriptor contract.
    InvalidRequiredBytes { binding: u32 },
    /// The selected host buffer cannot hold the required bytes.
    BufferTooSmall {
        binding: u32,
        required: u64,
        actual: u64,
    },
    /// A single writable buffer cannot be aliased by two live bindings.
    ConflictingAlias {
        buffer: BufferId,
        first_binding: u32,
        second_binding: u32,
    },
    /// The host resource table rejected the buffer lookup.
    Resource(ResourceError),
}

impl fmt::Display for ArtifactResourcePlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Artifact(error) => write!(formatter, "resource artifact rejected: {error}"),
            Self::BindingCountMismatch { expected, actual } => write!(
                formatter,
                "artifact requires {expected} resource bindings, host supplied {actual}"
            ),
            Self::NonCanonicalBinding { expected, actual } => write!(
                formatter,
                "resource binding is not dense: expected {expected}, got {actual}"
            ),
            Self::InvalidRequiredBytes { binding } => {
                write!(
                    formatter,
                    "resource binding {binding} requires positive byte capacity"
                )
            }
            Self::BufferTooSmall {
                binding,
                required,
                actual,
            } => write!(
                formatter,
                "resource binding {binding} requires {required} bytes, buffer has {actual}"
            ),
            Self::ConflictingAlias {
                buffer,
                first_binding,
                second_binding,
            } => write!(
                formatter,
                "buffer {} has conflicting writable aliases at bindings {} and {}",
                buffer.value(),
                first_binding,
                second_binding
            ),
            Self::Resource(error) => write!(formatter, "resource lookup failed: {error}"),
        }
    }
}

impl Error for ArtifactResourcePlanError {}

/// Builds a deterministic resource binding plan for a validated artifact.
///
/// This is the portable descriptor/resource boundary for native adapters. It
/// validates the shared artifact contract, dense binding order, byte capacity
/// and write-alias hazards. It does not require residency and never touches a
/// native GPU API; residency and access lifetime are handled by the explicit
/// lease methods on [`ResourceTable`].
pub fn plan_artifact_resources(
    table: &ResourceTable,
    artifact: &SpirvArtifact,
    requests: &[ArtifactResourceRequest],
) -> Result<ArtifactResourcePlan, ArtifactResourcePlanError> {
    let identity =
        validate_spirv_artifact_contract(artifact).map_err(ArtifactResourcePlanError::Artifact)?;
    let capabilities = artifact_resource_capability_matrix(artifact)
        .map_err(ArtifactResourcePlanError::Artifact)?;
    if requests.len() != artifact.resources.len() {
        return Err(ArtifactResourcePlanError::BindingCountMismatch {
            expected: artifact.resources.len(),
            actual: requests.len(),
        });
    }

    let mut bindings = Vec::with_capacity(requests.len());
    for (index, request) in requests.iter().enumerate() {
        let expected = u32::try_from(index).expect("artifact binding count fits u32");
        if request.binding != expected {
            return Err(ArtifactResourcePlanError::NonCanonicalBinding {
                expected,
                actual: request.binding,
            });
        }
        if request.required_bytes == 0 {
            return Err(ArtifactResourcePlanError::InvalidRequiredBytes {
                binding: request.binding,
            });
        }
        let info = table
            .info(request.buffer)
            .map_err(ArtifactResourcePlanError::Resource)?;
        if info.size < request.required_bytes {
            return Err(ArtifactResourcePlanError::BufferTooSmall {
                binding: request.binding,
                required: request.required_bytes,
                actual: info.size,
            });
        }
        let access = match artifact.resources[index].access {
            ResourceAccess::ReadOnly => AccessKind::Read,
            ResourceAccess::WriteOnly => AccessKind::Write,
            ResourceAccess::ReadWrite => AccessKind::ReadWrite,
        };
        if let Some(previous) = bindings
            .iter()
            .find(|binding: &&ArtifactResourceBinding| binding.buffer == request.buffer)
            && previous.access.conflicts(access)
        {
            return Err(ArtifactResourcePlanError::ConflictingAlias {
                buffer: request.buffer,
                first_binding: previous.binding,
                second_binding: request.binding,
            });
        }
        bindings.push(ArtifactResourceBinding {
            binding: request.binding,
            descriptor_set: artifact.resources[index].descriptor_set,
            address_space: capabilities[index].address_space,
            element_type_info: artifact.resources[index].element_type_info,
            layout: capabilities[index].layout,
            buffer: request.buffer,
            access,
            required_bytes: request.required_bytes,
        });
    }

    Ok(ArtifactResourcePlan {
        artifact: identity,
        bindings,
    })
}

/// Why an artifact route was rejected before native API work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactDispatchPlanError {
    /// Artifact metadata or words failed the shared contract.
    Artifact(SpirvArtifactContractError),
    /// Backend capability gate rejected the route.
    Backend(BackendPlanError),
}

impl fmt::Display for ArtifactDispatchPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Artifact(error) => write!(formatter, "artifact route rejected: {error}"),
            Self::Backend(error) => write!(formatter, "backend route rejected: {error}"),
        }
    }
}

impl Error for ArtifactDispatchPlanError {}

/// Routes a validated artifact through the existing capability-only backend
/// planner. This function selects transport/completion metadata only; it does
/// not load a native API, allocate a resource, translate a shader, or submit
/// work.
pub fn plan_artifact_dispatch(
    backend: GpuBackend,
    probe: BackendProbe,
    request: ArtifactDispatchRequest,
    artifact: &SpirvArtifact,
) -> Result<ArtifactDispatchPlan, ArtifactDispatchPlanError> {
    let identity =
        validate_spirv_artifact_contract(artifact).map_err(ArtifactDispatchPlanError::Artifact)?;
    let backend_request = BackendRequest {
        fp: request.fp,
        workgroup_size: identity.workgroup_size[0],
        require_bounded_global_u32_array: request.require_bounded_global_u32_array,
        require_async_completion: request.require_async_completion,
    };
    let backend_plan = plan_backend(backend, probe, backend_request)
        .map_err(ArtifactDispatchPlanError::Backend)?;
    let product = u64::from(identity.workgroup_size[0])
        .checked_mul(u64::from(identity.workgroup_size[1]))
        .and_then(|product| product.checked_mul(u64::from(identity.workgroup_size[2])))
        .unwrap_or(u64::MAX);
    if product > u64::from(probe.max_workgroup_size) {
        return Err(ArtifactDispatchPlanError::Backend(
            BackendPlanError::WorkgroupSizeUnsupported,
        ));
    }
    Ok(ArtifactDispatchPlan {
        backend: backend_plan,
        artifact: identity,
    })
}

/// Owned descriptor metadata passed from the portable planner to a native
/// backend encoder.
///
/// This object contains no API handle and no translated shader bytes. It is a
/// stable data contract for entry/workgroup/grid, route completion semantics
/// and resource-to-buffer binding order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactDispatchDescriptor {
    /// Shared identity of the validated artifact.
    pub artifact: SpirvArtifactIdentity,
    /// Backend family selected by capability probing.
    pub backend: GpuBackend,
    /// Shader transport expected by the backend adapter.
    pub shader_transport: ShaderTransport,
    /// Completion primitive expected by the backend adapter.
    pub completion: CompletionModel,
    /// Local workgroup dimensions from the artifact.
    pub workgroup_size: [u32; 3],
    /// Number of local workgroups to dispatch in x/y/z.
    pub workgroups: [u32; 3],
    /// Checked total invocation count for this dispatch.
    pub invocation_count: u64,
    /// Resource bindings in dense artifact declaration order.
    pub resources: Vec<ArtifactResourceBinding>,
    /// Source translation input for translated backends, or `None` for native
    /// SPIR-V submission.
    pub source_translation: Option<ArtifactSourceTranslationPlan>,
}

/// Why a prepared descriptor's source contract is internally inconsistent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactDispatchDescriptorError {
    /// Backend and selected shader transport do not form a known route.
    BackendTransportMismatch {
        backend: GpuBackend,
        transport: ShaderTransport,
    },
    /// A translated backend omitted its source translation plan.
    MissingSourceTranslation { backend: GpuBackend },
    /// Native SPIR-V unexpectedly carried a source translation plan.
    UnexpectedSourceTranslation { backend: GpuBackend },
    /// The source plan names a different source backend than the route.
    SourceBackendMismatch {
        expected: ArtifactSourceBackend,
        actual: ArtifactSourceBackend,
    },
    /// Source plan and descriptor refer to different artifacts.
    ArtifactIdentityMismatch,
    /// Source plan and descriptor expose different binding counts.
    ResourceCountMismatch { expected: usize, actual: usize },
    /// A source capability differs from its prepared descriptor binding.
    ResourceCapabilityMismatch { binding: u32 },
    /// The descriptor completion primitive does not match the backend route.
    CompletionMismatch {
        expected: CompletionModel,
        actual: CompletionModel,
    },
    /// Descriptor local workgroup dimensions differ from the artifact.
    WorkgroupSizeMismatch {
        expected: [u32; 3],
        actual: [u32; 3],
    },
    /// A dispatch dimension is zero and therefore cannot represent work.
    ZeroDispatchWorkgroup([u32; 3]),
    /// The descriptor invocation count differs from checked geometry.
    InvocationCountMismatch { expected: u64, actual: u64 },
    /// Resource bindings must remain dense and declaration ordered.
    ResourceBindingOrderMismatch { expected: u32, actual: u32 },
}

impl fmt::Display for ArtifactDispatchDescriptorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BackendTransportMismatch { backend, transport } => write!(
                formatter,
                "backend {backend:?} has incompatible shader transport {transport:?}"
            ),
            Self::MissingSourceTranslation { backend } => {
                write!(
                    formatter,
                    "translated backend {backend:?} is missing source plan"
                )
            }
            Self::UnexpectedSourceTranslation { backend } => write!(
                formatter,
                "native SPIR-V backend {backend:?} unexpectedly has source plan"
            ),
            Self::SourceBackendMismatch { expected, actual } => write!(
                formatter,
                "source plan backend {actual:?} does not match expected {expected:?}"
            ),
            Self::ArtifactIdentityMismatch => {
                formatter.write_str("descriptor and source plan refer to different artifacts")
            }
            Self::ResourceCountMismatch { expected, actual } => write!(
                formatter,
                "descriptor has {actual} resources but source plan has {expected}"
            ),
            Self::ResourceCapabilityMismatch { binding } => write!(
                formatter,
                "descriptor resource binding {binding} differs from source capability"
            ),
            Self::CompletionMismatch { expected, actual } => write!(
                formatter,
                "descriptor completion {actual:?} does not match backend completion {expected:?}"
            ),
            Self::WorkgroupSizeMismatch { expected, actual } => write!(
                formatter,
                "descriptor workgroup size {actual:?} differs from artifact {expected:?}"
            ),
            Self::ZeroDispatchWorkgroup(workgroups) => {
                write!(
                    formatter,
                    "descriptor dispatch workgroups must be non-zero: {workgroups:?}"
                )
            }
            Self::InvocationCountMismatch { expected, actual } => write!(
                formatter,
                "descriptor invocation count {actual} differs from checked geometry {expected}"
            ),
            Self::ResourceBindingOrderMismatch { expected, actual } => write!(
                formatter,
                "descriptor resource binding {actual} is not dense; expected {expected}"
            ),
        }
    }
}

impl Error for ArtifactDispatchDescriptorError {}

impl ArtifactDispatchDescriptor {
    /// Validates backend/source-plan/resource capability consistency before a
    /// native encoder receives this descriptor.
    pub fn validate_source_translation(&self) -> Result<(), ArtifactDispatchDescriptorError> {
        let route = ShaderTranslationRoute::for_backend(self.backend);
        if self.shader_transport != route.transport {
            return Err(ArtifactDispatchDescriptorError::BackendTransportMismatch {
                backend: self.backend,
                transport: self.shader_transport,
            });
        }
        if let Some(expected_source_backend) = route.source_backend {
            let Some(plan) = self.source_translation.as_ref() else {
                return Err(ArtifactDispatchDescriptorError::MissingSourceTranslation {
                    backend: self.backend,
                });
            };
            if plan.backend != expected_source_backend {
                return Err(ArtifactDispatchDescriptorError::SourceBackendMismatch {
                    expected: expected_source_backend,
                    actual: plan.backend,
                });
            }
            if plan.artifact != self.artifact {
                return Err(ArtifactDispatchDescriptorError::ArtifactIdentityMismatch);
            }
            if plan.resources.len() != self.resources.len() {
                return Err(ArtifactDispatchDescriptorError::ResourceCountMismatch {
                    expected: plan.resources.len(),
                    actual: self.resources.len(),
                });
            }
            for (capability, binding) in plan.resources.iter().zip(&self.resources) {
                let access_matches = matches!(
                    (capability.access, binding.access),
                    (ResourceAccess::ReadOnly, AccessKind::Read)
                        | (ResourceAccess::WriteOnly, AccessKind::Write)
                        | (ResourceAccess::ReadWrite, AccessKind::ReadWrite)
                );
                if capability.binding != binding.binding
                    || capability.descriptor_set != binding.descriptor_set
                    || capability.address_space != binding.address_space
                    || capability.layout != binding.layout
                    || !access_matches
                {
                    return Err(
                        ArtifactDispatchDescriptorError::ResourceCapabilityMismatch {
                            binding: capability.binding,
                        },
                    );
                }
            }
        } else if self.source_translation.is_some() {
            return Err(
                ArtifactDispatchDescriptorError::UnexpectedSourceTranslation {
                    backend: self.backend,
                },
            );
        }

        let expected_completion = match self.backend {
            GpuBackend::Vulkan => CompletionModel::Fence,
            GpuBackend::DirectX12 => CompletionModel::TimelineFence,
            GpuBackend::Metal => CompletionModel::CommandBufferCompletion,
        };
        if self.completion != expected_completion {
            return Err(ArtifactDispatchDescriptorError::CompletionMismatch {
                expected: expected_completion,
                actual: self.completion,
            });
        }
        if self.workgroup_size != self.artifact.workgroup_size {
            return Err(ArtifactDispatchDescriptorError::WorkgroupSizeMismatch {
                expected: self.artifact.workgroup_size,
                actual: self.workgroup_size,
            });
        }
        if self.workgroups.contains(&0) {
            return Err(ArtifactDispatchDescriptorError::ZeroDispatchWorkgroup(
                self.workgroups,
            ));
        }
        let expected_invocation_count = self
            .workgroup_size
            .into_iter()
            .chain(self.workgroups)
            .try_fold(1_u64, |product, dimension| {
                product.checked_mul(u64::from(dimension))
            })
            .unwrap_or(u64::MAX);
        if self.invocation_count != expected_invocation_count {
            return Err(ArtifactDispatchDescriptorError::InvocationCountMismatch {
                expected: expected_invocation_count,
                actual: self.invocation_count,
            });
        }
        if self.resources.len() != self.artifact.resource_binding_count {
            return Err(ArtifactDispatchDescriptorError::ResourceCountMismatch {
                expected: self.artifact.resource_binding_count,
                actual: self.resources.len(),
            });
        }
        for (expected, binding) in self.resources.iter().enumerate() {
            let expected = u32::try_from(expected).unwrap_or(u32::MAX);
            if binding.binding != expected {
                return Err(
                    ArtifactDispatchDescriptorError::ResourceBindingOrderMismatch {
                        expected,
                        actual: binding.binding,
                    },
                );
            }
        }
        Ok(())
    }
}

/// Backend-neutral dispatch identity used for cross-device metadata audits.
///
/// The fingerprint deliberately excludes native handles and host buffer
/// identities. It captures only the portable artifact, dispatch geometry and
/// resource capabilities that must remain stable when the same kernel moves
/// between Vulkan, DirectX 12 and Metal adapters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactDispatchFingerprint {
    /// Shared identity of the validated SPIR-V artifact.
    pub artifact: SpirvArtifactIdentity,
    /// Number of workgroups submitted in x/y/z order.
    pub workgroups: [u32; 3],
    /// Checked total invocation count represented by the dispatch.
    pub invocation_count: u64,
    /// Resource capabilities preserved by the descriptor boundary.
    pub resources: Vec<ArtifactResourceCapability>,
}

impl ArtifactDispatchDescriptor {
    /// Creates a backend-neutral fingerprint after descriptor validation.
    pub fn fingerprint(
        &self,
    ) -> Result<ArtifactDispatchFingerprint, ArtifactDispatchDescriptorError> {
        self.validate_source_translation()?;
        let resources = self
            .resources
            .iter()
            .map(|binding| ArtifactResourceCapability {
                binding: binding.binding,
                descriptor_set: binding.descriptor_set,
                address_space: binding.address_space,
                access: match binding.access {
                    AccessKind::Read => ResourceAccess::ReadOnly,
                    AccessKind::Write => ResourceAccess::WriteOnly,
                    AccessKind::ReadWrite => ResourceAccess::ReadWrite,
                },
                layout: binding.layout,
            })
            .collect();
        Ok(ArtifactDispatchFingerprint {
            artifact: self.artifact.clone(),
            workgroups: self.workgroups,
            invocation_count: self.invocation_count,
            resources,
        })
    }
}

/// Metadata-only parity report for one dispatch represented on two or more
/// backend routes. A successful report proves descriptor identity only; it is
/// not a claim that any listed backend compiled or executed the shader.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactDispatchParityReport {
    /// Shared portable dispatch fingerprint.
    pub fingerprint: ArtifactDispatchFingerprint,
    /// Backend families represented by the descriptors in input order.
    pub backends: Vec<GpuBackend>,
    /// Shader transports represented by the descriptors in input order.
    pub transports: Vec<ShaderTransport>,
}

/// Why dispatch descriptors cannot form one cross-device parity report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactDispatchParityError {
    /// A parity report needs at least two backend descriptors.
    InsufficientDescriptors { actual: usize },
    /// One descriptor failed its own backend/source contract.
    InvalidDescriptor {
        index: usize,
        error: ArtifactDispatchDescriptorError,
    },
    /// Descriptors refer to different portable artifacts.
    ArtifactIdentityMismatch,
    /// Descriptors use different workgroup geometry.
    WorkgroupMismatch,
    /// Descriptors represent different invocation counts.
    InvocationCountMismatch,
    /// Descriptors expose different portable resource capabilities.
    ResourceCapabilityMismatch,
    /// The same backend was supplied more than once.
    DuplicateBackend(GpuBackend),
}

impl fmt::Display for ArtifactDispatchParityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InsufficientDescriptors { actual } => write!(
                formatter,
                "dispatch parity requires at least two descriptors, got {actual}"
            ),
            Self::InvalidDescriptor { index, error } => {
                write!(formatter, "dispatch descriptor {index} is invalid: {error}")
            }
            Self::ArtifactIdentityMismatch => {
                formatter.write_str("dispatch descriptors refer to different artifacts")
            }
            Self::WorkgroupMismatch => {
                formatter.write_str("dispatch descriptors use different workgroup geometry")
            }
            Self::InvocationCountMismatch => {
                formatter.write_str("dispatch descriptors use different invocation counts")
            }
            Self::ResourceCapabilityMismatch => {
                formatter.write_str("dispatch descriptors expose different resource capabilities")
            }
            Self::DuplicateBackend(backend) => {
                write!(formatter, "dispatch parity repeats backend {backend:?}")
            }
        }
    }
}

impl Error for ArtifactDispatchParityError {}

/// Compares portable dispatch metadata across backend descriptors.
///
/// This is intentionally separate from native execution reports. It catches
/// route drift before API calls while leaving device availability, compilation,
/// queue completion and numeric readback to their backend-specific gates.
pub fn compare_artifact_dispatch_descriptors(
    descriptors: &[ArtifactDispatchDescriptor],
) -> Result<ArtifactDispatchParityReport, ArtifactDispatchParityError> {
    if descriptors.len() < 2 {
        return Err(ArtifactDispatchParityError::InsufficientDescriptors {
            actual: descriptors.len(),
        });
    }
    let first = &descriptors[0];
    let first_fingerprint = first
        .fingerprint()
        .map_err(|error| ArtifactDispatchParityError::InvalidDescriptor { index: 0, error })?;
    let mut backends = Vec::with_capacity(descriptors.len());
    let mut transports = Vec::with_capacity(descriptors.len());
    for (index, descriptor) in descriptors.iter().enumerate() {
        let fingerprint = descriptor
            .fingerprint()
            .map_err(|error| ArtifactDispatchParityError::InvalidDescriptor { index, error })?;
        if fingerprint.artifact != first_fingerprint.artifact {
            return Err(ArtifactDispatchParityError::ArtifactIdentityMismatch);
        }
        if fingerprint.workgroups != first_fingerprint.workgroups {
            return Err(ArtifactDispatchParityError::WorkgroupMismatch);
        }
        if fingerprint.invocation_count != first_fingerprint.invocation_count {
            return Err(ArtifactDispatchParityError::InvocationCountMismatch);
        }
        if fingerprint.resources != first_fingerprint.resources {
            return Err(ArtifactDispatchParityError::ResourceCapabilityMismatch);
        }
        if backends.contains(&descriptor.backend) {
            return Err(ArtifactDispatchParityError::DuplicateBackend(
                descriptor.backend,
            ));
        }
        backends.push(descriptor.backend);
        transports.push(descriptor.shader_transport);
    }
    Ok(ArtifactDispatchParityReport {
        fingerprint: first_fingerprint,
        backends,
        transports,
    })
}

/// One fully prepared, capability-gated artifact dispatch scope.
///
/// The scope joins backend transport/completion metadata, the deterministic
/// resource map and the live access lease. It still contains no native API
/// handle and does not encode or submit a shader. The owner must release it
/// through [`ResourceTable::release_prepared_artifact_dispatch`] after the
/// backend completion event.
#[derive(Debug, Eq, PartialEq)]
pub struct PreparedArtifactDispatch {
    route: ArtifactDispatchPlan,
    resources: ArtifactResourcePlan,
    descriptor: ArtifactDispatchDescriptor,
    lease: ArtifactResourceLease,
}

impl PreparedArtifactDispatch {
    /// Returns the capability-gated backend route.
    #[must_use]
    pub const fn route(&self) -> &ArtifactDispatchPlan {
        &self.route
    }

    /// Returns the deterministic resource map used by this scope.
    #[must_use]
    pub const fn resources(&self) -> &ArtifactResourcePlan {
        &self.resources
    }

    /// Returns the owned descriptor metadata for a native encoder.
    #[must_use]
    pub const fn descriptor(&self) -> &ArtifactDispatchDescriptor {
        &self.descriptor
    }

    /// Returns the live access lease held until completion.
    #[must_use]
    pub const fn lease(&self) -> &ArtifactResourceLease {
        &self.lease
    }
}

/// Why a complete artifact dispatch scope could not be prepared.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreparedArtifactDispatchError {
    /// Capability or shared artifact route failed.
    Dispatch(ArtifactDispatchPlanError),
    /// Resource metadata/capacity validation failed.
    Resources(ArtifactResourcePlanError),
    /// Dispatch geometry cannot be represented safely.
    Geometry(DispatchGeometryError),
    /// Residency/access acquisition failed; no partial lease is retained.
    Resource(ResourceError),
    /// Route and resource planners produced different artifact identities.
    ArtifactIdentityMismatch,
    /// The descriptor could not preserve backend/source/resource consistency.
    Descriptor(ArtifactDispatchDescriptorError),
}

impl fmt::Display for PreparedArtifactDispatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dispatch(error) => write!(formatter, "dispatch preparation rejected: {error}"),
            Self::Resources(error) => write!(formatter, "resource preparation rejected: {error}"),
            Self::Geometry(error) => write!(formatter, "geometry preparation rejected: {error}"),
            Self::Resource(error) => write!(formatter, "resource acquisition rejected: {error}"),
            Self::ArtifactIdentityMismatch => {
                formatter.write_str("dispatch and resource plans refer to different artifacts")
            }
            Self::Descriptor(error) => write!(formatter, "descriptor rejected: {error}"),
        }
    }
}

impl Error for PreparedArtifactDispatchError {}

/// Prepares one complete artifact dispatch scope without touching a native API.
///
/// The route is selected first, resource requests are validated against the
/// same artifact identity, and all access tokens are acquired transactionally.
/// If any stage fails, the caller receives no prepared scope and no partial
/// resource lease remains live.
pub fn prepare_artifact_dispatch(
    table: &mut ResourceTable,
    backend: GpuBackend,
    probe: BackendProbe,
    dispatch_request: ArtifactDispatchRequest,
    geometry: DispatchGeometry,
    resource_requests: &[ArtifactResourceRequest],
    artifact: &SpirvArtifact,
) -> Result<PreparedArtifactDispatch, PreparedArtifactDispatchError> {
    let route = plan_artifact_dispatch(backend, probe, dispatch_request, artifact)
        .map_err(PreparedArtifactDispatchError::Dispatch)?;
    let invocation_count = geometry
        .invocation_count(route.artifact.workgroup_size)
        .map_err(PreparedArtifactDispatchError::Geometry)?;
    let resources = plan_artifact_resources(table, artifact, resource_requests)
        .map_err(PreparedArtifactDispatchError::Resources)?;
    if route.artifact != *resources.artifact() {
        return Err(PreparedArtifactDispatchError::ArtifactIdentityMismatch);
    }
    let source_translation =
        match ShaderTranslationRoute::for_backend(route.backend.backend).source_backend {
            Some(source_backend) => Some(
                ArtifactSourceTranslationPlan::from_artifact(artifact, source_backend).map_err(
                    |error| {
                        PreparedArtifactDispatchError::Dispatch(
                            ArtifactDispatchPlanError::Artifact(error),
                        )
                    },
                )?,
            ),
            None => None,
        };
    let descriptor = ArtifactDispatchDescriptor {
        artifact: route.artifact.clone(),
        backend: route.backend.backend,
        shader_transport: route.backend.shader_transport,
        completion: route.backend.completion,
        workgroup_size: route.artifact.workgroup_size,
        workgroups: geometry.workgroups(),
        invocation_count,
        resources: resources.bindings().to_vec(),
        source_translation,
    };
    descriptor
        .validate_source_translation()
        .map_err(PreparedArtifactDispatchError::Descriptor)?;
    let lease = table
        .acquire_artifact_resources(&resources)
        .map_err(PreparedArtifactDispatchError::Resource)?;
    Ok(PreparedArtifactDispatch {
        route,
        resources,
        descriptor,
        lease,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_artifact() -> SpirvArtifact {
        let options = jadren_codegen_spirv::SpirvOptions::new([8, 2, 1]).unwrap();
        SpirvArtifact {
            entry_name: "add_u32".to_owned(),
            workgroup_size: [8, 2, 1],
            resources: Vec::new(),
            words: jadren_codegen_spirv::emit_storage_add("add_u32", options, 1).unwrap(),
        }
    }

    fn resource_artifact(accesses: &[ResourceAccess]) -> SpirvArtifact {
        let mut artifact = valid_artifact();
        artifact.resources = accesses
            .iter()
            .enumerate()
            .map(|(index, &access)| jadren_codegen_spirv::ResourceBinding {
                binding: u32::try_from(index).unwrap(),
                descriptor_set: 0,
                name: format!("resource_{index}"),
                element_type: jadren_jir::TypeId::new(0),
                element_type_info: None,
                element_stride: Some(4),
                address_space: if access == ResourceAccess::ReadOnly {
                    jadren_jir::AddressSpace::Uniform
                } else {
                    jadren_jir::AddressSpace::Storage
                },
                access,
            })
            .collect();
        artifact
    }

    fn descriptor_for_backend(
        backend: GpuBackend,
        artifact: &SpirvArtifact,
    ) -> ArtifactDispatchDescriptor {
        let route = ShaderTranslationRoute::for_backend(backend);
        let source_translation = route.source_backend.map(|source_backend| {
            ArtifactSourceTranslationPlan::from_artifact(artifact, source_backend).unwrap()
        });
        ArtifactDispatchDescriptor {
            artifact: validate_spirv_artifact_contract(artifact).unwrap(),
            backend,
            shader_transport: route.transport,
            completion: match backend {
                GpuBackend::Vulkan => CompletionModel::Fence,
                GpuBackend::DirectX12 => CompletionModel::TimelineFence,
                GpuBackend::Metal => CompletionModel::CommandBufferCompletion,
            },
            workgroup_size: artifact.workgroup_size,
            workgroups: [2, 1, 1],
            invocation_count: 32,
            resources: Vec::new(),
            source_translation,
        }
    }

    #[test]
    fn artifact_contract_returns_stable_identity() {
        let artifact = valid_artifact();
        let identity = validate_spirv_artifact_contract(&artifact).unwrap();
        assert_eq!(identity.entry_name, "add_u32");
        assert_eq!(identity.workgroup_size, [8, 2, 1]);
        assert_eq!(identity.resource_binding_count, 0);
        assert_eq!(identity.word_count, artifact.words.len());
        assert_eq!(identity.word_hash, stable_spirv_word_hash(&artifact.words));
    }

    #[test]
    fn source_translation_report_carries_stable_identity() {
        let artifact = valid_artifact();
        let source = "kernel void add_u32() {}".to_owned();
        let report = ArtifactSourceTranslationReport::from_artifact(
            &artifact,
            ArtifactSourceBackend::Hlsl,
            source.clone(),
        )
        .unwrap();
        assert_eq!(report.backend, ArtifactSourceBackend::Hlsl);
        assert!(report.resources.is_empty());
        assert_eq!(report.artifact.entry_name, artifact.entry_name);
        assert_eq!(report.source, source);
        assert_eq!(report.source_byte_count, report.source.len());
        assert_eq!(report.source_hash, stable_source_hash(&report.source));
        assert_eq!(
            ArtifactSourceTranslationReport::from_artifact(
                &artifact,
                ArtifactSourceBackend::Hlsl,
                report.source.clone(),
            )
            .unwrap()
            .source_hash,
            report.source_hash
        );
        assert_eq!(
            ArtifactSourceTranslationReport::from_artifact(
                &artifact,
                ArtifactSourceBackend::Msl,
                "  ".to_owned(),
            ),
            Err(SpirvArtifactContractError::EmptySourceOutput)
        );
    }

    #[test]
    fn shared_spirv_cross_boundary_reports_missing_tool_without_shell_fallback() {
        let error = translate_spirv_with_spirv_cross(
            std::path::Path::new("__jadren_missing_spirv_cross__"),
            &[0x0723_0203, 1, 0, 1, 0],
            "add_u32",
            ArtifactSourceBackend::Hlsl,
        )
        .unwrap_err();
        assert!(matches!(error, SpirvCrossError::Io(_)));
        assert!(error.to_string().contains("SPIRV-Cross"));
    }

    #[test]
    fn raw_source_translation_report_rejects_invalid_input_before_tool_lookup() {
        assert_eq!(
            translate_spirv_source_report(
                &[],
                "compute_main",
                std::path::Path::new("__jadren_missing_spirv_cross__"),
                ArtifactSourceBackend::Msl,
            ),
            Err(SpirvSourceTranslationError::InvalidInput(
                "empty SPIR-V word stream"
            ))
        );
        assert_eq!(
            translate_spirv_source_report(
                &[0x0723_0203, 1, 0, 1, 0],
                "",
                std::path::Path::new("__jadren_missing_spirv_cross__"),
                ArtifactSourceBackend::Hlsl,
            ),
            Err(SpirvSourceTranslationError::InvalidInput(
                "entry name must be non-empty and NUL-free"
            ))
        );
    }

    #[test]
    fn raw_source_translation_report_preserves_tool_boundary_error() {
        let artifact = valid_artifact();
        let error = translate_spirv_source_report(
            &artifact.words,
            &artifact.entry_name,
            std::path::Path::new("__jadren_missing_spirv_cross__"),
            ArtifactSourceBackend::Msl,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            SpirvSourceTranslationError::Tool(SpirvCrossError::Io(_))
        ));
    }

    #[test]
    fn raw_source_translation_report_validates_module_and_selected_entry() {
        assert_eq!(
            translate_spirv_source_report(
                &[0x0723_0203, 0x0001_0000, 0, 1, 0, 0],
                "compute_main",
                std::path::Path::new("__jadren_missing_spirv_cross__"),
                ArtifactSourceBackend::Msl,
            ),
            Err(SpirvSourceTranslationError::InvalidSpirv(
                "instruction word count is out of bounds"
            ))
        );
        let artifact = valid_artifact();
        assert_eq!(
            translate_spirv_source_report(
                &[0x0723_0203, 1, 0, 1, 0],
                "compute_main",
                std::path::Path::new("__jadren_missing_spirv_cross__"),
                ArtifactSourceBackend::Msl,
            ),
            Err(SpirvSourceTranslationError::EntryPointNotFound(
                "compute_main".to_owned()
            ))
        );
        assert_eq!(
            translate_spirv_source_report(
                &artifact.words,
                "missing_entry",
                std::path::Path::new("__jadren_missing_spirv_cross__"),
                ArtifactSourceBackend::Hlsl,
            ),
            Err(SpirvSourceTranslationError::EntryPointNotFound(
                "missing_entry".to_owned()
            ))
        );
    }

    #[test]
    fn raw_spirv_metadata_reflects_descriptor_binding_and_set_decorations() {
        let words = [
            0x0723_0203,
            0x0001_0000,
            0,
            32,
            0,
            (4_u32 << 16) | 71,
            10,
            33,
            2,
            (4_u32 << 16) | 71,
            10,
            34,
            1,
            (4_u32 << 16) | 71,
            11,
            33,
            0,
            (4_u32 << 16) | 59,
            2,
            10,
            12,
            (4_u32 << 16) | 59,
            2,
            11,
            12,
            (7_u32 << 16) | 15,
            5,
            1,
            0x7478_6966,
            0x5f65_7275,
            0x6e69_616d,
            0,
            (4_u32 << 16) | 21,
            2,
            32,
            0,
        ];
        let metadata = validate_raw_spirv_entry(&words, "fixture_main").unwrap();
        assert_eq!(metadata.execution_model, 5);
        assert_eq!(
            metadata.resources,
            vec![
                SpirvRawResourceBinding {
                    variable_id: 11,
                    binding: 0,
                    descriptor_set: 0,
                    storage_class: Some(12),
                    element_type: Some(ResourceElementType::Integer {
                        signed: false,
                        bits: 32,
                        lanes: 1,
                    }),
                    element_stride: Some(4),
                    access: Some(ResourceAccess::ReadWrite),
                },
                SpirvRawResourceBinding {
                    variable_id: 10,
                    binding: 2,
                    descriptor_set: 1,
                    storage_class: Some(12),
                    element_type: Some(ResourceElementType::Integer {
                        signed: false,
                        bits: 32,
                        lanes: 1,
                    }),
                    element_stride: Some(4),
                    access: Some(ResourceAccess::ReadWrite),
                },
            ]
        );
        let contract = inspect_spirv_source_module(&words, "fixture_main").unwrap();
        assert_eq!(contract.entry_name, "fixture_main");
        assert_eq!(contract.execution_model, 5);
        assert_eq!(contract.word_count, words.len());
        assert_eq!(contract.resources, metadata.resources);
        let mut native_contract = contract.clone();
        native_contract.resources[1].binding = 1;
        native_contract.resources[1].descriptor_set = 0;
        let native_plan = validate_spirv_raw_native_adapter(&native_contract).unwrap();
        assert_eq!(native_plan.entry_name, "fixture_main");
        assert_eq!(native_plan.resources, native_contract.resources);
        let vulkan_plan = native_plan.project_backend(GpuBackend::Vulkan);
        assert_eq!(vulkan_plan.resources.len(), 2);
        assert!(
            vulkan_plan
                .resources
                .iter()
                .all(|resource| resource.view == SpirvRawNativeResourceView::VulkanStorageBuffer)
        );
        let dx12_plan = native_plan.project_backend(GpuBackend::DirectX12);
        assert!(dx12_plan.resources.iter().all(|resource| {
            resource.view == SpirvRawNativeResourceView::DirectX12UnorderedAccess
        }));
        let metal_plan = native_plan.project_backend(GpuBackend::Metal);
        assert!(
            metal_plan.resources.iter().all(|resource| {
                resource.view == SpirvRawNativeResourceView::MetalDevicePointer
            })
        );
        let mut mixed_contract = native_contract.clone();
        mixed_contract.resources[0].access = Some(ResourceAccess::ReadOnly);
        mixed_contract.resources[1].access = Some(ResourceAccess::ReadWrite);
        let mixed_plan = validate_spirv_raw_native_adapter(&mixed_contract).unwrap();
        let mixed_dx12 = mixed_plan.project_backend(GpuBackend::DirectX12);
        assert_eq!(
            mixed_dx12
                .resources
                .iter()
                .map(|resource| resource.view)
                .collect::<Vec<_>>(),
            vec![
                SpirvRawNativeResourceView::DirectX12ShaderResource,
                SpirvRawNativeResourceView::DirectX12UnorderedAccess,
            ]
        );
        let mixed_metal = mixed_plan.project_backend(GpuBackend::Metal);
        assert_eq!(
            mixed_metal
                .resources
                .iter()
                .map(|resource| resource.view)
                .collect::<Vec<_>>(),
            vec![
                SpirvRawNativeResourceView::MetalConstDevicePointer,
                SpirvRawNativeResourceView::MetalDevicePointer,
            ]
        );
        let mut invalid_execution = native_contract.clone();
        invalid_execution.execution_model = 4;
        assert_eq!(
            validate_spirv_raw_native_adapter(&invalid_execution),
            Err(SpirvRawNativeAdapterError::UnsupportedExecutionModel { actual: 4 })
        );
        let mut invalid_type = native_contract;
        invalid_type.resources[0].element_type = None;
        assert_eq!(
            validate_spirv_raw_native_adapter(&invalid_type),
            Err(SpirvRawNativeAdapterError::MissingElementType { binding: 0 })
        );
    }

    #[test]
    fn raw_resource_access_and_explicit_output_selection_are_independent() {
        let options = jadren_codegen_spirv::SpirvOptions::new([64, 1, 1]).unwrap();
        let words = jadren_codegen_spirv::emit_storage_global_index_f32_binary_dynamic_length(
            "global_add_dynamic_f32",
            options,
            1.0_f32.to_bits(),
            jadren_codegen_spirv::F32ArithmeticOp::Add,
        )
        .unwrap();
        let contract = inspect_spirv_source_module(&words, "global_add_dynamic_f32").unwrap();
        assert_eq!(
            contract
                .resources
                .iter()
                .map(|resource| resource.access.unwrap())
                .collect::<Vec<_>>(),
            vec![
                ResourceAccess::ReadOnly,
                ResourceAccess::WriteOnly,
                ResourceAccess::ReadOnly,
            ]
        );
        let native = validate_spirv_raw_native_adapter(&contract).unwrap();
        assert_eq!(
            select_spirv_raw_output_binding(&native, 1),
            Ok(SpirvRawOutputSelection {
                binding: 1,
                resource_index: 1,
                access: ResourceAccess::WriteOnly,
            })
        );
        assert_eq!(
            select_spirv_raw_output_binding(&native, 0),
            Err(SpirvRawOutputSelectionError::NotWritable {
                binding: 0,
                access: ResourceAccess::ReadOnly,
            })
        );
        assert_eq!(
            select_spirv_raw_output_binding(&native, 9),
            Err(SpirvRawOutputSelectionError::MissingBinding { binding: 9 })
        );

        let mut contradictory = words;
        contradictory.splice(5..5, [(3_u32 << 16) | 71, 16, 24]);
        assert_eq!(
            inspect_spirv_source_module(&contradictory, "global_add_dynamic_f32"),
            Err(SpirvSourceTranslationError::InvalidSpirv(
                "resource cannot be both NonWritable and NonReadable"
            ))
        );
    }

    #[test]
    fn raw_source_contract_reflects_literal_local_workgroup_size() {
        let artifact = valid_artifact();
        let contract = inspect_spirv_source_module(&artifact.words, &artifact.entry_name).unwrap();
        assert_eq!(contract.workgroup_size, Some([8, 2, 1]));
        assert_eq!(contract.workgroup_size_ids, None);
        let native = validate_spirv_raw_native_adapter(&contract).unwrap();
        assert_eq!(native.workgroup_size, Some([8, 2, 1]));
        assert_eq!(native.workgroup_size_ids, None);
        assert_eq!(native.resolve_workgroup_size(None), Ok([8, 2, 1]));
        assert_eq!(
            native.resolve_workgroup_size_from_spec_map(&BTreeMap::new()),
            Ok([8, 2, 1])
        );
        assert_eq!(
            native.resolve_workgroup_size_from_spec_map(&BTreeMap::from([(77, 1)])),
            Err(SpirvRawNativeAdapterError::UnexpectedWorkgroupSpecialization)
        );
        assert_eq!(
            native.resolve_workgroup_size(Some(&SpirvRawWorkgroupSpecialization {
                ids: [1, 2, 3],
                spec_ids: None,
                values: [8, 2, 1],
            })),
            Err(SpirvRawNativeAdapterError::UnexpectedWorkgroupSpecialization)
        );
        assert_eq!(
            native.project_backend(GpuBackend::DirectX12).workgroup_size,
            Some([8, 2, 1])
        );
    }

    #[test]
    fn raw_source_contract_reflects_local_size_id_operands() {
        let artifact = valid_artifact();
        let mut words = artifact.words;
        let type_id = words[3];
        let x_id = type_id + 1;
        let y_id = type_id + 2;
        let z_id = type_id + 3;
        words[3] = type_id + 4;
        let mut offset = 5;
        while offset < words.len() {
            let word_count = (words[offset] >> 16) as usize;
            let opcode = (words[offset] & 0xffff) as u16;
            if opcode == 16 {
                words[offset] = (6_u32 << 16) | 331;
                words[offset + 2] = 38;
                words[offset + 3] = x_id;
                words[offset + 4] = y_id;
                words[offset + 5] = z_id;
                break;
            }
            offset += word_count;
        }
        let mut constants = vec![
            (4_u32 << 16) | 21,
            type_id,
            32,
            0,
            (4_u32 << 16) | 50,
            type_id,
            x_id,
            1,
            (4_u32 << 16) | 50,
            type_id,
            y_id,
            1,
            (6_u32 << 16) | 52,
            type_id,
            z_id,
            128,
            x_id,
            y_id,
            (4_u32 << 16) | 71,
            x_id,
            1,
            101,
            (4_u32 << 16) | 71,
            y_id,
            1,
            102,
            (4_u32 << 16) | 71,
            z_id,
            1,
            103,
        ];
        let mut insert_at = 5;
        while insert_at < words.len() {
            let word_count = (words[insert_at] >> 16) as usize;
            let opcode = (words[insert_at] & 0xffff) as u16;
            if opcode == 54 {
                words.splice(insert_at..insert_at, constants.drain(..));
                break;
            }
            insert_at += word_count;
        }
        let contract = inspect_spirv_source_module(&words, &artifact.entry_name).unwrap();
        assert_eq!(contract.workgroup_size, None);
        assert_eq!(contract.workgroup_size_ids, Some([x_id, y_id, z_id]));
        assert_eq!(contract.workgroup_size_spec_ids, Some([101, 102, 103]));
        let native = validate_spirv_raw_native_adapter(&contract).unwrap();
        assert_eq!(native.workgroup_size, None);
        assert_eq!(native.workgroup_size_ids, Some([x_id, y_id, z_id]));
        assert_eq!(native.workgroup_size_spec_ids, Some([101, 102, 103]));
        assert_eq!(
            native.resolve_workgroup_size(None),
            Err(SpirvRawNativeAdapterError::MissingWorkgroupSpecialization)
        );
        assert_eq!(
            native.resolve_workgroup_size(Some(&SpirvRawWorkgroupSpecialization {
                ids: [x_id, y_id, z_id],
                spec_ids: Some([101, 102, 103]),
                values: [8, 2, 1],
            })),
            Ok([8, 2, 1])
        );
        assert_eq!(
            native.resolve_workgroup_size(Some(&SpirvRawWorkgroupSpecialization {
                ids: [x_id, y_id, z_id],
                spec_ids: None,
                values: [8, 2, 1],
            })),
            Err(
                SpirvRawNativeAdapterError::WorkgroupSpecializationSpecIdsMismatch {
                    expected: Some([101, 102, 103]),
                    actual: None,
                }
            )
        );
        assert_eq!(
            native.resolve_workgroup_size(Some(&SpirvRawWorkgroupSpecialization {
                ids: [x_id, y_id, z_id],
                spec_ids: Some([101, 102, 104]),
                values: [8, 2, 1],
            })),
            Err(
                SpirvRawNativeAdapterError::WorkgroupSpecializationSpecIdsMismatch {
                    expected: Some([101, 102, 103]),
                    actual: Some([101, 102, 104]),
                }
            )
        );
        assert_eq!(
            native.resolve_workgroup_size(Some(&SpirvRawWorkgroupSpecialization {
                ids: [x_id, y_id, z_id + 1],
                spec_ids: Some([101, 102, 103]),
                values: [8, 2, 1],
            })),
            Err(
                SpirvRawNativeAdapterError::WorkgroupSpecializationIdsMismatch {
                    expected: [x_id, y_id, z_id],
                    actual: [x_id, y_id, z_id + 1],
                }
            )
        );
        assert_eq!(
            native.resolve_workgroup_size(Some(&SpirvRawWorkgroupSpecialization {
                ids: [x_id, y_id, z_id],
                spec_ids: Some([101, 102, 103]),
                values: [8, 0, 1],
            })),
            Err(SpirvRawNativeAdapterError::ZeroWorkgroupDimension { index: 1, value: 0 })
        );
        assert_eq!(
            native.project_backend(GpuBackend::Metal).workgroup_size_ids,
            Some([x_id, y_id, z_id])
        );
        assert_eq!(
            native
                .project_backend(GpuBackend::Metal)
                .workgroup_size_spec_ids,
            Some([101, 102, 103])
        );
        assert_eq!(
            native.resolve_workgroup_size_from_spec_map(&BTreeMap::from([
                (101, 8),
                (102, 2),
                (103, 1),
                (900, 17),
            ])),
            Ok([8, 2, 1])
        );
        assert_eq!(
            native.resolve_workgroup_size_from_spec_map(&BTreeMap::from([(101, 8), (103, 1)])),
            Err(SpirvRawNativeAdapterError::MissingWorkgroupSpecializationValue { spec_id: 102 })
        );
        assert_eq!(
            native.resolve_workgroup_size_from_spec_map(&BTreeMap::from([
                (101, 8),
                (102, 0),
                (103, 1),
            ])),
            Err(SpirvRawNativeAdapterError::ZeroWorkgroupDimension { index: 1, value: 0 })
        );
        assert_eq!(
            resolve_spirv_source_workgroup_size(
                &words,
                &artifact.entry_name,
                &BTreeMap::from([(101, 8), (102, 2)]),
            ),
            Ok([8, 2, 10])
        );
        assert_eq!(
            resolve_spirv_source_workgroup_size(
                &words,
                &artifact.entry_name,
                &BTreeMap::from([(101, 8), (102, 2), (103, 11)]),
            ),
            Ok([8, 2, 11])
        );
        assert_eq!(
            native.resolve_workgroup_size_from_spirv_words(
                &words,
                &artifact.entry_name,
                &BTreeMap::from([(101, 8), (102, 2)]),
            ),
            Ok([8, 2, 10])
        );
        assert_eq!(
            native
                .resolve_workgroup_size_from_spirv_words(&words, "wrong_entry", &BTreeMap::new(),),
            Err(SpirvRawNativeAdapterError::SpirvEntryMismatch {
                expected: artifact.entry_name.clone(),
                actual: "wrong_entry".to_owned(),
            })
        );
        let mut tampered_words = words.clone();
        tampered_words[10] ^= 1;
        assert_eq!(
            native.resolve_workgroup_size_from_spirv_words(
                &tampered_words,
                &artifact.entry_name,
                &BTreeMap::from([(101, 8), (102, 2)]),
            ),
            Err(SpirvRawNativeAdapterError::SpirvWordHashMismatch {
                expected: native.word_hash,
                actual: stable_spirv_word_hash(&tampered_words),
            })
        );
        assert_eq!(
            resolve_spirv_source_workgroup_size(
                &words,
                &artifact.entry_name,
                &BTreeMap::from([(101, 0), (102, 2)]),
            ),
            Err(SpirvRawWorkgroupEvaluationError::ZeroDimension { index: 0 })
        );
        let mut unsupported_words = words.clone();
        let mut unsupported_offset = 5;
        while unsupported_offset < unsupported_words.len() {
            let word_count = (unsupported_words[unsupported_offset] >> 16) as usize;
            let opcode = (unsupported_words[unsupported_offset] & 0xffff) as u16;
            if opcode == 52 {
                unsupported_words[unsupported_offset + 3] = 999;
                break;
            }
            unsupported_offset += word_count;
        }
        assert_eq!(
            resolve_spirv_source_workgroup_size(
                &unsupported_words,
                &artifact.entry_name,
                &BTreeMap::from([(101, 8), (102, 2)]),
            ),
            Err(SpirvRawWorkgroupEvaluationError::UnsupportedOperation { opcode: 999 })
        );
        let mut signed_negative_words = words.clone();
        let mut signed_offset = 5;
        while signed_offset < signed_negative_words.len() {
            let word_count = (signed_negative_words[signed_offset] >> 16) as usize;
            let opcode = (signed_negative_words[signed_offset] & 0xffff) as u16;
            if opcode == 21 && signed_negative_words[signed_offset + 1] == type_id {
                signed_negative_words[signed_offset + 3] = 1;
            }
            if opcode == 52 {
                signed_negative_words[signed_offset + 3] = 130;
                break;
            }
            signed_offset += word_count;
        }
        assert_eq!(
            resolve_spirv_source_workgroup_size(
                &signed_negative_words,
                &artifact.entry_name,
                &BTreeMap::from([(101, 2), (102, 8)]),
            ),
            Err(SpirvRawWorkgroupEvaluationError::NegativeDimension { index: 2 })
        );

        let mut partial_words = words.clone();
        let mut partial_offset = 5;
        while partial_offset < partial_words.len() {
            let word_count = (partial_words[partial_offset] >> 16) as usize;
            let opcode = (partial_words[partial_offset] & 0xffff) as u16;
            if opcode == 71 && partial_words[partial_offset + 1] == y_id {
                partial_words.drain(partial_offset..partial_offset + word_count);
                break;
            }
            partial_offset += word_count;
        }
        let partial_contract =
            inspect_spirv_source_module(&partial_words, &artifact.entry_name).unwrap();
        assert_eq!(partial_contract.workgroup_size_spec_ids, None);
        let partial_native = validate_spirv_raw_native_adapter(&partial_contract).unwrap();
        assert_eq!(
            partial_native.resolve_workgroup_size_from_spec_map(&BTreeMap::from([
                (101, 8),
                (102, 2),
                (103, 1),
            ])),
            Err(SpirvRawNativeAdapterError::MissingWorkgroupSpecializationSpecIds)
        );
        assert_eq!(
            partial_native.resolve_workgroup_size_from_spirv_words(
                &partial_words,
                &artifact.entry_name,
                &BTreeMap::from([(101, 8)]),
            ),
            Err(SpirvRawNativeAdapterError::MissingWorkgroupSpecializationSpecIds)
        );
        assert_eq!(
            partial_native.resolve_workgroup_size(Some(&SpirvRawWorkgroupSpecialization {
                ids: [x_id, y_id, z_id],
                spec_ids: None,
                values: [8, 2, 1],
            })),
            Ok([8, 2, 1])
        );
        let request = ArtifactDispatchRequest {
            fp: FpPolicy::Deterministic,
            require_bounded_global_u32_array: false,
            require_async_completion: true,
        };
        let dispatch = plan_spirv_raw_dispatch(
            GpuBackend::Vulkan,
            BackendProbe::prototype(GpuBackend::Vulkan),
            request,
            &words,
            &artifact.entry_name,
            &BTreeMap::from([(101, 8), (102, 2)]),
            [2, 3, 1],
        )
        .unwrap();
        assert_eq!(dispatch.backend.backend, GpuBackend::Vulkan);
        assert_eq!(dispatch.workgroup_size, [8, 2, 10]);
        assert_eq!(dispatch.workgroups, [2, 3, 1]);
        assert_eq!(dispatch.invocation_count, 960);
        assert_eq!(dispatch.adapter.backend, GpuBackend::Vulkan);
        assert_eq!(
            plan_spirv_raw_dispatch(
                GpuBackend::Vulkan,
                BackendProbe::prototype(GpuBackend::Vulkan),
                request,
                &words,
                &artifact.entry_name,
                &BTreeMap::from([(101, 8), (102, 2)]),
                [0, 1, 1],
            ),
            Err(SpirvRawDispatchPlanError::Geometry(
                DispatchGeometryError::ZeroDimension { axis: 0 }
            ))
        );
    }

    #[test]
    fn raw_source_contract_rejects_mixed_local_size_modes() {
        let artifact = valid_artifact();
        let mut words = artifact.words;
        let mut insert_at = 5;
        while insert_at < words.len() {
            let word_count = (words[insert_at] >> 16) as usize;
            let opcode = (words[insert_at] & 0xffff) as u16;
            if opcode == 16 {
                let mut local_size_id = Vec::new();
                local_size_id.extend([(6_u32 << 16) | 331, 1, 38, 12, 13, 14]);
                words.splice(insert_at..insert_at, local_size_id);
                break;
            }
            insert_at += word_count;
        }
        assert_eq!(
            inspect_spirv_source_module(&words, &artifact.entry_name),
            Err(SpirvSourceTranslationError::InvalidSpirv(
                "selected entry has both LocalSize and LocalSizeId"
            ))
        );
    }

    #[test]
    fn raw_source_contract_rejects_nonconstant_local_size_id() {
        let artifact = valid_artifact();
        let mut words = artifact.words;
        let mut offset = 5;
        while offset < words.len() {
            let word_count = (words[offset] >> 16) as usize;
            let opcode = (words[offset] & 0xffff) as u16;
            if opcode == 16 {
                words[offset] = (6_u32 << 16) | 331;
                words[offset + 2] = 38;
                words[offset + 3] = 12;
                words[offset + 4] = 13;
                words[offset + 5] = 14;
                break;
            }
            offset += word_count;
        }
        assert_eq!(
            inspect_spirv_source_module(&words, &artifact.entry_name),
            Err(SpirvSourceTranslationError::InvalidSpirv(
                "LocalSizeId operands are not scalar integer constants"
            ))
        );
    }

    #[test]
    fn raw_source_parity_compares_identity_and_backend_audits() {
        let report = |backend, source_hash| SpirvSourceTranslationReport {
            identity: SpirvSourceTranslationIdentity {
                backend,
                entry_name: "compute_main".to_owned(),
                execution_model: 5,
                workgroup_size: Some([1, 1, 1]),
                workgroup_size_ids: None,
                workgroup_size_spec_ids: None,
                resources: Vec::new(),
                word_count: 5,
                word_hash: 42,
            },
            source: format!("source-{source_hash}"),
            source_byte_count: 10,
            source_hash,
        };
        let parity = compare_spirv_source_reports(&[
            report(ArtifactSourceBackend::Hlsl, 11),
            report(ArtifactSourceBackend::Msl, 22),
        ])
        .unwrap();
        assert_eq!(parity.entry_name, "compute_main");
        assert_eq!(parity.execution_model, 5);
        assert!(parity.resources.is_empty());
        assert_eq!(parity.word_count, 5);
        assert_eq!(parity.word_hash, 42);
        assert_eq!(parity.sources.len(), 2);
        assert_eq!(parity.sources[0].source_hash, 11);
        assert_eq!(parity.sources[1].source_hash, 22);
    }

    #[test]
    fn raw_source_parity_rejects_insufficient_duplicate_and_mismatched_reports() {
        let report = |backend: ArtifactSourceBackend, entry_name: &str, word_hash: u64| {
            SpirvSourceTranslationReport {
                identity: SpirvSourceTranslationIdentity {
                    backend,
                    entry_name: entry_name.to_owned(),
                    execution_model: 5,
                    workgroup_size: Some([1, 1, 1]),
                    workgroup_size_ids: None,
                    workgroup_size_spec_ids: None,
                    resources: Vec::new(),
                    word_count: 5,
                    word_hash,
                },
                source: "source".to_owned(),
                source_byte_count: 6,
                source_hash: 7,
            }
        };
        assert_eq!(
            compare_spirv_source_reports(&[report(ArtifactSourceBackend::Msl, "main", 1)]),
            Err(SpirvSourceParityError::InsufficientReports { actual: 1 })
        );
        assert_eq!(
            compare_spirv_source_reports(&[
                report(ArtifactSourceBackend::Msl, "main", 1),
                report(ArtifactSourceBackend::Msl, "main", 1),
            ]),
            Err(SpirvSourceParityError::DuplicateBackend(
                ArtifactSourceBackend::Msl
            ))
        );
        assert_eq!(
            compare_spirv_source_reports(&[
                report(ArtifactSourceBackend::Msl, "main", 1),
                report(ArtifactSourceBackend::Hlsl, "other", 1),
            ]),
            Err(SpirvSourceParityError::ModuleIdentityMismatch)
        );
    }

    #[test]
    fn shared_spirv_cross_target_arguments_pin_entry_for_both_source_backends() {
        assert_eq!(
            spirv_cross_target_arguments("compute_main", ArtifactSourceBackend::Msl),
            vec![
                "--msl",
                "--entry",
                "compute_main",
                "--rename-entry-point",
                "compute_main",
                "compute_main",
                "comp",
                "--output"
            ]
        );
        assert_eq!(
            spirv_cross_target_arguments("compute_main", ArtifactSourceBackend::Hlsl),
            vec![
                "--hlsl",
                "--shader-model",
                "60",
                "--entry",
                "compute_main",
                "--rename-entry-point",
                "compute_main",
                "compute_main",
                "comp",
                "--output"
            ]
        );
    }

    #[test]
    fn generic_artifact_source_translation_rejects_contract_before_tool_lookup() {
        let mut artifact = valid_artifact();
        artifact.entry_name.clear();
        let error = translate_spirv_artifact_source(
            &artifact,
            std::path::Path::new("__jadren_missing_spirv_cross__"),
            ArtifactSourceBackend::Hlsl,
        )
        .unwrap_err();
        assert_eq!(
            error,
            ArtifactSourceTranslationError::Contract(SpirvArtifactContractError::EmptyEntryName)
        );
    }

    #[test]
    fn generic_artifact_source_translation_preserves_tool_boundary_error() {
        let error = translate_spirv_artifact_source(
            &valid_artifact(),
            std::path::Path::new("__jadren_missing_spirv_cross__"),
            ArtifactSourceBackend::Msl,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ArtifactSourceTranslationError::Tool(SpirvCrossError::Io(_))
        ));
    }

    #[test]
    fn source_translation_plan_snapshots_capabilities_before_toolchain() {
        let mut artifact =
            resource_artifact(&[ResourceAccess::ReadWrite, ResourceAccess::ReadOnly]);
        artifact.resources[0].element_type_info =
            Some(ResourceElementType::Float { bits: 32, lanes: 2 });
        artifact.resources[0].element_stride = Some(8);
        let plan =
            ArtifactSourceTranslationPlan::from_artifact(&artifact, ArtifactSourceBackend::Msl)
                .unwrap();

        assert_eq!(plan.backend, ArtifactSourceBackend::Msl);
        assert_eq!(plan.artifact.entry_name, artifact.entry_name);
        assert_eq!(plan.resources.len(), 2);
        assert_eq!(
            plan.resources[0].layout,
            ArtifactResourceLayout::ScalarVector {
                element: ResourceElementType::Float { bits: 32, lanes: 2 },
                stride: Some(8),
            }
        );
        assert_eq!(
            plan.resources[1].layout,
            ArtifactResourceLayout::Opaque { stride: Some(4) }
        );
        assert_eq!(plan.resources[1].address_space, AddressSpace::Uniform);
    }

    #[test]
    fn source_parity_report_compares_distinct_backends_for_same_artifact() {
        let artifact = valid_artifact();
        let hlsl = ArtifactSourceTranslationReport::from_artifact(
            &artifact,
            ArtifactSourceBackend::Hlsl,
            "[hlsl]".to_owned(),
        )
        .unwrap();
        let msl = ArtifactSourceTranslationReport::from_artifact(
            &artifact,
            ArtifactSourceBackend::Msl,
            "[msl]".to_owned(),
        )
        .unwrap();
        let parity = compare_artifact_source_reports(&[hlsl.clone(), msl.clone()]).unwrap();

        assert_eq!(parity.artifact, hlsl.artifact);
        assert!(parity.resources.is_empty());
        assert_eq!(
            parity.backends,
            vec![ArtifactSourceBackend::Hlsl, ArtifactSourceBackend::Msl]
        );
        assert_eq!(
            parity.source_hashes,
            vec![hlsl.source_hash, msl.source_hash]
        );
        assert_eq!(
            parity.source_byte_counts,
            vec![hlsl.source_byte_count, msl.source_byte_count]
        );
    }

    #[test]
    fn source_parity_report_rejects_mismatch_and_duplicate_backend() {
        let artifact = valid_artifact();
        let hlsl = ArtifactSourceTranslationReport::from_artifact(
            &artifact,
            ArtifactSourceBackend::Hlsl,
            "[hlsl]".to_owned(),
        )
        .unwrap();
        assert_eq!(
            compare_artifact_source_reports(std::slice::from_ref(&hlsl)),
            Err(ArtifactSourceParityError::InsufficientReports { actual: 1 })
        );
        let mut other_artifact = artifact.clone();
        other_artifact.entry_name = "different".to_owned();
        let other = ArtifactSourceTranslationReport::from_artifact(
            &other_artifact,
            ArtifactSourceBackend::Msl,
            "[msl]".to_owned(),
        )
        .unwrap();
        assert_eq!(
            compare_artifact_source_reports(&[hlsl.clone(), other]),
            Err(ArtifactSourceParityError::ArtifactIdentityMismatch)
        );
        assert_eq!(
            compare_artifact_source_reports(&[hlsl.clone(), hlsl]),
            Err(ArtifactSourceParityError::DuplicateBackend(
                ArtifactSourceBackend::Hlsl
            ))
        );
    }

    #[test]
    fn dispatch_parity_report_preserves_portable_identity_across_routes() {
        let artifact = valid_artifact();
        let vulkan = descriptor_for_backend(GpuBackend::Vulkan, &artifact);
        let directx12 = descriptor_for_backend(GpuBackend::DirectX12, &artifact);
        let parity = compare_artifact_dispatch_descriptors(&[vulkan, directx12]).unwrap();

        assert_eq!(parity.fingerprint.artifact.entry_name, "add_u32");
        assert_eq!(parity.fingerprint.workgroups, [2, 1, 1]);
        assert_eq!(parity.fingerprint.invocation_count, 32);
        assert!(parity.fingerprint.resources.is_empty());
        assert_eq!(
            parity.backends,
            vec![GpuBackend::Vulkan, GpuBackend::DirectX12]
        );
        assert_eq!(
            parity.transports,
            vec![ShaderTransport::NativeSpirv, ShaderTransport::SpirvToDxil]
        );
    }

    #[test]
    fn dispatch_parity_report_covers_all_portable_backend_routes() {
        let artifact = valid_artifact();
        let vulkan = descriptor_for_backend(GpuBackend::Vulkan, &artifact);
        let directx12 = descriptor_for_backend(GpuBackend::DirectX12, &artifact);
        let metal = descriptor_for_backend(GpuBackend::Metal, &artifact);
        let parity = compare_artifact_dispatch_descriptors(&[vulkan, directx12, metal]).unwrap();

        assert_eq!(
            parity.backends,
            vec![GpuBackend::Vulkan, GpuBackend::DirectX12, GpuBackend::Metal]
        );
        assert_eq!(
            parity.transports,
            vec![
                ShaderTransport::NativeSpirv,
                ShaderTransport::SpirvToDxil,
                ShaderTransport::Msl
            ]
        );
        assert_eq!(parity.fingerprint.workgroups, [2, 1, 1]);
        assert_eq!(parity.fingerprint.invocation_count, 32);
    }

    #[test]
    fn dispatch_parity_report_rejects_identity_and_duplicate_routes() {
        let artifact = valid_artifact();
        let vulkan = descriptor_for_backend(GpuBackend::Vulkan, &artifact);
        assert_eq!(
            compare_artifact_dispatch_descriptors(std::slice::from_ref(&vulkan)),
            Err(ArtifactDispatchParityError::InsufficientDescriptors { actual: 1 })
        );
        let mut other_artifact = valid_artifact();
        other_artifact.entry_name = "different".to_owned();
        let other = descriptor_for_backend(GpuBackend::Vulkan, &other_artifact);
        assert_eq!(
            compare_artifact_dispatch_descriptors(&[vulkan.clone(), other]),
            Err(ArtifactDispatchParityError::ArtifactIdentityMismatch)
        );
        let directx12 = descriptor_for_backend(GpuBackend::DirectX12, &artifact);
        assert_eq!(
            compare_artifact_dispatch_descriptors(&[vulkan.clone(), vulkan]),
            Err(ArtifactDispatchParityError::DuplicateBackend(
                GpuBackend::Vulkan
            ))
        );
        assert_eq!(
            compare_artifact_dispatch_descriptors(&[directx12.clone(), directx12]),
            Err(ArtifactDispatchParityError::DuplicateBackend(
                GpuBackend::DirectX12
            ))
        );
    }

    #[test]
    fn resource_capability_matrix_preserves_known_and_opaque_layouts() {
        let mut artifact =
            resource_artifact(&[ResourceAccess::ReadWrite, ResourceAccess::ReadOnly]);
        artifact.resources[0].element_type_info =
            Some(ResourceElementType::Float { bits: 32, lanes: 4 });
        artifact.resources[0].element_stride = Some(16);
        let capabilities = artifact_resource_capability_matrix(&artifact).unwrap();
        assert_eq!(capabilities.len(), 2);
        assert_eq!(capabilities[0].binding, 0);
        assert_eq!(capabilities[0].address_space, AddressSpace::Storage);
        assert_eq!(
            capabilities[0].layout,
            ArtifactResourceLayout::ScalarVector {
                element: ResourceElementType::Float { bits: 32, lanes: 4 },
                stride: Some(16),
            }
        );
        assert_eq!(
            capabilities[1].layout,
            ArtifactResourceLayout::Opaque { stride: Some(4) }
        );
        assert_eq!(capabilities[1].access, ResourceAccess::ReadOnly);
    }

    #[test]
    fn artifact_contract_rejects_noncanonical_binding_and_duplicate_name() {
        let mut artifact = valid_artifact();
        artifact.resources = vec![jadren_codegen_spirv::ResourceBinding {
            binding: 1,
            descriptor_set: 0,
            name: "data".to_owned(),
            element_type: jadren_jir::TypeId::new(0),
            element_type_info: None,
            element_stride: Some(4),
            address_space: jadren_jir::AddressSpace::Storage,
            access: jadren_codegen_spirv::ResourceAccess::ReadWrite,
        }];
        assert_eq!(
            validate_spirv_artifact_contract(&artifact),
            Err(SpirvArtifactContractError::NonCanonicalBinding {
                expected: 0,
                actual: 1,
            })
        );

        artifact.resources[0].binding = 0;
        artifact.resources.push(artifact.resources[0].clone());
        assert_eq!(
            validate_spirv_artifact_contract(&artifact),
            Err(SpirvArtifactContractError::NonCanonicalBinding {
                expected: 1,
                actual: 0,
            })
        );
        artifact.resources[1].binding = 1;
        assert_eq!(
            validate_spirv_artifact_contract(&artifact),
            Err(SpirvArtifactContractError::DuplicateResourceName(
                "data".to_owned()
            ))
        );
    }

    #[test]
    fn artifact_contract_rejects_invalid_stream_and_zero_workgroup() {
        let mut invalid_stream = valid_artifact();
        invalid_stream.words = vec![0];
        assert!(matches!(
            validate_spirv_artifact_contract(&invalid_stream),
            Err(SpirvArtifactContractError::InvalidSpirv(_))
        ));

        let mut zero_workgroup = valid_artifact();
        zero_workgroup.workgroup_size = [0, 1, 1];
        assert_eq!(
            validate_spirv_artifact_contract(&zero_workgroup),
            Err(SpirvArtifactContractError::ZeroWorkgroup([0, 1, 1]))
        );
    }

    #[test]
    fn artifact_contract_rejects_zero_resource_stride() {
        let mut artifact = valid_artifact();
        artifact.resources = vec![jadren_codegen_spirv::ResourceBinding {
            binding: 0,
            descriptor_set: 0,
            name: "data".to_owned(),
            element_type: jadren_jir::TypeId::new(0),
            element_type_info: None,
            element_stride: Some(0),
            address_space: jadren_jir::AddressSpace::Storage,
            access: jadren_codegen_spirv::ResourceAccess::ReadWrite,
        }];
        assert_eq!(
            validate_spirv_artifact_contract(&artifact),
            Err(SpirvArtifactContractError::InvalidResourceStride {
                binding: 0,
                stride: 0,
            })
        );
    }

    #[test]
    fn artifact_contract_rejects_inconsistent_resource_type_metadata() {
        let mut artifact = valid_artifact();
        artifact.resources = vec![jadren_codegen_spirv::ResourceBinding {
            binding: 0,
            descriptor_set: 0,
            name: "data".to_owned(),
            element_type: jadren_jir::TypeId::new(0),
            element_type_info: Some(jadren_codegen_spirv::ResourceElementType::Float {
                bits: 32,
                lanes: 4,
            }),
            element_stride: Some(4),
            address_space: jadren_jir::AddressSpace::Storage,
            access: jadren_codegen_spirv::ResourceAccess::ReadWrite,
        }];
        assert_eq!(
            validate_spirv_artifact_contract(&artifact),
            Err(SpirvArtifactContractError::ResourceTypeStrideMismatch {
                binding: 0,
                type_stride: 16,
                stride: 4,
            })
        );

        artifact.resources[0].element_stride = Some(16);
        artifact.resources[0].element_type_info =
            Some(jadren_codegen_spirv::ResourceElementType::Integer {
                signed: false,
                bits: 32,
                lanes: 0,
            });
        assert_eq!(
            validate_spirv_artifact_contract(&artifact),
            Err(SpirvArtifactContractError::InvalidResourceTypeMetadata { binding: 0 })
        );
    }

    #[test]
    fn artifact_dispatch_plan_preserves_identity_and_backend_route() {
        let artifact = valid_artifact();
        let plan = plan_artifact_dispatch(
            GpuBackend::Vulkan,
            BackendProbe::prototype(GpuBackend::Vulkan),
            ArtifactDispatchRequest {
                fp: FpPolicy::Strict,
                require_bounded_global_u32_array: false,
                require_async_completion: true,
            },
            &artifact,
        )
        .unwrap();
        assert_eq!(plan.backend.shader_transport, ShaderTransport::NativeSpirv);
        assert_eq!(plan.backend.completion, CompletionModel::Fence);
        assert_eq!(plan.artifact.entry_name, "add_u32");
        assert_eq!(
            plan.artifact.word_hash,
            stable_spirv_word_hash(&artifact.words)
        );
    }

    #[test]
    fn artifact_dispatch_plan_requires_dx12_translation_capability() {
        let artifact = valid_artifact();
        let request = ArtifactDispatchRequest {
            fp: FpPolicy::Strict,
            require_bounded_global_u32_array: false,
            require_async_completion: true,
        };
        assert_eq!(
            plan_artifact_dispatch(
                GpuBackend::DirectX12,
                BackendProbe::prototype(GpuBackend::DirectX12),
                request,
                &artifact,
            ),
            Err(ArtifactDispatchPlanError::Backend(
                BackendPlanError::DeviceUnavailable
            ))
        );
        let probe = BackendProbe {
            device_available: true,
            shader_translation_available: true,
            ..BackendProbe::prototype(GpuBackend::DirectX12)
        };
        let plan =
            plan_artifact_dispatch(GpuBackend::DirectX12, probe, request, &artifact).unwrap();
        assert_eq!(plan.backend.shader_transport, ShaderTransport::SpirvToDxil);
        assert_eq!(plan.backend.completion, CompletionModel::TimelineFence);
        assert!(plan.backend.requires_shader_translation);
    }

    #[test]
    fn artifact_dispatch_plan_rejects_artifact_before_backend_selection() {
        let mut artifact = valid_artifact();
        artifact.words = vec![0];
        let result = plan_artifact_dispatch(
            GpuBackend::Vulkan,
            BackendProbe::prototype(GpuBackend::Vulkan),
            ArtifactDispatchRequest {
                fp: FpPolicy::Strict,
                require_bounded_global_u32_array: false,
                require_async_completion: false,
            },
            &artifact,
        );
        assert!(matches!(
            result,
            Err(ArtifactDispatchPlanError::Artifact(
                SpirvArtifactContractError::InvalidSpirv(_)
            ))
        ));
    }

    #[test]
    fn artifact_resource_plan_validates_capacity_and_explicit_lease() {
        let mut table = ResourceTable::new();
        let buffer = table.create_buffer(64).unwrap();
        let mut artifact = resource_artifact(&[ResourceAccess::ReadWrite]);
        artifact.resources[0].descriptor_set = 3;
        artifact.resources[0].element_type_info =
            Some(jadren_codegen_spirv::ResourceElementType::Integer {
                signed: false,
                bits: 32,
                lanes: 1,
            });
        let plan = plan_artifact_resources(
            &table,
            &artifact,
            &[ArtifactResourceRequest {
                binding: 0,
                buffer,
                required_bytes: 32,
            }],
        )
        .unwrap();
        assert_eq!(plan.artifact().resource_binding_count, 1);
        assert_eq!(plan.bindings()[0].access, AccessKind::ReadWrite);
        assert_eq!(plan.bindings()[0].descriptor_set, 3);
        assert_eq!(plan.bindings()[0].address_space, AddressSpace::Storage);
        assert_eq!(
            plan.bindings()[0].element_type_info,
            Some(jadren_codegen_spirv::ResourceElementType::Integer {
                signed: false,
                bits: 32,
                lanes: 1,
            })
        );
        assert_eq!(
            plan.bindings()[0].layout,
            ArtifactResourceLayout::ScalarVector {
                element: jadren_codegen_spirv::ResourceElementType::Integer {
                    signed: false,
                    bits: 32,
                    lanes: 1,
                },
                stride: Some(4),
            }
        );
        assert_eq!(
            table.acquire_artifact_resources(&plan),
            Err(ResourceError::NotResident(buffer))
        );

        table.make_resident(buffer).unwrap();
        let lease = table.acquire_artifact_resources(&plan).unwrap();
        assert_eq!(lease.plan(), &plan);
        assert_eq!(lease.token_count(), 1);
        assert_eq!(table.info(buffer).unwrap().live_accesses, 1);
        table.release_artifact_resources(lease).unwrap();
        assert_eq!(table.info(buffer).unwrap().live_accesses, 0);
    }

    #[test]
    fn artifact_resource_plan_preserves_opaque_layout_stride() {
        let mut table = ResourceTable::new();
        let buffer = table.create_buffer(16).unwrap();
        let artifact = resource_artifact(&[ResourceAccess::ReadWrite]);
        let plan = plan_artifact_resources(
            &table,
            &artifact,
            &[ArtifactResourceRequest {
                binding: 0,
                buffer,
                required_bytes: 16,
            }],
        )
        .unwrap();

        assert_eq!(plan.bindings()[0].address_space, AddressSpace::Storage);
        assert_eq!(plan.bindings()[0].element_type_info, None);
        assert_eq!(
            plan.bindings()[0].layout,
            ArtifactResourceLayout::Opaque { stride: Some(4) }
        );
    }

    #[test]
    fn artifact_resource_plan_rejects_count_order_capacity_and_zero_requirements() {
        let mut table = ResourceTable::new();
        let buffer = table.create_buffer(8).unwrap();
        let artifact = resource_artifact(&[ResourceAccess::ReadWrite]);
        assert_eq!(
            plan_artifact_resources(&table, &artifact, &[]),
            Err(ArtifactResourcePlanError::BindingCountMismatch {
                expected: 1,
                actual: 0,
            })
        );
        assert_eq!(
            plan_artifact_resources(
                &table,
                &artifact,
                &[ArtifactResourceRequest {
                    binding: 1,
                    buffer,
                    required_bytes: 4,
                }]
            ),
            Err(ArtifactResourcePlanError::NonCanonicalBinding {
                expected: 0,
                actual: 1,
            })
        );
        assert_eq!(
            plan_artifact_resources(
                &table,
                &artifact,
                &[ArtifactResourceRequest {
                    binding: 0,
                    buffer,
                    required_bytes: 16,
                }]
            ),
            Err(ArtifactResourcePlanError::BufferTooSmall {
                binding: 0,
                required: 16,
                actual: 8,
            })
        );
        assert_eq!(
            plan_artifact_resources(
                &table,
                &artifact,
                &[ArtifactResourceRequest {
                    binding: 0,
                    buffer,
                    required_bytes: 0,
                }]
            ),
            Err(ArtifactResourcePlanError::InvalidRequiredBytes { binding: 0 })
        );
    }

    #[test]
    fn artifact_resource_plan_rejects_conflicting_writable_aliases() {
        let mut table = ResourceTable::new();
        let buffer = table.create_buffer(32).unwrap();
        let artifact = resource_artifact(&[ResourceAccess::ReadWrite, ResourceAccess::ReadWrite]);
        assert_eq!(
            plan_artifact_resources(
                &table,
                &artifact,
                &[
                    ArtifactResourceRequest {
                        binding: 0,
                        buffer,
                        required_bytes: 4,
                    },
                    ArtifactResourceRequest {
                        binding: 1,
                        buffer,
                        required_bytes: 4,
                    },
                ]
            ),
            Err(ArtifactResourcePlanError::ConflictingAlias {
                buffer,
                first_binding: 0,
                second_binding: 1,
            })
        );
    }

    #[test]
    fn artifact_resource_acquire_rolls_back_when_later_buffer_is_not_resident() {
        let mut table = ResourceTable::new();
        let first = table.create_buffer(16).unwrap();
        let second = table.create_buffer(16).unwrap();
        table.make_resident(first).unwrap();
        let artifact = resource_artifact(&[ResourceAccess::ReadWrite, ResourceAccess::ReadOnly]);
        let plan = plan_artifact_resources(
            &table,
            &artifact,
            &[
                ArtifactResourceRequest {
                    binding: 0,
                    buffer: first,
                    required_bytes: 8,
                },
                ArtifactResourceRequest {
                    binding: 1,
                    buffer: second,
                    required_bytes: 4,
                },
            ],
        )
        .unwrap();
        assert_eq!(
            table.acquire_artifact_resources(&plan),
            Err(ResourceError::NotResident(second))
        );
        assert_eq!(table.info(first).unwrap().live_accesses, 0);
        assert_eq!(table.info(second).unwrap().live_accesses, 0);
    }

    #[test]
    fn affine_tensor_layout_maps_coordinates_and_capacity() {
        let layout = TensorLayout2D::new(4, 3, 2, 10, 40).unwrap();
        assert_eq!(layout.physical_index(0, 0), Some(0));
        assert_eq!(layout.physical_index(3, 2), Some(26));
        assert_eq!(layout.physical_index(4, 0), None);
        assert_eq!(layout.physical_index(0, 3), None);
        let clipped = TensorLayout2D::new(4, 3, 2, 10, 20).unwrap();
        assert_eq!(clipped.physical_index(3, 2), None);
    }

    #[test]
    fn row_major_3d_layout_matches_gpu_contract() {
        let layout = TensorLayout3D::row_major(5, 3, 2, 40).unwrap();
        assert_eq!(layout.physical_index(0, 0, 0), Some(0));
        assert_eq!(layout.physical_index(4, 2, 1), Some(29));
        assert_eq!(layout.physical_index(5, 0, 0), None);
        assert_eq!(layout.physical_index(0, 0, 2), None);
    }

    #[test]
    fn tensor_layout_rejects_zero_components_and_overflow() {
        assert_eq!(
            TensorLayout2D::new(0, 1, 1, 1, 1),
            Err(TensorLayoutError::ZeroComponent)
        );
        assert_eq!(
            TensorLayout3D::new(1, 1, 1, 0, 1, 1, 1),
            Err(TensorLayoutError::ZeroComponent)
        );
        assert_eq!(
            TensorLayout3D::new(usize::MAX, 2, 2, 2, 1, 1, usize::MAX),
            Err(TensorLayoutError::ArithmeticOverflow)
        );
    }

    #[test]
    fn residency_requires_explicit_transition_and_join() {
        let mut table = ResourceTable::new();
        let id = table.create_buffer(128).unwrap();
        assert_eq!(
            table.acquire(id, AccessKind::Read),
            Err(ResourceError::NotResident(id))
        );
        table.make_resident(id).unwrap();
        let first = table.acquire(id, AccessKind::Read).unwrap();
        let second = table.acquire(id, AccessKind::Read).unwrap();
        assert_eq!(table.info(id).unwrap().live_accesses, 2);
        assert_eq!(table.evict(id), Err(ResourceError::Busy(id)));
        table.release(first).unwrap();
        table.release(second).unwrap();
        table.evict(id).unwrap();
        assert_eq!(table.info(id).unwrap().residency, Residency::Host);
    }

    #[test]
    fn conflicting_access_and_stale_token_are_rejected() {
        let mut table = ResourceTable::new();
        let id = table.create_buffer(64).unwrap();
        table.make_shared(id).unwrap();
        let read = table.acquire(id, AccessKind::Read).unwrap();
        assert_eq!(
            table.acquire(id, AccessKind::Write),
            Err(ResourceError::AccessConflict(id))
        );
        let stale = read.clone();
        table.release(read).unwrap();
        assert_eq!(table.release(stale), Err(ResourceError::StaleToken(id)));
        let write = table.acquire(id, AccessKind::ReadWrite).unwrap();
        table.release(write).unwrap();
    }

    #[test]
    fn fallback_linkage_requires_valid_symbol_and_preserves_gpu_entry() {
        let fallback = CpuFallbackLink::new("jadren_cpu_kernel", 9).unwrap();
        let linkage = KernelLinkage::new(Some("jadren_gpu_kernel"), fallback.clone()).unwrap();
        assert_eq!(linkage.gpu_entry(), Some("jadren_gpu_kernel"));
        assert_eq!(linkage.cpu_fallback(), &fallback);
        assert_eq!(fallback.abi_minor(), 9);
        assert_eq!(
            CpuFallbackLink::new("jadren-cpu", 9),
            Err(LinkageError::InvalidSymbol)
        );
        assert_eq!(
            CpuFallbackLink::new("jadren_cpu", 0),
            Err(LinkageError::InvalidAbi)
        );
        assert_eq!(
            KernelLinkage::new(Some("jadren-gpu"), fallback),
            Err(LinkageError::InvalidGpuEntry)
        );
    }

    #[test]
    fn target_selection_is_explicit_and_transfer_aware() {
        let base = SelectionRequest {
            preference: TargetPreference::Auto,
            fp: FpPolicy::Deterministic,
            gpu_available: true,
            gpu_supports_fp: true,
            transfer_cost: 10,
            estimated_cpu_cost: 100,
            estimated_gpu_cost: 20,
            allow_cpu_fallback: true,
        };
        assert_eq!(
            select_target(base).unwrap(),
            DispatchDecision {
                target: TargetPreference::Gpu,
                reason: SelectionReason::AutoGpuFaster
            }
        );
        let expensive = SelectionRequest {
            transfer_cost: 200,
            ..base
        };
        assert_eq!(
            select_target(expensive).unwrap(),
            DispatchDecision {
                target: TargetPreference::Cpu,
                reason: SelectionReason::TransferTooExpensive
            }
        );
        let explicit_cpu = SelectionRequest {
            preference: TargetPreference::Cpu,
            gpu_available: false,
            ..base
        };
        assert_eq!(
            select_target(explicit_cpu).unwrap().reason,
            SelectionReason::ExplicitCpu
        );
    }

    #[test]
    fn explicit_gpu_can_fail_when_fallback_is_disabled() {
        let request = SelectionRequest {
            preference: TargetPreference::Gpu,
            fp: FpPolicy::Strict,
            gpu_available: false,
            gpu_supports_fp: false,
            transfer_cost: 0,
            estimated_cpu_cost: 1,
            estimated_gpu_cost: 1,
            allow_cpu_fallback: false,
        };
        assert_eq!(select_target(request), Err(SelectionError::GpuUnavailable));
    }

    #[test]
    fn differential_compare_respects_exact_and_tolerance_modes() {
        assert!(compare_f32(&[1.0, -0.0], &[1.0, -0.0], DifferentialPolicy::Exact).is_ok());
        assert!(matches!(
            compare_f32(&[0.0], &[-0.0], DifferentialPolicy::Exact),
            Err(DifferentialError::ValueMismatch { index: 0, .. })
        ));
        assert!(
            compare_f32(
                &[1.0, f32::NAN],
                &[1.0001, f32::NAN],
                DifferentialPolicy::FloatTolerance {
                    absolute: 0.001,
                    relative: 0.0
                }
            )
            .is_ok()
        );
        assert_eq!(
            compare_f32(
                &[1.0],
                &[1.0],
                DifferentialPolicy::FloatTolerance {
                    absolute: -1.0,
                    relative: 0.0
                }
            ),
            Err(DifferentialError::InvalidTolerance)
        );
    }

    #[test]
    fn differential_compare_supports_exact_u32_outputs() {
        assert!(compare_u32(&[0, 42, u32::MAX], &[0, 42, u32::MAX]).is_ok());
        assert!(matches!(
            compare_u32(&[42], &[41]),
            Err(DifferentialError::ValueMismatch {
                index: 0,
                expected_bits: 42,
                actual_bits: 41,
                absolute_error_bits: 1,
            })
        ));
        assert!(matches!(
            compare_u32(&[1], &[]),
            Err(DifferentialError::LengthMismatch {
                expected: 1,
                actual: 0,
            })
        ));
    }

    #[test]
    fn shader_translation_route_is_canonical_for_each_backend() {
        assert_eq!(
            ShaderTranslationRoute::for_backend(GpuBackend::Vulkan),
            ShaderTranslationRoute {
                transport: ShaderTransport::NativeSpirv,
                source_backend: None,
            }
        );
        assert_eq!(
            ShaderTranslationRoute::for_backend(GpuBackend::DirectX12),
            ShaderTranslationRoute {
                transport: ShaderTransport::SpirvToDxil,
                source_backend: Some(ArtifactSourceBackend::Hlsl),
            }
        );
        assert_eq!(
            ShaderTranslationRoute::for_backend(GpuBackend::Metal),
            ShaderTranslationRoute {
                transport: ShaderTransport::Msl,
                source_backend: Some(ArtifactSourceBackend::Msl),
            }
        );
    }

    #[test]
    fn vulkan_platform_plan_matches_verified_array_contract() {
        let request = BackendRequest {
            fp: FpPolicy::Deterministic,
            workgroup_size: 64,
            require_bounded_global_u32_array: true,
            require_async_completion: true,
        };
        let plan = plan_backend(
            GpuBackend::Vulkan,
            BackendProbe::prototype(GpuBackend::Vulkan),
            request,
        )
        .unwrap();
        assert_eq!(plan.shader_transport, ShaderTransport::NativeSpirv);
        assert_eq!(plan.completion, CompletionModel::Fence);
        assert!(!plan.requires_shader_translation);
        assert_eq!(plan.workgroup_size, 64);
    }

    #[test]
    fn directx_requires_explicit_translation_toolchain() {
        let request = BackendRequest {
            fp: FpPolicy::Fast,
            workgroup_size: 64,
            require_bounded_global_u32_array: true,
            require_async_completion: true,
        };
        let unavailable = BackendProbe {
            device_available: true,
            ..BackendProbe::prototype(GpuBackend::DirectX12)
        };
        assert_eq!(
            plan_backend(GpuBackend::DirectX12, unavailable, request),
            Err(BackendPlanError::ShaderTranslationUnavailable)
        );
        let available = BackendProbe {
            shader_translation_available: true,
            ..unavailable
        };
        let plan = plan_backend(GpuBackend::DirectX12, available, request).unwrap();
        assert_eq!(plan.shader_transport, ShaderTransport::SpirvToDxil);
        assert_eq!(plan.completion, CompletionModel::TimelineFence);
        assert!(plan.requires_shader_translation);
    }

    #[test]
    fn metal_does_not_claim_deterministic_f32_before_msl_validation() {
        let request = BackendRequest {
            fp: FpPolicy::Deterministic,
            workgroup_size: 64,
            require_bounded_global_u32_array: true,
            require_async_completion: true,
        };
        let probe = BackendProbe {
            device_available: true,
            ..BackendProbe::prototype(GpuBackend::Metal)
        };
        assert_eq!(
            plan_backend(GpuBackend::Metal, probe, request),
            Err(BackendPlanError::DeterministicFpUnsupported)
        );
    }

    #[test]
    fn platform_probe_rejects_unavailable_device_before_backend_selection() {
        let request = BackendRequest {
            fp: FpPolicy::Fast,
            workgroup_size: 64,
            require_bounded_global_u32_array: false,
            require_async_completion: false,
        };
        assert_eq!(
            plan_backend(
                GpuBackend::DirectX12,
                BackendProbe::prototype(GpuBackend::DirectX12),
                request
            ),
            Err(BackendPlanError::DeviceUnavailable)
        );
    }

    #[test]
    fn dispatch_geometry_rejects_zero_and_product_overflow() {
        assert_eq!(
            DispatchGeometry::new([0, 1, 1]),
            Err(DispatchGeometryError::ZeroDimension { axis: 0 })
        );
        assert_eq!(
            DispatchGeometry::new([u32::MAX, u32::MAX, u32::MAX]),
            Err(DispatchGeometryError::WorkgroupCountOverflow)
        );
        let geometry = DispatchGeometry::new([u32::MAX, u32::MAX, 1]).unwrap();
        assert_eq!(
            geometry.invocation_count([8, 2, 1]),
            Err(DispatchGeometryError::InvocationCountOverflow)
        );
    }

    #[test]
    fn prepared_dispatch_rejects_invocation_overflow_before_resource_acquire() {
        let mut table = ResourceTable::new();
        let buffer = table.create_buffer(32).unwrap();
        table.make_resident(buffer).unwrap();
        let artifact = resource_artifact(&[ResourceAccess::ReadWrite]);
        let result = prepare_artifact_dispatch(
            &mut table,
            GpuBackend::Vulkan,
            BackendProbe::prototype(GpuBackend::Vulkan),
            ArtifactDispatchRequest {
                fp: FpPolicy::Strict,
                require_bounded_global_u32_array: false,
                require_async_completion: false,
            },
            DispatchGeometry::new([u32::MAX, u32::MAX, 1]).unwrap(),
            &[ArtifactResourceRequest {
                binding: 0,
                buffer,
                required_bytes: 16,
            }],
            &artifact,
        );
        assert_eq!(
            result,
            Err(PreparedArtifactDispatchError::Geometry(
                DispatchGeometryError::InvocationCountOverflow
            ))
        );
        assert_eq!(table.info(buffer).unwrap().live_accesses, 0);
    }

    #[test]
    fn prepared_dispatch_unifies_route_resources_and_release() {
        let mut table = ResourceTable::new();
        let buffer = table.create_buffer(64).unwrap();
        table.make_resident(buffer).unwrap();
        let artifact = resource_artifact(&[ResourceAccess::ReadWrite]);
        let prepared = prepare_artifact_dispatch(
            &mut table,
            GpuBackend::Vulkan,
            BackendProbe::prototype(GpuBackend::Vulkan),
            ArtifactDispatchRequest {
                fp: FpPolicy::Strict,
                require_bounded_global_u32_array: false,
                require_async_completion: true,
            },
            DispatchGeometry::new([2, 1, 1]).unwrap(),
            &[ArtifactResourceRequest {
                binding: 0,
                buffer,
                required_bytes: 32,
            }],
            &artifact,
        )
        .unwrap();
        assert_eq!(prepared.route().backend.backend, GpuBackend::Vulkan);
        assert_eq!(
            prepared.route().backend.shader_transport,
            ShaderTransport::NativeSpirv
        );
        assert_eq!(prepared.route().artifact, *prepared.resources().artifact());
        assert_eq!(prepared.descriptor().workgroups, [2, 1, 1]);
        assert_eq!(prepared.descriptor().invocation_count, 32);
        assert_eq!(prepared.descriptor().resources.len(), 1);
        assert!(prepared.descriptor().source_translation.is_none());
        assert_eq!(prepared.lease().token_count(), 1);
        assert_eq!(table.info(buffer).unwrap().live_accesses, 1);
        table.release_prepared_artifact_dispatch(prepared).unwrap();
        assert_eq!(table.info(buffer).unwrap().live_accesses, 0);
    }

    #[test]
    fn prepared_dispatch_rolls_back_resources_when_later_acquire_fails() {
        let mut table = ResourceTable::new();
        let first = table.create_buffer(32).unwrap();
        let second = table.create_buffer(32).unwrap();
        table.make_resident(first).unwrap();
        let artifact = resource_artifact(&[ResourceAccess::ReadWrite, ResourceAccess::ReadOnly]);
        let result = prepare_artifact_dispatch(
            &mut table,
            GpuBackend::Vulkan,
            BackendProbe::prototype(GpuBackend::Vulkan),
            ArtifactDispatchRequest {
                fp: FpPolicy::Strict,
                require_bounded_global_u32_array: false,
                require_async_completion: false,
            },
            DispatchGeometry::new([2, 1, 1]).unwrap(),
            &[
                ArtifactResourceRequest {
                    binding: 0,
                    buffer: first,
                    required_bytes: 16,
                },
                ArtifactResourceRequest {
                    binding: 1,
                    buffer: second,
                    required_bytes: 4,
                },
            ],
            &artifact,
        );
        assert_eq!(
            result,
            Err(PreparedArtifactDispatchError::Resource(
                ResourceError::NotResident(second)
            ))
        );
        assert_eq!(table.info(first).unwrap().live_accesses, 0);
        assert_eq!(table.info(second).unwrap().live_accesses, 0);
    }

    #[test]
    fn prepared_translated_dispatch_carries_source_translation_plan() {
        for (backend, expected_source_backend) in [
            (GpuBackend::DirectX12, ArtifactSourceBackend::Hlsl),
            (GpuBackend::Metal, ArtifactSourceBackend::Msl),
        ] {
            let mut table = ResourceTable::new();
            let buffer = table.create_buffer(32).unwrap();
            table.make_resident(buffer).unwrap();
            let artifact = resource_artifact(&[ResourceAccess::ReadWrite]);
            let probe = BackendProbe {
                device_available: true,
                shader_translation_available: true,
                ..BackendProbe::prototype(backend)
            };
            let prepared = prepare_artifact_dispatch(
                &mut table,
                backend,
                probe,
                ArtifactDispatchRequest {
                    fp: FpPolicy::Fast,
                    require_bounded_global_u32_array: false,
                    require_async_completion: false,
                },
                DispatchGeometry::new([1, 1, 1]).unwrap(),
                &[ArtifactResourceRequest {
                    binding: 0,
                    buffer,
                    required_bytes: 16,
                }],
                &artifact,
            )
            .unwrap();
            let plan = prepared
                .descriptor()
                .source_translation
                .as_ref()
                .expect("translated backend carries source plan");
            assert_eq!(plan.backend, expected_source_backend);
            assert_eq!(plan.artifact, *prepared.resources().artifact());
            assert_eq!(
                plan.resources,
                vec![ArtifactResourceCapability {
                    binding: 0,
                    descriptor_set: 0,
                    address_space: AddressSpace::Storage,
                    access: ResourceAccess::ReadWrite,
                    layout: ArtifactResourceLayout::Opaque { stride: Some(4) },
                }]
            );
            table.release_prepared_artifact_dispatch(prepared).unwrap();
        }
    }

    #[test]
    fn descriptor_validation_rejects_tampered_source_plan() {
        let mut table = ResourceTable::new();
        let buffer = table.create_buffer(32).unwrap();
        table.make_resident(buffer).unwrap();
        let artifact = resource_artifact(&[ResourceAccess::ReadWrite]);
        let prepared = prepare_artifact_dispatch(
            &mut table,
            GpuBackend::DirectX12,
            BackendProbe {
                device_available: true,
                shader_translation_available: true,
                ..BackendProbe::prototype(GpuBackend::DirectX12)
            },
            ArtifactDispatchRequest {
                fp: FpPolicy::Fast,
                require_bounded_global_u32_array: false,
                require_async_completion: false,
            },
            DispatchGeometry::new([1, 1, 1]).unwrap(),
            &[ArtifactResourceRequest {
                binding: 0,
                buffer,
                required_bytes: 16,
            }],
            &artifact,
        )
        .unwrap();
        let descriptor = prepared.descriptor().clone();
        assert!(descriptor.validate_source_translation().is_ok());

        let mut wrong_backend = descriptor.clone();
        wrong_backend.source_translation.as_mut().unwrap().backend = ArtifactSourceBackend::Msl;
        assert_eq!(
            wrong_backend.validate_source_translation(),
            Err(ArtifactDispatchDescriptorError::SourceBackendMismatch {
                expected: ArtifactSourceBackend::Hlsl,
                actual: ArtifactSourceBackend::Msl,
            })
        );

        let mut wrong_layout = descriptor;
        wrong_layout.resources[0].layout = ArtifactResourceLayout::Opaque { stride: None };
        assert_eq!(
            wrong_layout.validate_source_translation(),
            Err(ArtifactDispatchDescriptorError::ResourceCapabilityMismatch { binding: 0 })
        );
        table.release_prepared_artifact_dispatch(prepared).unwrap();
    }

    #[test]
    fn descriptor_validation_rejects_tampered_geometry_and_completion() {
        let artifact = valid_artifact();
        let descriptor = descriptor_for_backend(GpuBackend::Vulkan, &artifact);
        assert!(descriptor.validate_source_translation().is_ok());

        let mut wrong_completion = descriptor.clone();
        wrong_completion.completion = CompletionModel::TimelineFence;
        assert_eq!(
            wrong_completion.validate_source_translation(),
            Err(ArtifactDispatchDescriptorError::CompletionMismatch {
                expected: CompletionModel::Fence,
                actual: CompletionModel::TimelineFence,
            })
        );

        let mut wrong_workgroup_size = descriptor.clone();
        wrong_workgroup_size.workgroup_size = [4, 2, 1];
        assert_eq!(
            wrong_workgroup_size.validate_source_translation(),
            Err(ArtifactDispatchDescriptorError::WorkgroupSizeMismatch {
                expected: [8, 2, 1],
                actual: [4, 2, 1],
            })
        );

        let mut zero_workgroups = descriptor.clone();
        zero_workgroups.workgroups = [0, 1, 1];
        assert_eq!(
            zero_workgroups.validate_source_translation(),
            Err(ArtifactDispatchDescriptorError::ZeroDispatchWorkgroup([
                0, 1, 1
            ]))
        );

        let mut wrong_invocation_count = descriptor;
        wrong_invocation_count.invocation_count = 31;
        assert_eq!(
            wrong_invocation_count.validate_source_translation(),
            Err(ArtifactDispatchDescriptorError::InvocationCountMismatch {
                expected: 32,
                actual: 31,
            })
        );
    }

    #[test]
    fn prepared_dispatch_rejects_backend_before_resource_acquisition() {
        let mut table = ResourceTable::new();
        let buffer = table.create_buffer(32).unwrap();
        let artifact = resource_artifact(&[ResourceAccess::ReadWrite]);
        let result = prepare_artifact_dispatch(
            &mut table,
            GpuBackend::DirectX12,
            BackendProbe::prototype(GpuBackend::DirectX12),
            ArtifactDispatchRequest {
                fp: FpPolicy::Fast,
                require_bounded_global_u32_array: false,
                require_async_completion: false,
            },
            DispatchGeometry::new([1, 1, 1]).unwrap(),
            &[ArtifactResourceRequest {
                binding: 0,
                buffer,
                required_bytes: 16,
            }],
            &artifact,
        );
        assert_eq!(
            result,
            Err(PreparedArtifactDispatchError::Dispatch(
                ArtifactDispatchPlanError::Backend(BackendPlanError::DeviceUnavailable)
            ))
        );
        assert_eq!(table.info(buffer).unwrap().live_accesses, 0);
    }

    #[test]
    fn supported_subset_admits_exact_case_and_rejects_six_boundary_drifts() {
        let options = jadren_codegen_spirv::SpirvOptions::new([64, 1, 1]).unwrap();
        let words = jadren_codegen_spirv::emit_storage_global_index_f32_binary_dynamic_length(
            "global_add_dynamic_f32",
            options,
            1.0_f32.to_bits(),
            jadren_codegen_spirv::F32ArithmeticOp::Add,
        )
        .unwrap();
        let admitted =
            admit_gpu_supported_subset_v0_2("f32.add", &words, "global_add_dynamic_f32").unwrap();
        assert_eq!(admitted, JADREN_GPU_SUPPORTED_SUBSET_V0_2.cases[1]);
        assert_eq!(JADREN_GPU_SUPPORTED_SUBSET_V0_2.case_count, 7);

        assert!(matches!(
            admit_gpu_supported_subset_v0_2("f32.divide", &words, "global_add_dynamic_f32"),
            Err(GpuSupportedSubsetAdmissionError::UnknownCase(_))
        ));
        assert!(matches!(
            admit_gpu_supported_subset_v0_2("f32.add", &words, "other_entry"),
            Err(GpuSupportedSubsetAdmissionError::EntryMismatch { .. })
        ));

        let wrong_workgroup =
            jadren_codegen_spirv::emit_storage_global_index_f32_binary_dynamic_length(
                "global_add_dynamic_f32",
                jadren_codegen_spirv::SpirvOptions::new([32, 1, 1]).unwrap(),
                1.0_f32.to_bits(),
                jadren_codegen_spirv::F32ArithmeticOp::Add,
            )
            .unwrap();
        assert!(matches!(
            admit_gpu_supported_subset_v0_2("f32.add", &wrong_workgroup, "global_add_dynamic_f32"),
            Err(GpuSupportedSubsetAdmissionError::WorkgroupMismatch { .. })
        ));

        let wrong_resource =
            jadren_codegen_spirv::emit_storage_global_index_vector_f32_binary_dynamic_length(
                "global_add_dynamic_f32",
                options,
                1.0_f32.to_bits(),
                jadren_codegen_spirv::F32ArithmeticOp::Add,
            )
            .unwrap();
        assert!(matches!(
            admit_gpu_supported_subset_v0_2("f32.add", &wrong_resource, "global_add_dynamic_f32"),
            Err(GpuSupportedSubsetAdmissionError::ResourceContractMismatch)
        ));

        let wrong_hash = jadren_codegen_spirv::emit_storage_global_index_f32_binary_dynamic_length(
            "global_add_dynamic_f32",
            options,
            2.0_f32.to_bits(),
            jadren_codegen_spirv::F32ArithmeticOp::Add,
        )
        .unwrap();
        assert!(matches!(
            admit_gpu_supported_subset_v0_2("f32.add", &wrong_hash, "global_add_dynamic_f32"),
            Err(GpuSupportedSubsetAdmissionError::WordHashMismatch { .. })
        ));
        assert!(matches!(
            admit_gpu_supported_subset_v0_2(
                "f32.add",
                &words[..words.len() - 1],
                "global_add_dynamic_f32"
            ),
            Err(GpuSupportedSubsetAdmissionError::Source(_))
                | Err(GpuSupportedSubsetAdmissionError::WordCountMismatch { .. })
        ));
    }
}
