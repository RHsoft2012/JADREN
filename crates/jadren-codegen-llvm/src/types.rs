use std::error::Error;
use std::fmt;
use std::num::NonZeroU32;

use inkwell::AddressSpace as LlvmAddressSpace;
use inkwell::context::Context;
use inkwell::targets::TargetData;
use inkwell::types::{
    BasicMetadataTypeEnum, BasicType, BasicTypeEnum, FunctionType, IntType, VoidType,
};
use jadren_jir::{AddressSpace, Function, Module, Type, TypeId, VerificationError, verify};

/// Data layout emitted by the pinned LLVM 22.1.8 Windows x86-64 toolchain.
pub const X86_64_WINDOWS_MSVC_DATA_LAYOUT: &str =
    "e-m:w-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128";
/// Data layout emitted by LLVM 22.1.8 for x86-64 Linux GNU.
pub const X86_64_LINUX_GNU_DATA_LAYOUT: &str =
    "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128";
/// Data layout emitted by the Android NDK AArch64 clang target at API 24.
pub const AARCH64_ANDROID_DATA_LAYOUT: &str =
    "e-m:e-p270:32:32-p271:32:32-p272:64:64-i8:8:32-i16:16:32-i64:64-i128:128-n32:64-S128-Fn32";

/// Target-specific choices required while lowering target-neutral JIR types.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypeLoweringConfig {
    pub target_triple: String,
    pub data_layout: String,
}

impl TypeLoweringConfig {
    #[must_use]
    pub fn x86_64_windows_msvc() -> Self {
        Self {
            target_triple: "x86_64-pc-windows-msvc".to_owned(),
            data_layout: X86_64_WINDOWS_MSVC_DATA_LAYOUT.to_owned(),
        }
    }

    #[must_use]
    pub fn x86_64_linux_gnu() -> Self {
        Self {
            target_triple: "x86_64-unknown-linux-gnu".to_owned(),
            data_layout: X86_64_LINUX_GNU_DATA_LAYOUT.to_owned(),
        }
    }

    /// Returns the portable AArch64 Android API 24 scalar policy.
    #[must_use]
    pub fn aarch64_android() -> Self {
        Self {
            target_triple: "aarch64-unknown-linux-android24".to_owned(),
            data_layout: AARCH64_ANDROID_DATA_LAYOUT.to_owned(),
        }
    }
}

impl Default for TypeLoweringConfig {
    fn default() -> Self {
        Self::x86_64_windows_msvc()
    }
}

/// LLVM type corresponding to one canonical JIR type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoweredType<'ctx> {
    Void(VoidType<'ctx>),
    Basic(BasicTypeEnum<'ctx>),
}

impl<'ctx> LoweredType<'ctx> {
    #[must_use]
    pub const fn as_basic(self) -> Option<BasicTypeEnum<'ctx>> {
        match self {
            Self::Void(_) => None,
            Self::Basic(ty) => Some(ty),
        }
    }
}

/// Concrete tagged-union representation selected from the LLVM target layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnumLayout {
    pub tag_field: u32,
    pub payload_field: u32,
    pub payload_offset: u64,
    pub payload_size: u64,
    pub payload_alignment: u32,
}

/// Complete dense JIR-to-LLVM type table for one module.
pub struct LoweredTypeTable<'ctx> {
    context: &'ctx Context,
    types: Vec<LoweredType<'ctx>>,
    enum_layouts: Vec<Option<EnumLayout>>,
    target_data: TargetData,
    c_abi_aggregate_indirect: bool,
    c_abi_aggregate_register_bytes: u64,
    c_abi_aggregate_register_return: bool,
}

