use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use inkwell::OptimizationLevel;
use inkwell::context::Context;
use inkwell::module::Module as LlvmModule;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use jadren_jir::Module;

use crate::{
    CodegenError, DebugInfoConfig, TypeLoweringConfig, lower_module, lower_module_with_debug,
};

/// Optimization policy used while emitting a Windows x86-64 object.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ObjectOptimization {
    #[default]
    Debug,
    Release,
}

/// CPU code-generation variants known to the x86-64 backend.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CpuVariant {
    /// Portable x86-64 baseline (no optional ISA feature).
    X86_64Baseline,
    /// x86-64 with AVX2 enabled explicitly.
    X86_64Avx2,
}

impl CpuVariant {
    /// Returns the stable report spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::X86_64Baseline => "x86-64-baseline",
            Self::X86_64Avx2 => "x86-64-avx2",
        }
    }
}

/// CPU capabilities observed by the host-side selector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpuFeatures {
    /// Whether the host advertises AVX2 and the OS supports saving its state.
    pub avx2: bool,
}

impl CpuFeatures {
    /// Detects features using the standard library's OS-aware x86 probe.
    #[must_use]
    pub fn detect() -> Self {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            Self {
                avx2: std::is_x86_feature_detected!("avx2"),
            }
        }
        #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
        {
            Self { avx2: false }
        }
    }

    /// Creates a deterministic capability set for tests and callers with a
    /// previously recorded CPUID result.
    #[must_use]
    pub const fn from_avx2(avx2: bool) -> Self {
        Self { avx2 }
    }

    /// Returns whether a variant is executable on this host.
    #[must_use]
    pub const fn supports(self, variant: CpuVariant) -> bool {
        match variant {
            CpuVariant::X86_64Baseline => true,
            CpuVariant::X86_64Avx2 => self.avx2,
        }
    }

    /// Chooses the best supported variant while preserving baseline fallback.
    #[must_use]
    pub const fn preferred_variant(self) -> CpuVariant {
        if self.avx2 {
            CpuVariant::X86_64Avx2
        } else {
            CpuVariant::X86_64Baseline
        }
    }
}

/// Result of selecting one callable implementation from a dispatch table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DispatchSelection<F> {
    /// Variant selected for this process.
    pub variant: CpuVariant,
    /// Callable implementation associated with the variant.
    pub implementation: F,
}

/// One-time CPU dispatch table for a baseline and optional AVX2 implementation.
///
/// `resolve` caches the first selection so a hot loop never performs CPUID or
/// feature checks. `select` is the pure, non-cached form used by tests and by
/// callers that need an explicit forced-variant policy.
pub struct CpuDispatch<F> {
    baseline: F,
    avx2: Option<F>,
    selected: OnceLock<DispatchSelection<F>>,
}

impl<F: Copy + Send + Sync> CpuDispatch<F> {
    /// Creates a dispatch table with a mandatory baseline and optional AVX2 entry.
    #[must_use]
    pub fn new(baseline: F, avx2: Option<F>) -> Self {
        Self {
            baseline,
            avx2,
            selected: OnceLock::new(),
        }
    }

    /// Selects an implementation without caching the result.
    #[must_use]
    pub fn select(
        &self,
        features: CpuFeatures,
        requested: Option<CpuVariant>,
    ) -> DispatchSelection<F> {
        let requested = requested.unwrap_or_else(|| features.preferred_variant());
        if requested == CpuVariant::X86_64Avx2
            && features.supports(CpuVariant::X86_64Avx2)
            && let Some(implementation) = self.avx2
        {
            return DispatchSelection {
                variant: CpuVariant::X86_64Avx2,
                implementation,
            };
        }
        DispatchSelection {
            variant: CpuVariant::X86_64Baseline,
            implementation: self.baseline,
        }
    }

    /// Detects host features and caches the selected implementation exactly once.
    #[must_use]
    pub fn resolve(&self) -> DispatchSelection<F> {
        *self
            .selected
            .get_or_init(|| self.select(CpuFeatures::detect(), None))
    }

    /// Caches a selection from an explicit capability set (useful for hosts/tests).
    #[must_use]
    pub fn resolve_with_features(&self, features: CpuFeatures) -> DispatchSelection<F> {
        *self.selected.get_or_init(|| self.select(features, None))
    }
}

