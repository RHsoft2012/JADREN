use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::Path;

use inkwell::context::Context;
use inkwell::debug_info::{
    AsDIScope, DICompileUnit, DIFile, DIFlags, DIFlagsConstants, DILocation, DISubprogram,
    DWARFEmissionKind, DWARFSourceLanguage, DebugInfoBuilder,
};
use inkwell::module::{FlagBehavior, Module};
use inkwell::values::{AsValueRef, BasicValueEnum, FunctionValue, InstructionValue};
use jadren_determinism::normalize_path;
use jadren_jir::{Function, Linkage, TypeId};
use jadren_source::{SourceId, SourceManager, Span};

use crate::{LoweredTypeTable, TypeLoweringConfig};

const PRODUCER: &str = "Jadren LLVM backend 0.1";

/// Stable source inputs used to generate native debug metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugInfoConfig {
    compilation_directory: String,
    producer: String,
    optimized: bool,
    sources: BTreeMap<SourceId, DebugSource>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DebugSource {
    path: String,
    text: String,
}

impl DebugInfoConfig {
    /// Copies and lexically normalizes all source inputs for deterministic code generation.
    pub fn from_source_manager(
        sources: &SourceManager,
        compilation_directory: impl AsRef<Path>,
        optimized: bool,
    ) -> Result<Self, DebugInfoError> {
        if sources.is_empty() {
            return Err(DebugInfoError::EmptySources);
        }
        let compilation_directory = normalize_path(compilation_directory);
        validate_text("compilation directory", &compilation_directory)?;
        let mut copied = BTreeMap::new();
        for source in sources.iter() {
            let path = normalize_path(source.path());
            if path.is_empty() {
                return Err(DebugInfoError::EmptyPath(source.id()));
            }
            validate_text("source path", &path)?;
            copied.insert(
                source.id(),
                DebugSource {
                    path,
                    text: source.text().to_owned(),
                },
            );
        }
        Ok(Self {
            compilation_directory,
            producer: PRODUCER.to_owned(),
            optimized,
            sources: copied,
        })
    }

    /// Returns the normalized compilation directory embedded in the compile unit.
    #[must_use]
    pub fn compilation_directory(&self) -> &str {
        &self.compilation_directory
    }

    /// Returns the normalized path registered for one source file.
    #[must_use]
    pub fn source_path(&self, source: SourceId) -> Option<&str> {
        self.sources.get(&source).map(|source| source.path.as_str())
    }
}

/// Failure while constructing source-accurate LLVM debug metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DebugInfoError {
    EmptySources,
    EmptyPath(SourceId),
    InvalidText(&'static str),
    MissingSource(SourceId),
    InvalidOffset { source: SourceId, offset: usize },
    LocationOverflow { source: SourceId, offset: usize },
    MissingFunctionSpan(String),
    TypeMetadata(String),
}

impl fmt::Display for DebugInfoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySources => {
                formatter.write_str("debug info requires at least one source file")
            }
            Self::EmptyPath(source) => {
                write!(
                    formatter,
                    "debug source {} has an empty path",
                    source.index()
                )
            }
            Self::InvalidText(field) => write!(formatter, "{field} contains NUL"),
            Self::MissingSource(source) => {
                write!(
                    formatter,
                    "debug span references missing source {}",
                    source.index()
                )
            }
            Self::InvalidOffset { source, offset } => write!(
                formatter,
                "debug span offset {offset} is invalid for source {}",
                source.index()
            ),
            Self::LocationOverflow { source, offset } => write!(
                formatter,
                "debug location at offset {offset} in source {} exceeds LLVM limits",
                source.index()
            ),
            Self::MissingFunctionSpan(name) => {
                write!(formatter, "debug function `{name}` has no source span")
            }
            Self::TypeMetadata(message) => {
                write!(formatter, "debug type metadata failed: {message}")
            }
        }
    }
}

impl Error for DebugInfoError {}