impl<'ctx> LoweredTypeTable<'ctx> {
    #[must_use]
    pub fn get(&self, ty: TypeId) -> Option<LoweredType<'ctx>> {
        self.types.get(ty.index()).copied()
    }

    #[must_use]
    pub fn enum_layout(&self, ty: TypeId) -> Option<EnumLayout> {
        self.enum_layouts.get(ty.index()).copied().flatten()
    }

    #[must_use]
    pub const fn target_data(&self) -> &TargetData {
        &self.target_data
    }

    /// Windows x64 C ABI returns larger exported/imported aggregates through
    /// a caller-owned result pointer. Internal Jadren functions keep the
    /// ordinary LLVM aggregate return so optimisation and inlining remain
    /// unchanged inside a module.
    #[must_use]
    pub fn uses_c_aggregate_return(&self, function: &Function) -> bool {
        if !self.c_abi_aggregate_indirect
            || matches!(function.linkage, jadren_jir::Linkage::Internal)
        {
            return false;
        }
        let Some(BasicTypeEnum::StructType(_) | BasicTypeEnum::ArrayType(_)) =
            self.get(function.result).and_then(LoweredType::as_basic)
        else {
            return false;
        };
        self.get(function.result)
            .and_then(LoweredType::as_basic)
            .is_some_and(|ty| {
                self.target_data.get_store_size(&ty) > self.c_abi_aggregate_register_bytes
            })
    }

    /// The platform C ABIs return small aggregates as one integer register.
    /// LLVM's ordinary aggregate return uses multiple registers for values
    /// such as `{ i16, i8 }`, while Clang packs the same `repr(C)` value into
    /// one 32-bit register. Adapt only the external signature.
    #[must_use]
    pub fn c_aggregate_register_type(&self, function: &Function) -> Option<IntType<'ctx>> {
        if !self.c_abi_aggregate_register_return
            || matches!(function.linkage, jadren_jir::Linkage::Internal)
            || self.uses_c_aggregate_return(function)
        {
            return None;
        }
        let Some(BasicTypeEnum::StructType(_) | BasicTypeEnum::ArrayType(_)) =
            self.get(function.result).and_then(LoweredType::as_basic)
        else {
            return None;
        };
        let size = self
            .get(function.result)
            .and_then(LoweredType::as_basic)
            .map(|ty| self.target_data.get_store_size(&ty))?;
        let bits = match size {
            1 => 8,
            2 => 16,
            3..=4 => 32,
            5..=8 => 64,
            _ => return None,
        };
        Some(
            self.context
                .custom_width_int_type(NonZeroU32::new(bits).expect("non-zero integer width"))
                .expect("supported C ABI aggregate register width"),
        )
    }

    /// Lowers a verified JIR function signature without emitting its body.
    pub fn function_type(&self, function: &Function) -> Result<FunctionType<'ctx>, TypeLowerError> {
        let mut parameters = function
            .parameters
            .iter()
            .map(|parameter| {
                self.required_basic(parameter.ty, "function parameter")
                    .map(BasicMetadataTypeEnum::from)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if self.uses_c_aggregate_return(function) {
            self.required_basic(function.result, "function result")?;
            let result_pointer = self.context.ptr_type(LlvmAddressSpace::default());
            parameters.insert(0, result_pointer.into());
            return Ok(self.context.void_type().fn_type(&parameters, false));
        }
        if let Some(return_type) = self.c_aggregate_register_type(function) {
            return Ok(return_type.fn_type(&parameters, false));
        }
        match self.required(function.result)? {
            LoweredType::Void(ty) => Ok(ty.fn_type(&parameters, false)),
            LoweredType::Basic(ty) => Ok(ty.fn_type(&parameters, false)),
        }
    }

    /// Lowers a first-class JIR function-pointer signature.
    ///
    /// Aggregate returns are intentionally rejected here until the function
    /// pointer carries an explicit C-ABI/sret convention.  A plain pointer
    /// cannot safely alias both Jadren's internal aggregate return and a
    /// platform C aggregate callback signature.
    pub fn function_pointer_type(
        &self,
        module: &Module,
        ty: TypeId,
    ) -> Result<FunctionType<'ctx>, TypeLowerError> {
        let Type::Function { parameters, result } = module
            .types
            .get(ty.index())
            .ok_or(TypeLowerError::MissingType(ty))?
        else {
            return Err(TypeLowerError::NotFunctionPointer(ty));
        };
        let parameters = parameters
            .iter()
            .map(|parameter| {
                self.required_basic(*parameter, "function pointer parameter")
                    .map(BasicMetadataTypeEnum::from)
            })
            .collect::<Result<Vec<_>, _>>()?;
        match self.required(*result)? {
            LoweredType::Void(ty) => Ok(ty.fn_type(&parameters, false)),
            LoweredType::Basic(BasicTypeEnum::StructType(_) | BasicTypeEnum::ArrayType(_)) => {
                Err(TypeLowerError::UnsupportedFunctionPointerResult { ty: *result })
            }
            LoweredType::Basic(ty) => Ok(ty.fn_type(&parameters, false)),
        }
    }

    fn required(&self, ty: TypeId) -> Result<LoweredType<'ctx>, TypeLowerError> {
        self.get(ty).ok_or(TypeLowerError::MissingType(ty))
    }

    fn required_basic(
        &self,
        ty: TypeId,
        usage: &'static str,
    ) -> Result<BasicTypeEnum<'ctx>, TypeLowerError> {
        self.required(ty)?
            .as_basic()
            .ok_or(TypeLowerError::UnitInValue { ty, usage })
    }
}