/// Reproducible target-machine inputs for JAD-607.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectOptions {
    pub cpu: String,
    pub features: String,
    pub optimization: ObjectOptimization,
}

impl Default for ObjectOptions {
    fn default() -> Self {
        Self::x86_64_baseline()
    }
}

impl ObjectOptions {
    /// Returns the portable x86-64 baseline policy.
    ///
    /// The baseline deliberately leaves LLVM's feature string empty and uses
    /// the generic `x86-64` CPU name.  This is the only SIMD policy available
    /// in JAD-909; AVX variants must be introduced as separate policies so a
    /// caller cannot accidentally make the baseline ISA-dependent.
    #[must_use]
    pub fn x86_64_baseline() -> Self {
        Self::for_variant(CpuVariant::X86_64Baseline)
    }

    /// Returns the portable x86-64 baseline Release policy.
    #[must_use]
    pub fn x86_64_baseline_release() -> Self {
        Self::for_variant_with_optimization(CpuVariant::X86_64Baseline, ObjectOptimization::Release)
    }

    /// Returns the explicit x86-64 AVX2 policy used by JAD-910.
    ///
    /// AVX2 is opt-in.  Keeping the feature in the target-machine input (and
    /// not in the JIR) lets the same verified module be emitted for baseline
    /// and AVX2 hosts while preserving a deterministic fallback path.
    #[must_use]
    pub fn x86_64_avx2() -> Self {
        Self::for_variant(CpuVariant::X86_64Avx2)
    }

    /// Returns the explicit x86-64 AVX2 Release policy.
    #[must_use]
    pub fn x86_64_avx2_release() -> Self {
        Self::for_variant_with_optimization(CpuVariant::X86_64Avx2, ObjectOptimization::Release)
    }

    /// Returns the conservative AArch64 Android scalar policy.
    ///
    /// NEON is intentionally not enabled here; it is a separate target
    /// variant after the scalar ABI/codegen gate.
    #[must_use]
    pub fn aarch64_android() -> Self {
        Self {
            cpu: "generic".to_owned(),
            features: String::new(),
            optimization: ObjectOptimization::Debug,
        }
    }

    /// Returns the explicit AArch64 Android NEON policy marker.
    ///
    /// NEON is mandatory in AArch64, so this can currently produce the same
    /// instructions as the scalar policy. A later cost/scheduling pass may
    /// specialize the policy without changing the target-neutral JIR.
    #[must_use]
    pub fn aarch64_neon() -> Self {
        Self {
            cpu: "generic".to_owned(),
            features: "+neon".to_owned(),
            optimization: ObjectOptimization::Debug,
        }
    }

    /// Returns the release AArch64 Android NEON policy.
    ///
    /// The target remains the portable `generic` AArch64 CPU and only enables
    /// the mandatory NEON feature. `Aggressive` LLVM optimization activates
    /// the cost model, vector combining and instruction scheduling without
    /// baking a device-specific microarchitecture into Android binaries.
    #[must_use]
    pub fn aarch64_neon_release() -> Self {
        Self {
            cpu: "generic".to_owned(),
            features: "+neon".to_owned(),
            optimization: ObjectOptimization::Release,
        }
    }

    /// Creates object options for one explicitly selected CPU variant.
    #[must_use]
    pub fn for_variant(variant: CpuVariant) -> Self {
        Self::for_variant_with_optimization(variant, ObjectOptimization::Debug)
    }

    /// Creates object options for one CPU variant and optimization policy.
    #[must_use]
    pub fn for_variant_with_optimization(
        variant: CpuVariant,
        optimization: ObjectOptimization,
    ) -> Self {
        let features = match variant {
            CpuVariant::X86_64Baseline => String::new(),
            CpuVariant::X86_64Avx2 => "+avx2".to_owned(),
        };
        Self {
            cpu: "x86-64".to_owned(),
            features,
            optimization,
        }
    }
}

/// Failure before a valid target object is available.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObjectError {
    Codegen(CodegenError),
    TargetInitialization(String),
    UnsupportedTriple(String),
    TargetLookup(String),
    TargetMachine(String),
    DataLayoutMismatch { module: String, machine: String },
    LlvmVerifier(String),
    Emit(String),
    Write { path: String, message: String },
}