fn validate_text(field: &'static str, text: &str) -> Result<(), DebugInfoError> {
    if text.contains('\0') {
        Err(DebugInfoError::InvalidText(field))
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(crate) struct FunctionDebugInfo<'ctx> {
    pub scope: DISubprogram<'ctx>,
    pub file: DIFile<'ctx>,
    pub location: DILocation<'ctx>,
}

pub(crate) struct DebugState<'ctx, 'config> {
    builder: DebugInfoBuilder<'ctx>,
    compile_unit: DICompileUnit<'ctx>,
    files: BTreeMap<SourceId, DIFile<'ctx>>,
    config: &'config DebugInfoConfig,
}

impl<'ctx, 'config> DebugState<'ctx, 'config> {
    pub fn create(
        context: &'ctx Context,
        module: &Module<'ctx>,
        target: &TypeLoweringConfig,
        config: &'config DebugInfoConfig,
    ) -> Result<Self, DebugInfoError> {
        let (&primary_id, primary) = config
            .sources
            .first_key_value()
            .ok_or(DebugInfoError::EmptySources)?;
        let (filename, directory) = split_source_path(&primary.path, &config.compilation_directory);
        let (builder, compile_unit) = module.create_debug_info_builder(
            true,
            DWARFSourceLanguage::Rust,
            filename,
            &directory,
            &config.producer,
            config.optimized,
            "",
            0,
            "",
            DWARFEmissionKind::Full,
            0,
            false,
            false,
            "",
            "",
        );
        add_module_flags(context, module, &target.target_triple);
        let mut files = BTreeMap::new();
        files.insert(primary_id, compile_unit.get_file());
        for (&id, source) in &config.sources {
            if id == primary_id {
                continue;
            }
            let (filename, directory) =
                split_source_path(&source.path, &config.compilation_directory);
            files.insert(id, builder.create_file(filename, &directory));
        }
        Ok(Self {
            builder,
            compile_unit,
            files,
            config,
        })
    }

    pub fn create_function(
        &self,
        context: &'ctx Context,
        function: &Function,
        llvm_function: FunctionValue<'ctx>,
    ) -> Result<FunctionDebugInfo<'ctx>, DebugInfoError> {
        let span = function
            .span
            .ok_or_else(|| DebugInfoError::MissingFunctionSpan(function.name.clone()))?;
        let (file, line, column) = self.resolve(span)?;
        let function_type = self
            .builder
            .create_subroutine_type(file, None, &[], DIFlags::ZERO);
        let scope = self.builder.create_function(
            self.compile_unit.as_debug_info_scope(),
            &function.name,
            Some(llvm_function.get_name().to_string_lossy().as_ref()),
            file,
            line,
            function_type,
            function.linkage == Linkage::Internal,
            true,
            line,
            DIFlags::ZERO,
            self.config.optimized,
        );
        llvm_function.set_subprogram(scope);
        Ok(FunctionDebugInfo {
            scope,
            file,
            location: self.builder.create_debug_location(
                context,
                line,
                column,
                scope.as_debug_info_scope(),
                None,
            ),
        })
    }

    pub fn location(
        &self,
        context: &'ctx Context,
        span: Span,
        function: FunctionDebugInfo<'ctx>,
    ) -> Result<DILocation<'ctx>, DebugInfoError> {
        let (file, line, column) = self.resolve(span)?;
        let scope = if file == function.file {
            function.scope.as_debug_info_scope()
        } else {
            self.builder
                .create_lexical_block(function.scope.as_debug_info_scope(), file, line, column)
                .as_debug_info_scope()
        };
        Ok(self
            .builder
            .create_debug_location(context, line, column, scope, None))
    }

    pub fn insert_parameters(
        &self,
        function: &Function,
        llvm_function: FunctionValue<'ctx>,
        info: FunctionDebugInfo<'ctx>,
        first_instruction: InstructionValue<'ctx>,
        types: &LoweredTypeTable<'ctx>,
    ) -> Result<(), DebugInfoError> {
        let (_, line, _) = self.resolve(function.span.expect("debug function span checked"))?;
        for (index, parameter) in function.parameters.iter().enumerate() {
            let Some(name) = parameter.name.as_deref() else {
                continue;
            };
            let value = llvm_function
                .get_nth_param(u32::try_from(index).map_err(|_| {
                    DebugInfoError::LocationOverflow {
                        source: function.span.expect("debug function span checked").source,
                        offset: index,
                    }
                })?)
                .expect("verified LLVM parameter exists");
            let ty = self.parameter_type(parameter.ty, types)?;
            let variable = self.builder.create_parameter_variable(
                info.scope.as_debug_info_scope(),
                name,
                u32::try_from(index + 1).expect("parameter number fits u32"),
                info.file,
                line,
                ty,
                true,
                DIFlags::ZERO,
            );
            insert_debug_value_record(
                &self.builder,
                value,
                variable,
                info.location,
                first_instruction,
            );
        }
        Ok(())
    }

    fn parameter_type(
        &self,
        ty: TypeId,
        types: &LoweredTypeTable<'ctx>,
    ) -> Result<inkwell::debug_info::DIType<'ctx>, DebugInfoError> {
        let lowered = types.get(ty).and_then(|ty| ty.as_basic()).ok_or_else(|| {
            DebugInfoError::TypeMetadata(format!("missing type %t{}", ty.index()))
        })?;
        let bits = types
            .target_data()
            .get_store_size(&lowered)
            .saturating_mul(8);
        self.builder
            .create_basic_type(&format!("jadren.t{}", ty.index()), bits, 0, DIFlags::ZERO)
            .map(|ty| ty.as_type())
            .map_err(|error| DebugInfoError::TypeMetadata(error.to_string()))
    }

    fn resolve(&self, span: Span) -> Result<(DIFile<'ctx>, u32, u32), DebugInfoError> {
        let source = self
            .config
            .sources
            .get(&span.source)
            .ok_or(DebugInfoError::MissingSource(span.source))?;
        let (line, column) =
            source_location(&source.text, span.start).ok_or(DebugInfoError::InvalidOffset {
                source: span.source,
                offset: span.start,
            })?;
        let line = u32::try_from(line).map_err(|_| DebugInfoError::LocationOverflow {
            source: span.source,
            offset: span.start,
        })?;
        let column = u32::try_from(column).map_err(|_| DebugInfoError::LocationOverflow {
            source: span.source,
            offset: span.start,
        })?;
        let file = self
            .files
            .get(&span.source)
            .copied()
            .ok_or(DebugInfoError::MissingSource(span.source))?;
        Ok((file, line, column))
    }

    pub fn finalize(&self) {
        self.builder.finalize();
    }
}