/// Deterministic failure to represent verified JIR in the selected LLVM target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TypeLowerError {
    InvalidJir(Vec<VerificationError>),
    MissingType(TypeId),
    UnitInValue {
        ty: TypeId,
        usage: &'static str,
    },
    NotFunctionPointer(TypeId),
    UnsupportedFunctionPointerResult {
        ty: TypeId,
    },
    UnsupportedFloat {
        ty: TypeId,
        bits: u16,
    },
    UnsupportedArrayLength {
        ty: TypeId,
        length: u64,
    },
    UnsupportedVectorElement {
        ty: TypeId,
        element: TypeId,
    },
    UnsupportedAddressSpace {
        ty: TypeId,
        address_space: AddressSpace,
    },
    RecursiveValueType(TypeId),
    IntegerWidthRejected {
        ty: TypeId,
        bits: u16,
    },
    NominalBodyAlreadyDefined(TypeId),
}

impl fmt::Display for TypeLowerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJir(errors) => write!(
                formatter,
                "JIR verifier rejected type lowering with {} error(s): {}",
                errors.len(),
                errors
                    .first()
                    .map_or("unknown verifier error", |error| error.message.as_str())
            ),
            Self::MissingType(ty) => write!(formatter, "missing JIR type %t{}", ty.index()),
            Self::UnitInValue { ty, usage } => {
                write!(
                    formatter,
                    "Unit type %t{} cannot be used as {usage}",
                    ty.index()
                )
            }
            Self::NotFunctionPointer(ty) => {
                write!(
                    formatter,
                    "JIR type %t{} is not a function pointer",
                    ty.index()
                )
            }
            Self::UnsupportedFunctionPointerResult { ty } => write!(
                formatter,
                "function pointer result %t{} requires an explicit ABI convention",
                ty.index()
            ),
            Self::UnsupportedFloat { ty, bits } => write!(
                formatter,
                "JIR type %t{} uses unsupported f{bits}",
                ty.index()
            ),
            Self::UnsupportedArrayLength { ty, length } => write!(
                formatter,
                "JIR array %t{} length {length} exceeds the LLVM API limit",
                ty.index()
            ),
            Self::UnsupportedVectorElement { ty, element } => write!(
                formatter,
                "JIR vector %t{} has unsupported element %t{}",
                ty.index(),
                element.index()
            ),
            Self::UnsupportedAddressSpace { ty, address_space } => write!(
                formatter,
                "JIR pointer %t{} uses {address_space:?}, which is not valid in the CPU backend",
                ty.index()
            ),
            Self::RecursiveValueType(ty) => write!(
                formatter,
                "JIR type %t{} is recursive by value; recursion requires a pointer",
                ty.index()
            ),
            Self::IntegerWidthRejected { ty, bits } => write!(
                formatter,
                "LLVM rejected i{bits} for JIR type %t{}",
                ty.index()
            ),
            Self::NominalBodyAlreadyDefined(ty) => write!(
                formatter,
                "LLVM nominal type for %t{} was defined more than once",
                ty.index()
            ),
        }
    }
}

impl Error for TypeLowerError {}

/// Lowers the complete canonical type table after the mandatory JIR verifier.
pub fn lower_types<'ctx>(
    context: &'ctx Context,
    module: &Module,
    config: &TypeLoweringConfig,
) -> Result<LoweredTypeTable<'ctx>, TypeLowerError> {
    let verifier_errors = verify(module);
    if !verifier_errors.is_empty() {
        return Err(TypeLowerError::InvalidJir(verifier_errors));
    }
    validate_value_recursion(module)?;

    let target_data = TargetData::create(&config.data_layout);
    let mut lowerer = TypeLowerer::new(context, module, &target_data);
    lowerer.predeclare_nominal_types();
    for index in 0..module.types.len() {
        lowerer.lower(TypeId::new(index))?;
    }
    Ok(LoweredTypeTable {
        context,
        types: lowerer
            .lowered
            .into_iter()
            .enumerate()
            .map(|(index, ty)| ty.ok_or(TypeLowerError::MissingType(TypeId::new(index))))
            .collect::<Result<_, _>>()?,
        enum_layouts: lowerer.enum_layouts,
        target_data,
        c_abi_aggregate_indirect: matches!(
            config.target_triple.as_str(),
            "x86_64-pc-windows-msvc" | "x86_64-unknown-linux-gnu"
        ),
        c_abi_aggregate_register_bytes: if config.target_triple == "x86_64-unknown-linux-gnu" {
            16
        } else {
            8
        },
        c_abi_aggregate_register_return: matches!(
            config.target_triple.as_str(),
            "x86_64-pc-windows-msvc"
                | "x86_64-unknown-linux-gnu"
                | "aarch64-unknown-linux-android24"
        ),
    })
}