impl fmt::Display for ObjectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codegen(error) => error.fmt(formatter),
            Self::TargetInitialization(message) => {
                write!(formatter, "failed to initialize LLVM x86 target: {message}")
            }
            Self::UnsupportedTriple(triple) => {
                write!(
                    formatter,
                    "JAD-607 does not support target triple `{triple}`"
                )
            }
            Self::TargetLookup(message) => {
                write!(formatter, "LLVM target lookup failed: {message}")
            }
            Self::TargetMachine(triple) => {
                write!(
                    formatter,
                    "LLVM could not create a target machine for `{triple}`"
                )
            }
            Self::DataLayoutMismatch { module, machine } => write!(
                formatter,
                "LLVM module data layout `{module}` differs from target machine `{machine}`"
            ),
            Self::LlvmVerifier(message) => write!(formatter, "LLVM verifier failed: {message}"),
            Self::Emit(message) => write!(formatter, "LLVM object emission failed: {message}"),
            Self::Write { path, message } => {
                write!(formatter, "failed to write object `{path}`: {message}")
            }
        }
    }
}

impl Error for ObjectError {}

impl From<CodegenError> for ObjectError {
    fn from(error: CodegenError) -> Self {
        Self::Codegen(error)
    }
}

/// Lowers verified JIR and emits one supported target object in memory.
pub fn lower_to_object(
    context: &Context,
    jir: &Module,
    module_name: &str,
    type_config: &TypeLoweringConfig,
    options: &ObjectOptions,
) -> Result<Vec<u8>, ObjectError> {
    let llvm = lower_module(context, jir, module_name, type_config)?;
    emit_object(&llvm, options)
}

/// Lowers verified JIR with debug metadata and emits one x86-64 object in memory.
pub fn lower_to_object_with_debug(
    context: &Context,
    jir: &Module,
    module_name: &str,
    type_config: &TypeLoweringConfig,
    debug_config: &DebugInfoConfig,
    options: &ObjectOptions,
) -> Result<Vec<u8>, ObjectError> {
    let llvm = lower_module_with_debug(context, jir, module_name, type_config, debug_config)?;
    emit_object(&llvm, options)
}

/// Emits deterministic object bytes from an already verified LLVM module.
pub fn emit_object(
    module: &LlvmModule<'_>,
    options: &ObjectOptions,
) -> Result<Vec<u8>, ObjectError> {
    let machine = create_target_machine(module, options)?;
    let buffer = machine
        .write_to_memory_buffer(module, FileType::Object)
        .map_err(|error| ObjectError::Emit(error.to_string()))?;
    Ok(buffer.as_slice().to_vec())
}

/// Emits target assembly text bytes from an already verified LLVM module.
pub fn emit_assembly(
    module: &LlvmModule<'_>,
    options: &ObjectOptions,
) -> Result<Vec<u8>, ObjectError> {
    let machine = create_target_machine(module, options)?;
    let buffer = machine
        .write_to_memory_buffer(module, FileType::Assembly)
        .map_err(|error| ObjectError::Emit(error.to_string()))?;
    let mut bytes = buffer.as_slice().to_vec();
    if bytes.last() == Some(&0) {
        let _ = bytes.pop();
    }
    Ok(bytes)
}

fn create_target_machine(
    module: &LlvmModule<'_>,
    options: &ObjectOptions,
) -> Result<TargetMachine, ObjectError> {
    module
        .verify()
        .map_err(|error| ObjectError::LlvmVerifier(error.to_string()))?;
    let triple = module.get_triple();
    let triple_text = triple.as_str().to_string_lossy().into_owned();
    let is_x86 = triple_text.starts_with("x86_64-pc-windows-msvc")
        || triple_text == "x86_64-unknown-linux-gnu";
    let is_aarch64_android = triple_text.starts_with("aarch64-unknown-linux-android")
        || triple_text.starts_with("aarch64-linux-android");
    if !is_x86 && !is_aarch64_android {
        return Err(ObjectError::UnsupportedTriple(triple_text));
    }
    let initialization = InitializationConfig {
        asm_parser: false,
        asm_printer: true,
        base: true,
        disassembler: false,
        info: true,
        machine_code: true,
    };
    if is_aarch64_android {
        Target::initialize_aarch64(&initialization);
    } else {
        Target::initialize_native(&initialization).map_err(ObjectError::TargetInitialization)?;
    }
    let target = Target::from_triple(&triple)
        .map_err(|error| ObjectError::TargetLookup(error.to_string()))?;
    let optimization = match options.optimization {
        ObjectOptimization::Debug => OptimizationLevel::None,
        ObjectOptimization::Release => OptimizationLevel::Aggressive,
    };
    let machine = target
        .create_target_machine(
            &triple,
            &options.cpu,
            &options.features,
            optimization,
            RelocMode::Default,
            CodeModel::Default,
        )
        .ok_or(ObjectError::TargetMachine(triple_text))?;
    verify_data_layout(module, &machine)?;
    Ok(machine)
}