// JADREN-UNSAFE-AUDIT: this is the sole LLVM-C debug metadata escape hatch;
// builder and instruction pointers are borrowed for one call and the returned
// non-owning record is deliberately discarded.
#[allow(unsafe_code)]
fn insert_debug_value_record<'ctx>(
    builder: &DebugInfoBuilder<'ctx>,
    value: BasicValueEnum<'ctx>,
    variable: inkwell::debug_info::DILocalVariable<'ctx>,
    location: DILocation<'ctx>,
    instruction: InstructionValue<'ctx>,
) {
    let expression = builder.create_expression(Vec::new());
    // LLVM 22 returns an opaque DbgRecord, not an LLVM instruction. Inkwell 0.9
    // incorrectly wraps that pointer as InstructionValue on this API, so use the
    // matching LLVM-C function and deliberately discard the non-owning record.
    unsafe {
        let _ = llvm_sys::debuginfo::LLVMDIBuilderInsertDbgValueRecordBefore(
            builder.as_mut_ptr(),
            value.as_value_ref(),
            variable.as_mut_ptr(),
            expression.as_mut_ptr(),
            location.as_mut_ptr(),
            instruction.as_value_ref(),
        );
    }
}

fn add_module_flags<'ctx>(context: &'ctx Context, module: &Module<'ctx>, target: &str) {
    module.add_basic_value_flag(
        "Debug Info Version",
        FlagBehavior::Warning,
        context.i32_type().const_int(3, false),
    );
    if target.contains("windows-msvc") {
        module.add_basic_value_flag(
            "CodeView",
            FlagBehavior::Warning,
            context.i32_type().const_int(1, false),
        );
    } else {
        module.add_basic_value_flag(
            "Dwarf Version",
            FlagBehavior::Warning,
            context.i32_type().const_int(5, false),
        );
    }
}

fn split_source_path<'a>(path: &'a str, compilation_directory: &str) -> (&'a str, String) {
    let (directory, filename) = path.rsplit_once('/').unwrap_or(("", path));
    let absolute = path.starts_with('/') || path.as_bytes().get(1) == Some(&b':');
    let directory = if absolute {
        directory.to_owned()
    } else if directory.is_empty() {
        compilation_directory.to_owned()
    } else if compilation_directory.is_empty() {
        directory.to_owned()
    } else {
        format!("{compilation_directory}/{directory}")
    };
    (filename, directory)
}

fn source_location(text: &str, offset: usize) -> Option<(usize, usize)> {
    if offset > text.len() || !text.is_char_boundary(offset) {
        return None;
    }
    let before = &text[..offset];
    let line = before.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    let column = text[line_start..offset].chars().count() + 1;
    Some((line, column))
}

#[cfg(test)]
mod tests {
    use jadren_source::SourceManager;

    use super::{DebugInfoConfig, DebugInfoError, source_location, split_source_path};

    #[test]
    fn normalizes_paths_and_maps_unicode_locations() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(r"C:\work\.\src\main.jdn", "fn main() {\n  ľ\n}\n")
            .expect("source ID");
        let config = DebugInfoConfig::from_source_manager(&sources, r"D:\build\.", false)
            .expect("debug config");

        assert_eq!(config.compilation_directory(), "d:/build");
        assert_eq!(config.source_path(id), Some("c:/work/src/main.jdn"));
        assert_eq!(source_location("a\nľ", 4), Some((2, 2)));
        assert_eq!(
            split_source_path("src/main.jdn", "d:/build"),
            ("main.jdn", "d:/build/src".to_owned())
        );
    }

    #[test]
    fn rejects_empty_source_set_and_invalid_offsets() {
        assert_eq!(
            DebugInfoConfig::from_source_manager(&SourceManager::new(), ".", false),
            Err(DebugInfoError::EmptySources)
        );
        assert_eq!(source_location("ľ", 1), None);
    }
}