struct TypeLowerer<'ctx, 'module, 'layout> {
    context: &'ctx Context,
    module: &'module Module,
    target_data: &'layout TargetData,
    lowered: Vec<Option<LoweredType<'ctx>>>,
    enum_layouts: Vec<Option<EnumLayout>>,
    state: Vec<LowerState>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum LowerState {
    Pending,
    Visiting,
    Done,
}

impl<'ctx, 'module, 'layout> TypeLowerer<'ctx, 'module, 'layout> {
    fn new(
        context: &'ctx Context,
        module: &'module Module,
        target_data: &'layout TargetData,
    ) -> Self {
        Self {
            context,
            module,
            target_data,
            lowered: vec![None; module.types.len()],
            enum_layouts: vec![None; module.types.len()],
            state: vec![LowerState::Pending; module.types.len()],
        }
    }

    fn predeclare_nominal_types(&mut self) {
        for (index, ty) in self.module.types.iter().enumerate() {
            let name = match ty {
                Type::NominalStruct { identity, .. } => {
                    Some(format!("jadren.record.{identity:016x}.t{index}"))
                }
                Type::NominalEnum { identity, .. } => {
                    Some(format!("jadren.enum.{identity:016x}.t{index}"))
                }
                _ => None,
            };
            if let Some(name) = name {
                self.lowered[index] = Some(LoweredType::Basic(
                    self.context.opaque_struct_type(&name).into(),
                ));
            }
        }
    }