/// Writes already-emitted object bytes without invoking a linker.
pub fn write_object(path: &Path, bytes: &[u8]) -> Result<(), ObjectError> {
    fs::write(path, bytes).map_err(|error| ObjectError::Write {
        path: path.display().to_string(),
        message: error.to_string(),
    })
}

fn verify_data_layout(module: &LlvmModule<'_>, machine: &TargetMachine) -> Result<(), ObjectError> {
    let module_layout = module.get_data_layout();
    let machine_layout = machine.get_target_data().get_data_layout();
    if *module_layout == machine_layout {
        Ok(())
    } else {
        Err(ObjectError::DataLayoutMismatch {
            module: module_layout.as_str().to_string_lossy().into_owned(),
            machine: machine_layout.as_str().to_string_lossy().into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use inkwell::context::Context;
    use jadren_jir::{
        BinaryOp, Block, BlockId, Function, FunctionId, Instruction, InstructionKind, Linkage,
        Module, Parameter, Terminator, Type, TypeId, TypedValue, ValueId,
    };
    use jadren_source::{SourceManager, Span};

    use super::{
        ObjectOptions, emit_assembly, lower_to_object, lower_to_object_with_debug, write_object,
    };
    use crate::{DebugInfoConfig, TypeLoweringConfig, lower_module_with_debug};

    #[test]
    fn cpu_feature_selection_has_a_safe_baseline_fallback() {
        let without_avx2 = super::CpuFeatures::from_avx2(false);
        assert!(without_avx2.supports(super::CpuVariant::X86_64Baseline));
        assert!(!without_avx2.supports(super::CpuVariant::X86_64Avx2));
        assert_eq!(
            without_avx2.preferred_variant(),
            super::CpuVariant::X86_64Baseline
        );

        let with_avx2 = super::CpuFeatures::from_avx2(true);
        assert!(with_avx2.supports(super::CpuVariant::X86_64Avx2));
        assert_eq!(with_avx2.preferred_variant(), super::CpuVariant::X86_64Avx2);
        assert_eq!(super::CpuVariant::X86_64Avx2.as_str(), "x86-64-avx2");
    }

    #[test]
    fn host_feature_probe_returns_a_supported_preferred_variant() {
        let detected = super::CpuFeatures::detect();
        assert!(detected.supports(detected.preferred_variant()));
    }

    fn baseline_impl() -> u32 {
        1
    }

    fn avx2_impl() -> u32 {
        2
    }

    #[test]
    fn dispatch_selects_supported_variant_and_caches_first_choice() {
        let dispatch =
            super::CpuDispatch::new(baseline_impl as fn() -> u32, Some(avx2_impl as fn() -> u32));
        let unsupported = dispatch.select(
            super::CpuFeatures::from_avx2(false),
            Some(super::CpuVariant::X86_64Avx2),
        );
        assert_eq!(unsupported.variant, super::CpuVariant::X86_64Baseline);
        assert_eq!((unsupported.implementation)(), 1);

        let supported = dispatch.select(
            super::CpuFeatures::from_avx2(true),
            Some(super::CpuVariant::X86_64Avx2),
        );
        assert_eq!(supported.variant, super::CpuVariant::X86_64Avx2);
        assert_eq!((supported.implementation)(), 2);

        let first = dispatch.resolve_with_features(super::CpuFeatures::from_avx2(false));
        let second = dispatch.resolve_with_features(super::CpuFeatures::from_avx2(true));
        assert_eq!(first.variant, super::CpuVariant::X86_64Baseline);
        assert_eq!(second.variant, super::CpuVariant::X86_64Baseline);
        assert_eq!((second.implementation)(), 1);
    }

    #[test]
    fn dispatch_without_avx2_entry_falls_back_even_when_host_supports_it() {
        let dispatch = super::CpuDispatch::new(baseline_impl as fn() -> u32, None);
        let selected = dispatch.select(super::CpuFeatures::from_avx2(true), None);
        assert_eq!(selected.variant, super::CpuVariant::X86_64Baseline);
        assert_eq!((selected.implementation)(), 1);
    }

    #[test]
    fn default_object_options_pin_x86_64_baseline_policy() {
        let options = ObjectOptions::default();
        assert_eq!(options, ObjectOptions::x86_64_baseline());
        assert_eq!(options.cpu, "x86-64");
        assert!(options.features.is_empty());
    }

    #[test]
    fn avx2_object_policy_is_explicit_and_distinct_from_baseline() {
        let baseline = ObjectOptions::x86_64_baseline();
        let avx2 = ObjectOptions::x86_64_avx2();
        assert_eq!(avx2.cpu, "x86-64");
        assert_eq!(avx2.features, "+avx2");
        assert_ne!(baseline.features, avx2.features);
    }

    #[test]
    fn release_object_policies_preserve_cpu_variant_and_enable_optimization() {
        let baseline = ObjectOptions::x86_64_baseline_release();
        let avx2 = ObjectOptions::x86_64_avx2_release();
        assert_eq!(baseline.cpu, "x86-64");
        assert!(baseline.features.is_empty());
        assert_eq!(baseline.optimization, super::ObjectOptimization::Release);
        assert_eq!(avx2.features, "+avx2");
        assert_eq!(avx2.optimization, super::ObjectOptimization::Release);
        assert_ne!(baseline, avx2);
    }

    #[test]
    fn emits_reproducible_amd64_coff_with_exported_symbol() {
        let jir = add_module();
        let context = Context::create();
        let first = lower_to_object(
            &context,
            &jir,
            "object_test",
            &TypeLoweringConfig::default(),
            &ObjectOptions::default(),
        )
        .expect("first COFF object");
        let second = lower_to_object(
            &context,
            &jir,
            "object_test",
            &TypeLoweringConfig::default(),
            &ObjectOptions::default(),
        )
        .expect("second COFF object");

        assert_eq!(first, second, "object emission must be reproducible");
        assert_eq!(&first[..2], &[0x64, 0x86], "COFF machine must be AMD64");
        assert!(
            first
                .windows(b"add_values".len())
                .any(|window| window == b"add_values"),
            "exported symbol must be present in the COFF string table"
        );

        let path = std::env::temp_dir().join(format!("jadren-object-{}.obj", std::process::id()));
        write_object(&path, &first).expect("write COFF object");
        assert_eq!(fs::read(&path).expect("read COFF object"), first);
        let readobj = Command::new(llvm_tool("llvm-readobj"))
            .args(["--file-headers", "--symbols"])
            .arg(&path)
            .output()
            .expect("llvm-readobj should start");
        assert!(
            readobj.status.success(),
            "{}",
            String::from_utf8_lossy(&readobj.stderr)
        );
        let inspection = String::from_utf8_lossy(&readobj.stdout);
        assert!(
            inspection.contains("IMAGE_FILE_MACHINE_AMD64"),
            "{inspection}"
        );
        assert!(inspection.contains("Name: add_values"), "{inspection}");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn cross_emits_reproducible_x86_64_elf_with_exported_symbol() {
        let jir = add_module();
        let context = Context::create();
        let config = TypeLoweringConfig::x86_64_linux_gnu();
        let first = lower_to_object(
            &context,
            &jir,
            "elf_object_test",
            &config,
            &ObjectOptions::default(),
        )
        .expect("first ELF object");
        let second = lower_to_object(
            &context,
            &jir,
            "elf_object_test",
            &config,
            &ObjectOptions::default(),
        )
        .expect("second ELF object");

        assert_eq!(first, second, "ELF object emission must be reproducible");
        assert_eq!(&first[..4], b"\x7fELF");
        assert!(
            first
                .windows(b"add_values".len())
                .any(|window| window == b"add_values")
        );

        let path = std::env::temp_dir().join(format!("jadren-elf-{}.o", std::process::id()));
        write_object(&path, &first).expect("write ELF object");
        let readobj = Command::new(llvm_tool("llvm-readobj"))
            .args(["--file-headers", "--symbols"])
            .arg(&path)
            .output()
            .expect("llvm-readobj should inspect ELF");
        assert!(
            readobj.status.success(),
            "{}",
            String::from_utf8_lossy(&readobj.stderr)
        );
        let inspection = String::from_utf8_lossy(&readobj.stdout);
        assert!(inspection.contains("Format: elf64-x86-64"), "{inspection}");
        assert!(inspection.contains("Machine: EM_X86_64"), "{inspection}");
        assert!(inspection.contains("Name: add_values"), "{inspection}");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn cross_emits_reproducible_aarch64_android_elf_with_exported_symbol() {
        let jir = add_module();
        let context = Context::create();
        let config = TypeLoweringConfig::aarch64_android();
        let options = ObjectOptions::aarch64_android();
        let first = lower_to_object(&context, &jir, "android_object_test", &config, &options)
            .expect("first AArch64 Android ELF object");
        let second = lower_to_object(&context, &jir, "android_object_test", &config, &options)
            .expect("second AArch64 Android ELF object");

        assert_eq!(
            first, second,
            "AArch64 object emission must be reproducible"
        );
        assert_eq!(&first[..4], b"\x7fELF");

        let path =
            std::env::temp_dir().join(format!("jadren-aarch64-android-{}.o", std::process::id()));
        write_object(&path, &first).expect("write AArch64 ELF object");
        let readobj = Command::new(llvm_tool("llvm-readobj"))
            .args(["--file-headers", "--symbols"])
            .arg(&path)
            .output()
            .expect("llvm-readobj should inspect AArch64 ELF");
        assert!(
            readobj.status.success(),
            "{}",
            String::from_utf8_lossy(&readobj.stderr)
        );
        let inspection = String::from_utf8_lossy(&readobj.stdout);
        assert!(
            inspection.contains("Machine: EM_AARCH64") || inspection.contains("Machine: AArch64"),
            "{inspection}"
        );
        assert!(inspection.contains("Name: add_values"), "{inspection}");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn emits_reproducible_target_assembly() {
        let jir = add_module();
        let context = Context::create();
        let llvm = crate::lower_module(
            &context,
            &jir,
            "assembly_test",
            &TypeLoweringConfig::default(),
        )
        .expect("LLVM module");
        let first = emit_assembly(&llvm, &ObjectOptions::default()).expect("first assembly");
        let second = emit_assembly(&llvm, &ObjectOptions::default()).expect("second assembly");

        assert_eq!(first, second, "assembly emission must be reproducible");
        let text = String::from_utf8(first).expect("LLVM assembly is UTF-8");
        assert!(text.contains("add_values:"), "{text}");
        assert!(text.contains("addl"), "{text}");
    }

    #[test]
    fn emits_reproducible_codeview_with_source_functions_parameters_and_lines() {
        let (jir, sources) = debug_module();
        let debug = DebugInfoConfig::from_source_manager(&sources, r"C:\workspace", false)
            .expect("debug configuration");
        let context = Context::create();
        let config = TypeLoweringConfig::x86_64_windows_msvc();
        let llvm = lower_module_with_debug(&context, &jir, "codeview_test", &config, &debug)
            .expect("debug LLVM module");
        let ir = llvm.print_to_string().to_string();
        assert!(ir.contains("!DICompileUnit(language: DW_LANG_Rust"), "{ir}");
        assert!(ir.contains("!DIFile(filename: \"add.jdn\""), "{ir}");
        assert!(
            ir.contains("DILocalVariable(name: \"left\", arg: 1"),
            "{ir}"
        );
        assert!(ir.contains("DILocation(line: 2"), "{ir}");

        let first = lower_to_object_with_debug(
            &context,
            &jir,
            "codeview_test",
            &config,
            &debug,
            &ObjectOptions::default(),
        )
        .expect("first CodeView object");
        let second = lower_to_object_with_debug(
            &context,
            &jir,
            "codeview_test",
            &config,
            &debug,
            &ObjectOptions::default(),
        )
        .expect("second CodeView object");
        assert_eq!(first, second, "CodeView object must be reproducible");

        let path = std::env::temp_dir().join(format!("jadren-codeview-{}.obj", std::process::id()));
        write_object(&path, &first).expect("write CodeView object");
        let inspection = llvm_readobj(&path, &["--sections", "--codeview"]);
        assert!(inspection.contains(".debug$S"), "{inspection}");
        assert!(inspection.contains("S_COMPILE3"), "{inspection}");
        assert!(inspection.contains("add.jdn"), "{inspection}");
        assert!(inspection.contains("add_values"), "{inspection}");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn cross_emits_reproducible_dwarf_with_source_and_line_sections() {
        let (jir, sources) = debug_module();
        let debug = DebugInfoConfig::from_source_manager(&sources, "/workspace", false)
            .expect("debug configuration");
        let context = Context::create();
        let config = TypeLoweringConfig::x86_64_linux_gnu();
        let first = lower_to_object_with_debug(
            &context,
            &jir,
            "dwarf_test",
            &config,
            &debug,
            &ObjectOptions::default(),
        )
        .expect("first DWARF object");
        let second = lower_to_object_with_debug(
            &context,
            &jir,
            "dwarf_test",
            &config,
            &debug,
            &ObjectOptions::default(),
        )
        .expect("second DWARF object");
        assert_eq!(first, second, "DWARF object must be reproducible");

        let path = std::env::temp_dir().join(format!("jadren-dwarf-{}.o", std::process::id()));
        write_object(&path, &first).expect("write DWARF object");
        let sections = llvm_readobj(&path, &["--sections"]);
        assert!(sections.contains(".debug_info"), "{sections}");
        assert!(sections.contains(".debug_line"), "{sections}");
        assert!(sections.contains(".debug_str"), "{sections}");
        let strings = Command::new(llvm_tool("llvm-strings"))
            .arg(&path)
            .output()
            .expect("llvm-strings should start");
        assert!(
            strings.status.success(),
            "{}",
            String::from_utf8_lossy(&strings.stderr)
        );
        let strings = String::from_utf8_lossy(&strings.stdout);
        assert!(strings.contains("add.jdn"), "{strings}");
        assert!(strings.contains("add_values"), "{strings}");
        let _ = fs::remove_file(path);
    }

    fn llvm_readobj(path: &Path, arguments: &[&str]) -> String {
        let output = Command::new(llvm_tool("llvm-readobj"))
            .args(arguments)
            .arg(path)
            .output()
            .expect("llvm-readobj should start");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn llvm_tool(name: &str) -> std::path::PathBuf {
        let suffix = if cfg!(windows) { ".exe" } else { "" };
        Path::new(env!("JADREN_LLVM_BIN")).join(format!("{name}{suffix}"))
    }

    fn add_module() -> Module {
        Module {
            types: vec![Type::Integer {
                signed: true,
                bits: 32,
            }],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "add_values".to_owned(),
                linkage: Linkage::Export,
                parameters: vec![
                    Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(0),
                        name: Some("left".to_owned()),
                    },
                    Parameter {
                        value: ValueId::new(1),
                        ty: TypeId::new(0),
                        name: Some("right".to_owned()),
                    },
                ],
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![Instruction {
                        result: Some(TypedValue {
                            value: ValueId::new(2),
                            ty: TypeId::new(0),
                        }),
                        kind: InstructionKind::Binary {
                            op: BinaryOp::Add,
                            left: ValueId::new(0),
                            right: ValueId::new(1),
                        },
                        span: None,
                    }],
                    terminator: Terminator::Return {
                        value: Some(ValueId::new(2)),
                    },
                    span: None,
                }],
                span: None,
            }],
        }
    }

    fn debug_module() -> (Module, SourceManager) {
        let text = "pub fn add_values(left: i32, right: i32) -> i32 {\n    let sum = left + right\n    return sum\n}\n";
        let mut sources = SourceManager::new();
        let source = sources
            .add(r"C:\workspace\.\src\add.jdn", text)
            .expect("source ID");
        let function_span = Span::new(source, 0, text.len()).expect("function span");
        let block_start = text.find("let sum").expect("block text");
        let expression_start = text.find("left + right").expect("expression text");
        let block_span = Span::new(source, block_start, text.len() - 2).expect("block span");
        let instruction_span = Span::new(
            source,
            expression_start,
            expression_start + "left + right".len(),
        )
        .expect("instruction span");
        let mut module = add_module();
        module.functions[0].span = Some(function_span);
        module.functions[0].blocks[0].span = Some(block_span);
        module.functions[0].blocks[0].instructions[0].span = Some(instruction_span);
        (module, sources)
    }
}
