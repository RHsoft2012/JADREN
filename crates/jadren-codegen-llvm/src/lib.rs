//! LLVM 22 backend foundations for Jadren JIR.

#[cfg(any(windows, target_os = "linux"))]
mod debug;
#[cfg(windows)]
mod link;
#[cfg(any(windows, target_os = "linux"))]
mod module;
#[cfg(any(windows, target_os = "linux"))]
mod object;
#[cfg(any(windows, target_os = "linux"))]
mod types;

#[cfg(any(windows, target_os = "linux"))]
pub use debug::{DebugInfoConfig, DebugInfoError};
#[cfg(windows)]
pub use link::{
    LinkError, WindowsLinkOptions, WindowsSubsystem, create_windows_static_library,
    link_windows_executable, link_windows_shared_library,
};
#[cfg(any(windows, target_os = "linux"))]
pub use module::{CodegenError, lower_module, lower_module_with_debug};
#[cfg(any(windows, target_os = "linux"))]
pub use object::{
    CpuDispatch, CpuFeatures, CpuVariant, DispatchSelection, ObjectError, ObjectOptimization,
    ObjectOptions, emit_assembly, emit_object, lower_to_object, lower_to_object_with_debug,
    write_object,
};
#[cfg(any(windows, target_os = "linux"))]
pub use types::{
    AARCH64_ANDROID_DATA_LAYOUT, EnumLayout, LoweredType, LoweredTypeTable, TypeLowerError,
    TypeLoweringConfig, X86_64_LINUX_GNU_DATA_LAYOUT, X86_64_WINDOWS_MSVC_DATA_LAYOUT, lower_types,
};

/// LLVM release pinned by the Jadren native backend.
pub const LLVM_VERSION: &str = env!("JADREN_LLVM_VERSION");

#[cfg(all(test, windows))]
mod differential;

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests {
    use inkwell::context::Context;

    use super::LLVM_VERSION;

    #[test]
    fn links_pinned_llvm_c_runtime() {
        let context = Context::create();
        assert_eq!(context.i32_type().print_to_string().to_string(), "i32");
        assert_eq!(LLVM_VERSION, "22.1.8");
    }
}