    fn lower(&mut self, id: TypeId) -> Result<LoweredType<'ctx>, TypeLowerError> {
        match self.state.get(id.index()).copied() {
            Some(LowerState::Done) => {
                return self.lowered[id.index()].ok_or(TypeLowerError::MissingType(id));
            }
            Some(LowerState::Visiting) => return Err(TypeLowerError::RecursiveValueType(id)),
            Some(LowerState::Pending) => {}
            None => return Err(TypeLowerError::MissingType(id)),
        }
        self.state[id.index()] = LowerState::Visiting;
        let source = self
            .module
            .types
            .get(id.index())
            .ok_or(TypeLowerError::MissingType(id))?;
        let lowered = match source {
            Type::Unit => LoweredType::Void(self.context.void_type()),
            Type::RegionHandle => {
                LoweredType::Basic(self.context.ptr_type(LlvmAddressSpace::default()).into())
            }
            Type::Bool => LoweredType::Basic(self.context.bool_type().into()),
            Type::Integer { bits, .. } => {
                let width = NonZeroU32::new(u32::from(*bits)).ok_or(
                    TypeLowerError::IntegerWidthRejected {
                        ty: id,
                        bits: *bits,
                    },
                )?;
                let ty = self.context.custom_width_int_type(width).map_err(|_| {
                    TypeLowerError::IntegerWidthRejected {
                        ty: id,
                        bits: *bits,
                    }
                })?;
                LoweredType::Basic(ty.into())
            }
            Type::Float { bits } => LoweredType::Basic(match bits {
                16 => self.context.f16_type().into(),
                32 => self.context.f32_type().into(),
                64 => self.context.f64_type().into(),
                _ => {
                    return Err(TypeLowerError::UnsupportedFloat {
                        ty: id,
                        bits: *bits,
                    });
                }
            }),
            Type::Pointer { address_space, .. } => LoweredType::Basic(
                self.context
                    .ptr_type(self.lower_address_space(id, *address_space)?)
                    .into(),
            ),
            Type::Function { parameters, result } => {
                let _parameters = parameters
                    .iter()
                    .map(|parameter| {
                        self.lower_basic(*parameter, "function pointer parameter")
                            .map(BasicMetadataTypeEnum::from)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                match self.lower(*result)? {
                    LoweredType::Void(_) => {}
                    LoweredType::Basic(
                        BasicTypeEnum::StructType(_) | BasicTypeEnum::ArrayType(_),
                    ) => {
                        return Err(TypeLowerError::UnsupportedFunctionPointerResult {
                            ty: *result,
                        });
                    }
                    LoweredType::Basic(_) => {}
                }
                LoweredType::Basic(self.context.ptr_type(LlvmAddressSpace::default()).into())
            }
            Type::Array { element, length } => {
                let length =
                    u32::try_from(*length).map_err(|_| TypeLowerError::UnsupportedArrayLength {
                        ty: id,
                        length: *length,
                    })?;
                let element = self.lower_basic(*element, "array element")?;
                LoweredType::Basic(element.array_type(length).into())
            }
            Type::Struct { fields } => {
                let fields = self.lower_fields(fields, "struct field")?;
                LoweredType::Basic(self.context.struct_type(&fields, false).into())
            }
            Type::NominalStruct { fields, .. } => {
                let body = self.lower_fields(fields, "nominal struct field")?;
                let nominal = self.lowered[id.index()]
                    .and_then(LoweredType::as_basic)
                    .ok_or(TypeLowerError::MissingType(id))?
                    .into_struct_type();
                if !nominal.set_body(&body, false) {
                    return Err(TypeLowerError::NominalBodyAlreadyDefined(id));
                }
                LoweredType::Basic(nominal.into())
            }
            Type::Enum { variants } => {
                let (body, layout) = self.lower_enum_body(id, variants)?;
                self.enum_layouts[id.index()] = Some(layout);
                LoweredType::Basic(self.context.struct_type(&body, false).into())
            }
            Type::NominalEnum { variants, .. } => {
                let (body, layout) = self.lower_enum_body(id, variants)?;
                let nominal = self.lowered[id.index()]
                    .and_then(LoweredType::as_basic)
                    .ok_or(TypeLowerError::MissingType(id))?
                    .into_struct_type();
                if !nominal.set_body(&body, false) {
                    return Err(TypeLowerError::NominalBodyAlreadyDefined(id));
                }
                self.enum_layouts[id.index()] = Some(layout);
                LoweredType::Basic(nominal.into())
            }
            Type::Vector { element, lanes } => {
                let element_type = self.lower_basic(*element, "vector element")?;
                let vector = match element_type {
                    BasicTypeEnum::IntType(ty) => ty.vec_type(u32::from(*lanes)),
                    BasicTypeEnum::FloatType(ty) => ty.vec_type(u32::from(*lanes)),
                    BasicTypeEnum::PointerType(ty) => ty.vec_type(u32::from(*lanes)),
                    _ => {
                        return Err(TypeLowerError::UnsupportedVectorElement {
                            ty: id,
                            element: *element,
                        });
                    }
                };
                LoweredType::Basic(vector.into())
            }
        };
        self.lowered[id.index()] = Some(lowered);
        self.state[id.index()] = LowerState::Done;
        Ok(lowered)
    }

    fn lower_basic(
        &mut self,
        id: TypeId,
        usage: &'static str,
    ) -> Result<BasicTypeEnum<'ctx>, TypeLowerError> {
        self.lower(id)?
            .as_basic()
            .ok_or(TypeLowerError::UnitInValue { ty: id, usage })
    }

    fn lower_fields(
        &mut self,
        fields: &[TypeId],
        usage: &'static str,
    ) -> Result<Vec<BasicTypeEnum<'ctx>>, TypeLowerError> {
        fields
            .iter()
            .map(|field| self.lower_basic(*field, usage))
            .collect()
    }

    fn lower_enum_body(
        &mut self,
        id: TypeId,
        variants: &[Vec<TypeId>],
    ) -> Result<(Vec<BasicTypeEnum<'ctx>>, EnumLayout), TypeLowerError> {
        let mut payload_size = 0;
        let mut payload_alignment = 1;
        let mut alignment_anchor: BasicTypeEnum<'ctx> = self.context.i8_type().into();
        for variant in variants {
            let fields = self.lower_fields(variant, "enum payload field")?;
            let payload: BasicTypeEnum<'ctx> = self.context.struct_type(&fields, false).into();
            let size = self.target_data.get_store_size(&payload);
            let alignment = self.target_data.get_abi_alignment(&payload);
            payload_size = payload_size.max(size);
            if alignment > payload_alignment {
                payload_alignment = alignment;
                alignment_anchor = payload;
            }
        }
        let payload_length =
            u32::try_from(payload_size).map_err(|_| TypeLowerError::UnsupportedArrayLength {
                ty: id,
                length: payload_size,
            })?;
        let tag: BasicTypeEnum<'ctx> = self.context.i32_type().into();
        let tag_size = self.target_data.get_store_size(&tag);
        let alignment = u64::from(payload_alignment);
        let padding = (alignment - tag_size % alignment) % alignment;
        let padding_length = u32::try_from(padding).expect("LLVM alignment is a u32");

        let mut body = vec![tag];
        if padding_length > 0 {
            body.push(self.context.i8_type().array_type(padding_length).into());
        }
        let payload_field = u32::try_from(body.len()).expect("enum body has fewer than u32 fields");
        body.push(self.context.i8_type().array_type(payload_length).into());
        body.push(alignment_anchor.array_type(0).into());
        Ok((
            body,
            EnumLayout {
                tag_field: 0,
                payload_field,
                payload_offset: tag_size + padding,
                payload_size,
                payload_alignment,
            },
        ))
    }

    fn lower_address_space(
        &self,
        id: TypeId,
        address_space: AddressSpace,
    ) -> Result<LlvmAddressSpace, TypeLowerError> {
        match address_space {
            AddressSpace::Generic
            | AddressSpace::Stack
            | AddressSpace::Heap
            | AddressSpace::Region
            | AddressSpace::Global => Ok(LlvmAddressSpace::default()),
            AddressSpace::Workgroup | AddressSpace::Uniform | AddressSpace::Storage => {
                Err(TypeLowerError::UnsupportedAddressSpace {
                    ty: id,
                    address_space,
                })
            }
        }
    }
}

fn validate_value_recursion(module: &Module) -> Result<(), TypeLowerError> {
    let mut state = vec![LowerState::Pending; module.types.len()];
    for index in 0..module.types.len() {
        visit_value_type(module, TypeId::new(index), &mut state)?;
    }
    Ok(())
}

fn visit_value_type(
    module: &Module,
    id: TypeId,
    state: &mut [LowerState],
) -> Result<(), TypeLowerError> {
    match state.get(id.index()).copied() {
        Some(LowerState::Done) => return Ok(()),
        Some(LowerState::Visiting) => return Err(TypeLowerError::RecursiveValueType(id)),
        Some(LowerState::Pending) => {}
        None => return Err(TypeLowerError::MissingType(id)),
    }
    state[id.index()] = LowerState::Visiting;
    let dependencies: Vec<TypeId> = match module
        .types
        .get(id.index())
        .ok_or(TypeLowerError::MissingType(id))?
    {
        Type::Function { parameters, result } => parameters
            .iter()
            .copied()
            .chain(std::iter::once(*result))
            .collect(),
        Type::Array { element, .. } | Type::Vector { element, .. } => vec![*element],
        Type::Struct { fields } | Type::NominalStruct { fields, .. } => fields.clone(),
        Type::Enum { variants } | Type::NominalEnum { variants, .. } => {
            variants.iter().flatten().copied().collect()
        }
        Type::Pointer { .. }
        | Type::Unit
        | Type::RegionHandle
        | Type::Bool
        | Type::Integer { .. }
        | Type::Float { .. } => Vec::new(),
    };
    for dependency in dependencies {
        visit_value_type(module, dependency, state)?;
    }
    state[id.index()] = LowerState::Done;
    Ok(())
}

#[cfg(test)]
mod tests {
    use inkwell::context::Context;
    use jadren_jir::{
        AddressSpace, Function, FunctionId, Linkage, Module, Parameter, Type, TypeId, ValueId,
    };

    use super::{EnumLayout, TypeLowerError, TypeLoweringConfig, lower_types};

    #[test]
    fn lowers_scalars_aggregates_vectors_pointers_and_function_signatures() {
        let module = Module {
            types: vec![
                Type::Unit,
                Type::Bool,
                Type::Integer {
                    signed: true,
                    bits: 32,
                },
                Type::Float { bits: 32 },
                Type::Pointer {
                    pointee: TypeId::new(2),
                    address_space: AddressSpace::Stack,
                },
                Type::Array {
                    element: TypeId::new(2),
                    length: 4,
                },
                Type::Vector {
                    element: TypeId::new(3),
                    lanes: 8,
                },
                Type::Struct {
                    fields: vec![TypeId::new(1), TypeId::new(2), TypeId::new(4)],
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "convert".to_owned(),
                linkage: Linkage::Import,
                parameters: vec![Parameter {
                    value: ValueId::new(0),
                    ty: TypeId::new(2),
                    name: Some("value".to_owned()),
                }],
                result: TypeId::new(3),
                blocks: Vec::new(),
                span: None,
            }],
        };
        let context = Context::create();
        let table = lower_types(&context, &module, &TypeLoweringConfig::default())
            .expect("valid type table");

        let rendered = (0..module.types.len())
            .map(|index| {
                table
                    .get(TypeId::new(index))
                    .expect("dense type")
                    .as_basic()
                    .map_or_else(|| "void".to_owned(), |ty| ty.print_to_string().to_string())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            rendered,
            [
                "void",
                "i1",
                "i32",
                "float",
                "ptr",
                "[4 x i32]",
                "<8 x float>",
                "{ i1, i32, ptr }",
            ]
        );
        assert_eq!(
            table
                .function_type(&module.functions[0])
                .expect("function type")
                .print_to_string()
                .to_string(),
            "float (i32)"
        );
    }

    #[test]
    fn lowers_windows_c_aggregate_returns_through_result_pointer() {
        let module = Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: false,
                    bits: 64,
                },
                Type::Struct {
                    fields: vec![TypeId::new(1), TypeId::new(1)],
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "scan".to_owned(),
                linkage: Linkage::Import,
                parameters: Vec::new(),
                result: TypeId::new(2),
                blocks: Vec::new(),
                span: None,
            }],
        };
        let context = Context::create();
        let table = lower_types(&context, &module, &TypeLoweringConfig::default())
            .expect("valid aggregate type table");
        assert!(table.uses_c_aggregate_return(&module.functions[0]));
        assert_eq!(
            table
                .function_type(&module.functions[0])
                .expect("aggregate function type")
                .print_to_string()
                .to_string(),
            "void (ptr)"
        );

        let linux = lower_types(&context, &module, &TypeLoweringConfig::x86_64_linux_gnu())
            .expect("valid Linux aggregate type table");
        assert!(!linux.uses_c_aggregate_return(&module.functions[0]));
    }

    #[test]
    fn lowers_x86_64_and_android_c_small_aggregate_as_packed_register() {
        let module = Module {
            types: vec![
                Type::Integer {
                    signed: false,
                    bits: 16,
                },
                Type::Integer {
                    signed: false,
                    bits: 8,
                },
                Type::Struct {
                    fields: vec![TypeId::new(0), TypeId::new(1)],
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "diagnostic".to_owned(),
                linkage: Linkage::Import,
                parameters: vec![
                    Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(0),
                        name: None,
                    },
                    Parameter {
                        value: ValueId::new(1),
                        ty: TypeId::new(1),
                        name: None,
                    },
                ],
                result: TypeId::new(2),
                blocks: Vec::new(),
                span: None,
            }],
        };
        let context = Context::create();
        let table = lower_types(&context, &module, &TypeLoweringConfig::default())
            .expect("valid small aggregate type table");
        assert!(!table.uses_c_aggregate_return(&module.functions[0]));
        assert_eq!(
            table
                .c_aggregate_register_type(&module.functions[0])
                .expect("packed register type")
                .get_bit_width(),
            32
        );
        assert_eq!(
            table
                .function_type(&module.functions[0])
                .expect("small aggregate function type")
                .print_to_string()
                .to_string(),
            "i32 (i16, i8)"
        );
        let linux = lower_types(&context, &module, &TypeLoweringConfig::x86_64_linux_gnu())
            .expect("valid Linux small aggregate type table");
        assert_eq!(
            linux
                .c_aggregate_register_type(&module.functions[0])
                .expect("Linux packed register type")
                .get_bit_width(),
            32
        );
        assert_eq!(
            linux
                .function_type(&module.functions[0])
                .expect("Linux small aggregate function type")
                .print_to_string()
                .to_string(),
            "i32 (i16, i8)"
        );
        let android = lower_types(&context, &module, &TypeLoweringConfig::aarch64_android())
            .expect("valid Android small aggregate type table");
        assert_eq!(
            android
                .c_aggregate_register_type(&module.functions[0])
                .expect("Android packed register type")
                .get_bit_width(),
            32
        );
        assert_eq!(
            android
                .function_type(&module.functions[0])
                .expect("Android small aggregate function type")
                .print_to_string()
                .to_string(),
            "i32 (i16, i8)"
        );
    }

    #[test]
    fn lowers_linux_sysv_large_aggregate_returns_through_result_pointer() {
        let module = Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: false,
                    bits: 64,
                },
                Type::Struct {
                    fields: vec![TypeId::new(1), TypeId::new(1), TypeId::new(1)],
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "count".to_owned(),
                linkage: Linkage::Import,
                parameters: Vec::new(),
                result: TypeId::new(2),
                blocks: Vec::new(),
                span: None,
            }],
        };
        let context = Context::create();
        let linux = lower_types(&context, &module, &TypeLoweringConfig::x86_64_linux_gnu())
            .expect("valid Linux aggregate type table");
        assert!(linux.uses_c_aggregate_return(&module.functions[0]));
        assert_eq!(
            linux
                .function_type(&module.functions[0])
                .expect("aggregate function type")
                .print_to_string()
                .to_string(),
            "void (ptr)"
        );
    }

    #[test]
    fn completes_recursive_nominals_and_computes_aligned_enum_storage() {
        let module = Module {
            types: vec![
                Type::NominalStruct {
                    identity: 0x1234,
                    fields: vec![TypeId::new(1)],
                },
                Type::Pointer {
                    pointee: TypeId::new(0),
                    address_space: AddressSpace::Heap,
                },
                Type::NominalEnum {
                    identity: 0x5678,
                    variants: vec![Vec::new(), vec![TypeId::new(0)]],
                },
            ],
            functions: Vec::new(),
        };
        let context = Context::create();
        let table = lower_types(&context, &module, &TypeLoweringConfig::default())
            .expect("recursive pointer layout is valid");
        let record = table
            .get(TypeId::new(0))
            .and_then(|ty| ty.as_basic())
            .expect("record")
            .into_struct_type();
        let tagged = table
            .get(TypeId::new(2))
            .and_then(|ty| ty.as_basic())
            .expect("enum")
            .into_struct_type();

        assert!(!record.is_opaque());
        assert!(!tagged.is_opaque());
        assert_eq!(
            record.get_field_types()[0].print_to_string().to_string(),
            "ptr"
        );
        assert_eq!(
            table.enum_layout(TypeId::new(2)),
            Some(EnumLayout {
                tag_field: 0,
                payload_field: 2,
                payload_offset: 8,
                payload_size: 8,
                payload_alignment: 8,
            })
        );
        assert_eq!(table.target_data().get_store_size(&tagged), 16);
        assert_eq!(table.target_data().get_abi_alignment(&tagged), 8);
    }

    #[test]
    fn rejects_cpu_incompatible_and_unsized_type_shapes() {
        let context = Context::create();
        let gpu_pointer = Module {
            types: vec![
                Type::Integer {
                    signed: false,
                    bits: 8,
                },
                Type::Pointer {
                    pointee: TypeId::new(0),
                    address_space: AddressSpace::Workgroup,
                },
            ],
            functions: Vec::new(),
        };
        assert!(matches!(
            lower_types(&context, &gpu_pointer, &TypeLoweringConfig::default()),
            Err(TypeLowerError::UnsupportedAddressSpace {
                address_space: AddressSpace::Workgroup,
                ..
            })
        ));

        let recursive_value = Module {
            types: vec![Type::Struct {
                fields: vec![TypeId::new(0)],
            }],
            functions: Vec::new(),
        };
        match lower_types(&context, &recursive_value, &TypeLoweringConfig::default()) {
            Err(error) => assert_eq!(error, TypeLowerError::RecursiveValueType(TypeId::new(0))),
            Ok(_) => panic!("by-value recursion must fail"),
        }

        let unsupported_float = Module {
            types: vec![Type::Float { bits: 80 }],
            functions: Vec::new(),
        };
        assert!(matches!(
            lower_types(&context, &unsupported_float, &TypeLoweringConfig::default()),
            Err(TypeLowerError::UnsupportedFloat { bits: 80, .. })
        ));

        let aggregate_function_pointer = Module {
            types: vec![
                Type::Unit,
                Type::Integer {
                    signed: false,
                    bits: 32,
                },
                Type::Struct {
                    fields: vec![TypeId::new(1), TypeId::new(1)],
                },
                Type::Function {
                    parameters: vec![TypeId::new(1)],
                    result: TypeId::new(2),
                },
            ],
            functions: Vec::new(),
        };
        assert!(matches!(
            lower_types(
                &context,
                &aggregate_function_pointer,
                &TypeLoweringConfig::default()
            ),
            Err(TypeLowerError::UnsupportedFunctionPointerResult { .. })
        ));
    }
}
