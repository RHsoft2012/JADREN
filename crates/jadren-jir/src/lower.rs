use std::collections::{BTreeMap, BTreeSet};

use jadren_lexer::Operator;
use jadren_mir::{
    BasicBlockId as MirBlockId, CarrierPart, MirFunction, MirModule, MirOperand, MirOperandKind,
    MirPattern, MirPropagationKind, MirStatement, Place, Projection, Terminator as MirTerminator,
};
use jadren_resolve::SymbolId;
use jadren_source::Span;
use jadren_types::{
    Capability, CarrierTag, FloatWidth, IntegerWidth, NominalLayout, NominalLayoutKind,
    NominalTypeId, Signedness, Substitution, TypeId as SemanticTypeId, TypeKind, TypeStore,
};

use crate::{
    AddressSpace, BinaryOp, Block, BlockId, CastOp, ComparePredicate, Constant, Function,
    FunctionId, Instruction, InstructionKind, Linkage, Module, Parameter, Terminator, Type, TypeId,
    TypedValue, UnaryOp, ValueId,
};

/// Target choices needed while converting target-dependent semantic types.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LowerOptions {
    /// Width used for `IntSize` and `UIntSize` until target layout is a first-class JIR input.
    pub pointer_bits: u16,
}

impl Default for LowerOptions {
    fn default() -> Self {
        Self { pointer_bits: 64 }
    }
}

/// One explicit reason why valid MIR cannot yet be represented by JIR 0.1.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LowerError {
    pub span: Option<Span>,
    pub message: String,
}

/// Lowers the scalar/core subset of verified place-based MIR into deterministic JIR SSA.
///
/// Mutable MIR locals are represented by entry-block stack slots. This deliberately avoids
/// inventing phi values before the later mem2reg optimization while all instruction results,
/// parameters, and control-flow conditions remain SSA values.
pub fn lower_from_mir(
    mir: &MirModule,
    types: &TypeStore,
    options: LowerOptions,
) -> Result<Module, Vec<LowerError>> {
    if !matches!(options.pointer_bits, 32 | 64) {
        return Err(vec![LowerError {
            span: None,
            message: format!(
                "unsupported target pointer width {}; expected 32 or 64",
                options.pointer_bits
            ),
        }]);
    }

    let local_function_ids: BTreeMap<_, _> = mir
        .functions
        .iter()
        .enumerate()
        .map(|(index, function)| (function.symbol, FunctionId::new(index)))
        .collect();
    let call_targets = collect_call_targets(mir, types, &local_function_ids);
    let mut type_table = TypeTable::new(types, options.pointer_bits, &mir.nominal_layouts);
    let mut functions = Vec::with_capacity(mir.functions.len() + call_targets.external.len());
    let mut errors = Vec::new();
    for (index, function) in mir.functions.iter().enumerate() {
        match FunctionLowerer::new(
            function,
            FunctionId::new(index),
            &call_targets,
            &mut type_table,
        )
        .lower()
        {
            Ok(function) => functions.push(function),
            Err(mut function_errors) => errors.append(&mut function_errors),
        }
    }
    let mut external_targets: Vec<_> = call_targets.external.iter().collect();
    external_targets.sort_by_key(|(_, external)| external.id.index());
    for (target, external) in external_targets {
        match lower_import(target, external, &mut type_table) {
            Ok(function) => functions.push(function),
            Err(error) => errors.push(error),
        }
    }
    if errors.is_empty() {
        Ok(Module {
            types: type_table.types,
            functions,
        })
    } else {
        Err(errors)
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ExternalTarget {
    symbol: Option<SymbolId>,
    name: String,
    parameters: Vec<SemanticTypeId>,
    result: SemanticTypeId,
}

struct ExternalFunction {
    id: FunctionId,
    span: Span,
}

struct CallTargets {
    local: BTreeMap<SymbolId, FunctionId>,
    external: BTreeMap<ExternalTarget, ExternalFunction>,
}

fn collect_call_targets(
    mir: &MirModule,
    types: &TypeStore,
    local: &BTreeMap<SymbolId, FunctionId>,
) -> CallTargets {
    let mut targets = CallTargets {
        local: local.clone(),
        external: BTreeMap::new(),
    };
    for function in &mir.functions {
        for block in &function.blocks {
            for statement in &block.statements {
                match statement {
                    MirStatement::Assign {
                        destination_indices,
                        value,
                        ..
                    } => {
                        for index in destination_indices {
                            collect_operand_calls(
                                index,
                                mir,
                                types,
                                mir.functions.len(),
                                &mut targets,
                            );
                        }
                        if let Some(value) = value {
                            collect_operand_calls(
                                value,
                                mir,
                                types,
                                mir.functions.len(),
                                &mut targets,
                            );
                        }
                    }
                    MirStatement::Evaluate { value, .. } => {
                        if let Some(value) = value {
                            collect_operand_calls(
                                value,
                                mir,
                                types,
                                mir.functions.len(),
                                &mut targets,
                            );
                        }
                    }
                    MirStatement::StorageLive { .. }
                    | MirStatement::StorageDead { .. }
                    | MirStatement::RegionEnter { .. }
                    | MirStatement::RegionExit { .. }
                    | MirStatement::Borrow { .. }
                    | MirStatement::Drop { .. } => {}
                }
            }
            match &block.terminator {
                MirTerminator::Switch { value, .. } | MirTerminator::Return { value, .. } => {
                    if let Some(value) = value {
                        collect_operand_calls(value, mir, types, mir.functions.len(), &mut targets);
                    }
                }
                MirTerminator::Match { value, .. } | MirTerminator::Propagate { value, .. } => {
                    collect_operand_calls(value, mir, types, mir.functions.len(), &mut targets);
                }
                MirTerminator::Goto { .. } | MirTerminator::Unreachable { .. } => {}
            }
        }
    }
    targets
}

fn collect_operand_calls(
    operand: &MirOperand,
    mir: &MirModule,
    types: &TypeStore,
    local_count: usize,
    targets: &mut CallTargets,
) {
    match &operand.kind {
        MirOperandKind::Call { callee, arguments } => {
            if let MirOperandKind::Function { name, symbol } = &callee.kind
                && symbol.is_none_or(|symbol| !targets.local.contains_key(&symbol))
                && !is_lowered_vector_intrinsic(name)
                && constructor_variant_index(mir, types, operand.ty, name).is_none()
            {
                let target = ExternalTarget {
                    symbol: *symbol,
                    name: name.clone(),
                    parameters: arguments.iter().map(|argument| argument.ty).collect(),
                    result: operand.ty,
                };
                let next = FunctionId::new(local_count + targets.external.len());
                targets.external.entry(target).or_insert(ExternalFunction {
                    id: next,
                    span: callee.span,
                });
            }
            collect_operand_calls(callee, mir, types, local_count, targets);
            for argument in arguments {
                collect_operand_calls(argument, mir, types, local_count, targets);
            }
        }
        MirOperandKind::Function { name, symbol }
            if symbol.is_none_or(|symbol| !targets.local.contains_key(&symbol))
                && !is_lowered_vector_intrinsic(name)
                && let Some(TypeKind::Function { parameters, result }) = types.kind(operand.ty) =>
        {
            let target = ExternalTarget {
                symbol: *symbol,
                name: name.clone(),
                parameters: parameters.to_vec(),
                result: *result,
            };
            let next = FunctionId::new(local_count + targets.external.len());
            targets.external.entry(target).or_insert(ExternalFunction {
                id: next,
                span: operand.span,
            });
        }
        MirOperandKind::Unary { operand, .. } => {
            collect_operand_calls(operand, mir, types, local_count, targets);
        }
        MirOperandKind::Cast { operand } => {
            collect_operand_calls(operand, mir, types, local_count, targets);
        }
        MirOperandKind::Binary { left, right, .. } => {
            collect_operand_calls(left, mir, types, local_count, targets);
            collect_operand_calls(right, mir, types, local_count, targets);
        }
        MirOperandKind::RegionAllocate { arguments, .. } | MirOperandKind::Array(arguments) => {
            for argument in arguments {
                collect_operand_calls(argument, mir, types, local_count, targets);
            }
        }
        MirOperandKind::Index { base, index } => {
            collect_operand_calls(base, mir, types, local_count, targets);
            collect_operand_calls(index, mir, types, local_count, targets);
        }
        MirOperandKind::Length { base } => {
            collect_operand_calls(base, mir, types, local_count, targets);
        }
        MirOperandKind::Field { base, .. } => {
            collect_operand_calls(base, mir, types, local_count, targets);
        }
        MirOperandKind::Struct { fields, .. } => {
            for (_, value) in fields {
                collect_operand_calls(value, mir, types, local_count, targets);
            }
        }
        MirOperandKind::Unit
        | MirOperandKind::Place(_)
        | MirOperandKind::Literal(_)
        | MirOperandKind::Function { .. }
        | MirOperandKind::PatternExtract { .. }
        | MirOperandKind::CarrierExtract { .. }
        | MirOperandKind::PropagateResidual { .. }
        | MirOperandKind::HighLevel(_) => {}
    }
}

fn is_lowered_vector_intrinsic(name: &str) -> bool {
    vector_intrinsic_lanes(name).is_some()
}

fn vector_intrinsic_lanes(name: &str) -> Option<u16> {
    match name {
        "vector_load2" | "vector_splat2" | "vector_store2" => Some(2),
        "vector_load3" | "vector_splat3" | "vector_store3" => Some(3),
        "vector_load4" | "vector_splat4" | "vector_store4" => Some(4),
        "vector_load8" | "vector_splat8" | "vector_store8" => Some(8),
        _ => None,
    }
}

fn constructor_variant_index(
    mir: &MirModule,
    types: &TypeStore,
    ty: SemanticTypeId,
    path: &str,
) -> Option<u32> {
    let name = path.rsplit('.').next().unwrap_or(path);
    let names: Vec<&str> = match types.kind(ty)? {
        TypeKind::Option(_) => vec!["None", "Some"],
        TypeKind::Result { .. } => vec!["Error", "Ok"],
        TypeKind::Nominal { constructor, .. } => {
            let layout = mir
                .nominal_layouts
                .iter()
                .find(|layout| layout.constructor == *constructor)?;
            let NominalLayoutKind::Enum { variants } = &layout.kind else {
                return None;
            };
            variants
                .iter()
                .map(|variant| variant.name.as_str())
                .collect()
        }
        _ => return None,
    };
    names
        .iter()
        .position(|candidate| *candidate == name)
        .map(|index| index as u32)
}

fn lower_import(
    target: &ExternalTarget,
    external: &ExternalFunction,
    types: &mut TypeTable,
) -> Result<Function, LowerError> {
    let mut lowered_parameters = Vec::with_capacity(target.parameters.len());
    for (index, parameter) in target.parameters.iter().enumerate() {
        lowered_parameters.push(Parameter {
            value: ValueId::new(index),
            ty: types.lower(*parameter, Some(external.span))?,
            name: None,
        });
    }
    Ok(Function {
        id: external.id,
        name: target.name.clone(),
        linkage: Linkage::Import,
        parameters: lowered_parameters,
        result: types.lower(target.result, Some(external.span))?,
        blocks: Vec::new(),
        span: Some(external.span),
    })
}

struct TypeTable {
    semantic: TypeStore,
    pointer_bits: u16,
    types: Vec<Type>,
    lowered: BTreeMap<SemanticTypeId, TypeId>,
    layouts: BTreeMap<NominalTypeId, NominalLayout>,
    lowering: BTreeSet<SemanticTypeId>,
}

impl TypeTable {
    fn new(semantic: &TypeStore, pointer_bits: u16, layouts: &[NominalLayout]) -> Self {
        Self {
            semantic: semantic.clone(),
            pointer_bits,
            types: Vec::new(),
            lowered: BTreeMap::new(),
            layouts: layouts
                .iter()
                .cloned()
                .map(|layout| (layout.constructor, layout))
                .collect(),
            lowering: BTreeSet::new(),
        }
    }

    fn lower(&mut self, source: SemanticTypeId, span: Option<Span>) -> Result<TypeId, LowerError> {
        if let Some(lowered) = self.lowered.get(&source) {
            return Ok(*lowered);
        }
        let source_kind = self
            .semantic
            .kind(source)
            .cloned()
            .ok_or_else(|| LowerError {
                span,
                message: format!("semantic type #{} does not exist", source.index()),
            })?;
        if let TypeKind::Nominal {
            constructor,
            arguments,
        } = &source_kind
        {
            if *constructor == NominalTypeId::from_path("core.Region") {
                let lowered = self.intern(Type::RegionHandle);
                self.lowered.insert(source, lowered);
                return Ok(lowered);
            }
            return self.lower_nominal_forward(source, *constructor, arguments, span);
        }
        if !self.lowering.insert(source) {
            return Err(LowerError {
                span,
                message: "recursive non-nominal JIR type is invalid".to_owned(),
            });
        }
        let result = (|| {
            let kind = source_kind;
            let lowered = match &kind {
                TypeKind::Bool => self.intern(Type::Bool),
                TypeKind::Char => self.intern(Type::Integer {
                    signed: false,
                    bits: 32,
                }),
                TypeKind::Unit | TypeKind::Never => self.intern(Type::Unit),
                TypeKind::Integer { signedness, width } => self.intern(Type::Integer {
                    signed: *signedness == Signedness::Signed,
                    bits: integer_bits(*width, self.pointer_bits),
                }),
                TypeKind::Float(width) => self.intern(Type::Float {
                    bits: float_bits(*width),
                }),
                TypeKind::Vector { element, lanes } => {
                    let element = self.lower(*element, span)?;
                    self.intern(Type::Vector {
                        element,
                        lanes: *lanes,
                    })
                }
                TypeKind::Array { element, length } => {
                    let element = self.lower(*element, span)?;
                    self.intern(Type::Array {
                        element,
                        length: *length,
                    })
                }
                TypeKind::Pointer(inner) => {
                    let pointee = self.lower(*inner, span)?;
                    self.intern(Type::Pointer {
                        pointee,
                        address_space: AddressSpace::Generic,
                    })
                }
                TypeKind::Capability { capability, inner } => match capability {
                    Capability::Owned => self.lower(*inner, span)?,
                    Capability::Read | Capability::Write => {
                        self.lower_borrow_capability(*inner, false, span)?
                    }
                },
                TypeKind::Option(payload) => {
                    let payload = self.lower(*payload, span)?;
                    self.intern(Type::Enum {
                        variants: vec![Vec::new(), vec![payload]],
                    })
                }
                TypeKind::Result { ok, error } => {
                    let ok = self.lower(*ok, span)?;
                    let error = self.lower(*error, span)?;
                    self.intern(Type::Enum {
                        variants: vec![vec![error], vec![ok]],
                    })
                }
                TypeKind::Buffer(element) => {
                    self.lower_buffer(*element, AddressSpace::Heap, true, span)?
                }
                TypeKind::Slice(element) => {
                    self.lower_buffer(*element, AddressSpace::Generic, false, span)?
                }
                TypeKind::String => {
                    let byte = self.intern(Type::Integer {
                        signed: false,
                        bits: 8,
                    });
                    let data = self.intern(Type::Pointer {
                        pointee: byte,
                        address_space: AddressSpace::Global,
                    });
                    let size = self.intern(Type::Integer {
                        signed: false,
                        bits: self.pointer_bits,
                    });
                    self.intern(Type::Struct {
                        fields: vec![data, size],
                    })
                }
                TypeKind::Nominal { .. } => unreachable!("nominal types are handled above"),
                TypeKind::Function { parameters, result } => {
                    let parameters = parameters
                        .iter()
                        .map(|parameter| self.lower(*parameter, span))
                        .collect::<Result<Vec<_>, _>>()?;
                    let result = self.lower(*result, span)?;
                    self.intern(Type::Function { parameters, result })
                }
                TypeKind::Error
                | TypeKind::GenericParameter(_)
                | TypeKind::InferenceVariable(_) => {
                    return Err(LowerError {
                        span,
                        message: format!(
                            "JIR lowering does not yet support semantic type {kind:?}"
                        ),
                    });
                }
            };
            Ok(lowered)
        })();
        self.lowering.remove(&source);
        let lowered = result?;
        self.lowered.insert(source, lowered);
        Ok(lowered)
    }

    fn lower_buffer(
        &mut self,
        element: SemanticTypeId,
        address_space: AddressSpace,
        owning: bool,
        span: Option<Span>,
    ) -> Result<TypeId, LowerError> {
        let element = self.lower(element, span)?;
        let data = self.intern(Type::Pointer {
            pointee: element,
            address_space,
        });
        let size = self.intern(Type::Integer {
            signed: false,
            bits: self.pointer_bits,
        });
        let fields = if owning {
            vec![data, size, size]
        } else {
            vec![data, size]
        };
        Ok(self.intern(Type::Struct { fields }))
    }

    fn lower_borrow_capability(
        &mut self,
        inner: SemanticTypeId,
        _region_owned: bool,
        span: Option<Span>,
    ) -> Result<TypeId, LowerError> {
        let pointee = match self.semantic.kind(inner).cloned() {
            Some(TypeKind::Buffer(element) | TypeKind::Slice(element)) => {
                return self.lower_buffer(element, AddressSpace::Generic, false, span);
            }
            _ => self.lower(inner, span)?,
        };
        Ok(self.pointer(pointee, AddressSpace::Generic))
    }

    fn lower_nominal_forward(
        &mut self,
        source: SemanticTypeId,
        constructor: NominalTypeId,
        arguments: &[SemanticTypeId],
        span: Option<Span>,
    ) -> Result<TypeId, LowerError> {
        let layout = self
            .layouts
            .get(&constructor)
            .cloned()
            .ok_or_else(|| LowerError {
                span,
                message: format!("missing nominal layout for {constructor:?}"),
            })?;
        let identity = constructor.fingerprint().as_u64();
        let placeholder = match layout.kind {
            NominalLayoutKind::Record { .. } => Type::NominalStruct {
                identity,
                fields: Vec::new(),
            },
            NominalLayoutKind::Enum { .. } => Type::NominalEnum {
                identity,
                variants: Vec::new(),
            },
        };
        let id = TypeId::new(self.types.len());
        self.types.push(placeholder);
        self.lowered.insert(source, id);
        let result = self.build_nominal(constructor, arguments, span);
        match result {
            Ok(ty) => {
                self.types[id.index()] = ty;
                Ok(id)
            }
            Err(error) => {
                self.lowered.remove(&source);
                Err(error)
            }
        }
    }

    fn build_nominal(
        &mut self,
        constructor: NominalTypeId,
        arguments: &[SemanticTypeId],
        span: Option<Span>,
    ) -> Result<Type, LowerError> {
        let layout = self
            .layouts
            .get(&constructor)
            .cloned()
            .ok_or_else(|| LowerError {
                span,
                message: format!("missing nominal layout for {constructor:?}"),
            })?;
        if layout.generic_parameters.len() != arguments.len() {
            return Err(LowerError {
                span,
                message: "nominal layout generic argument count differs from type".to_owned(),
            });
        }
        let mut substitution = Substitution::new();
        for (parameter, argument) in layout.generic_parameters.iter().zip(arguments) {
            substitution.insert(*parameter, *argument);
        }
        let mut lower_field = |field: SemanticTypeId| -> Result<TypeId, LowerError> {
            let concrete = substitution
                .apply(&mut self.semantic, field)
                .map_err(|error| LowerError {
                    span,
                    message: format!("cannot instantiate nominal field type: {error:?}"),
                })?;
            self.lower(concrete, span)
        };
        match layout.kind {
            NominalLayoutKind::Record { fields } => {
                let fields = fields
                    .into_iter()
                    .map(|field| lower_field(field.ty))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Type::NominalStruct {
                    identity: constructor.fingerprint().as_u64(),
                    fields,
                })
            }
            NominalLayoutKind::Enum { variants } => {
                let variants = variants
                    .into_iter()
                    .map(|variant| {
                        variant
                            .fields
                            .into_iter()
                            .map(&mut lower_field)
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Type::NominalEnum {
                    identity: constructor.fingerprint().as_u64(),
                    variants,
                })
            }
        }
    }

    fn record_fields(
        &mut self,
        ty: SemanticTypeId,
        span: Option<Span>,
    ) -> Result<Vec<(String, SemanticTypeId)>, LowerError> {
        let Some(TypeKind::Nominal {
            constructor,
            arguments,
        }) = self.semantic.kind(ty).cloned()
        else {
            return Err(LowerError {
                span,
                message: "field operation requires a nominal record type".to_owned(),
            });
        };
        let layout = self
            .layouts
            .get(&constructor)
            .cloned()
            .ok_or_else(|| LowerError {
                span,
                message: format!("missing nominal layout for {constructor:?}"),
            })?;
        let NominalLayoutKind::Record { fields } = layout.kind else {
            return Err(LowerError {
                span,
                message: "field operation cannot use an enum layout".to_owned(),
            });
        };
        let mut substitution = Substitution::new();
        for (parameter, argument) in layout.generic_parameters.iter().zip(arguments.iter()) {
            substitution.insert(*parameter, *argument);
        }
        fields
            .into_iter()
            .map(|field| {
                substitution
                    .apply(&mut self.semantic, field.ty)
                    .map(|ty| (field.name, ty))
                    .map_err(|error| LowerError {
                        span,
                        message: format!("cannot instantiate record field type: {error:?}"),
                    })
            })
            .collect()
    }

    fn field_index_and_type(
        &mut self,
        ty: SemanticTypeId,
        field: &str,
        span: Option<Span>,
    ) -> Result<(u32, SemanticTypeId), LowerError> {
        self.record_fields(ty, span)?
            .into_iter()
            .enumerate()
            .find(|(_, (name, _))| name == field)
            .map(|(index, (_, ty))| (index as u32, ty))
            .ok_or_else(|| LowerError {
                span,
                message: format!("record layout has no field {field:?}"),
            })
    }

    fn enum_variants(
        &mut self,
        ty: SemanticTypeId,
        span: Option<Span>,
    ) -> Result<Vec<(String, Vec<SemanticTypeId>)>, LowerError> {
        match self.semantic.kind(ty).cloned() {
            Some(TypeKind::Option(payload)) => Ok(vec![
                ("None".to_owned(), Vec::new()),
                ("Some".to_owned(), vec![payload]),
            ]),
            Some(TypeKind::Result { ok, error }) => Ok(vec![
                ("Error".to_owned(), vec![error]),
                ("Ok".to_owned(), vec![ok]),
            ]),
            Some(TypeKind::Nominal {
                constructor,
                arguments,
            }) => {
                let layout = self
                    .layouts
                    .get(&constructor)
                    .cloned()
                    .ok_or_else(|| LowerError {
                        span,
                        message: format!("missing nominal layout for {constructor:?}"),
                    })?;
                let NominalLayoutKind::Enum { variants } = layout.kind else {
                    return Err(LowerError {
                        span,
                        message: "enum operation requires an enum nominal layout".to_owned(),
                    });
                };
                let mut substitution = Substitution::new();
                for (parameter, argument) in layout.generic_parameters.iter().zip(arguments.iter())
                {
                    substitution.insert(*parameter, *argument);
                }
                variants
                    .into_iter()
                    .map(|variant| {
                        let fields = variant
                            .fields
                            .into_iter()
                            .map(|field| {
                                substitution
                                    .apply(&mut self.semantic, field)
                                    .map_err(|error| LowerError {
                                        span,
                                        message: format!(
                                            "cannot instantiate enum payload type: {error:?}"
                                        ),
                                    })
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        Ok((variant.name, fields))
                    })
                    .collect()
            }
            _ => Err(LowerError {
                span,
                message: "enum operation requires Option, Result, or an enum nominal".to_owned(),
            }),
        }
    }

    fn variant_index_and_fields(
        &mut self,
        ty: SemanticTypeId,
        path: &str,
        span: Option<Span>,
    ) -> Result<(u32, Vec<SemanticTypeId>), LowerError> {
        let name = path.rsplit('.').next().unwrap_or(path);
        self.enum_variants(ty, span)?
            .into_iter()
            .enumerate()
            .find(|(_, (candidate, _))| candidate == name)
            .map(|(index, (_, fields))| (index as u32, fields))
            .ok_or_else(|| LowerError {
                span,
                message: format!("enum layout has no variant {path:?}"),
            })
    }

    fn intern(&mut self, ty: Type) -> TypeId {
        if let Some(index) = self.types.iter().position(|candidate| candidate == &ty) {
            return TypeId::new(index);
        }
        let id = TypeId::new(self.types.len());
        self.types.push(ty);
        id
    }

    fn stack_pointer(&mut self, pointee: TypeId) -> TypeId {
        self.pointer(pointee, AddressSpace::Stack)
    }

    fn pointer(&mut self, pointee: TypeId, address_space: AddressSpace) -> TypeId {
        self.intern(Type::Pointer {
            pointee,
            address_space,
        })
    }

    fn is_unit(&self, ty: TypeId) -> bool {
        matches!(self.types.get(ty.index()), Some(Type::Unit))
    }
}

struct FunctionLowerer<'a, 'types> {
    source: &'a MirFunction,
    id: FunctionId,
    call_targets: &'a CallTargets,
    type_table: &'types mut TypeTable,
    next_value: usize,
    local_addresses: Vec<ValueId>,
    local_types: Vec<TypeId>,
    prelude: Vec<Instruction>,
    synthetic_blocks: Vec<Block>,
    errors: Vec<LowerError>,
}

#[derive(Clone, Copy)]
struct PatternTargets {
    matched: BlockId,
    otherwise: BlockId,
}

impl<'a, 'types> FunctionLowerer<'a, 'types> {
    fn new(
        source: &'a MirFunction,
        id: FunctionId,
        call_targets: &'a CallTargets,
        type_table: &'types mut TypeTable,
    ) -> Self {
        Self {
            source,
            id,
            call_targets,
            type_table,
            next_value: source
                .locals
                .iter()
                .filter(|local| local.is_parameter)
                .count(),
            local_addresses: Vec::new(),
            local_types: Vec::new(),
            prelude: Vec::new(),
            synthetic_blocks: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn lower(mut self) -> Result<Function, Vec<LowerError>> {
        let (parameter_types, result_source) =
            match self.type_table.semantic.kind(self.source.signature) {
                Some(TypeKind::Function { parameters, result }) => (parameters.clone(), *result),
                kind => {
                    return Err(vec![self.error(
                        Some(self.source.span),
                        format!("MIR function signature is not a function type: {kind:?}"),
                    )]);
                }
            };
        let mut parameters = Vec::with_capacity(parameter_types.len());
        for (index, source_ty) in parameter_types.iter().enumerate() {
            match self.type_table.lower(*source_ty, Some(self.source.span)) {
                Ok(ty) => parameters.push(Parameter {
                    value: ValueId::new(index),
                    ty,
                    name: self
                        .source
                        .locals
                        .iter()
                        .filter(|local| local.is_parameter)
                        .nth(index)
                        .map(|local| local.name.clone()),
                }),
                Err(error) => self.errors.push(error),
            }
        }
        let result = match self.type_table.lower(result_source, Some(self.source.span)) {
            Ok(result) => result,
            Err(error) => {
                self.errors.push(error);
                self.type_table.intern(Type::Unit)
            }
        };
        self.prepare_locals(&parameters);
        self.lower_disjoint_contract();

        let mut blocks = Vec::with_capacity(self.source.blocks.len());
        for block in &self.source.blocks {
            let mut instructions = if block.id.index() == 0 {
                std::mem::take(&mut self.prelude)
            } else {
                Vec::new()
            };
            for statement in &block.statements {
                self.lower_statement(statement, &mut instructions);
            }
            let terminator = self.lower_terminator(&block.terminator, &mut instructions);
            blocks.push(Block {
                id: BlockId::new(block.id.index()),
                parameters: Vec::new(),
                instructions,
                terminator,
                span: Some(self.source.span),
            });
        }
        blocks.append(&mut self.synthetic_blocks);
        if self.errors.is_empty() {
            let (name, linkage) = self.source.export.as_ref().map_or_else(
                || (self.source.name.clone(), Linkage::Internal),
                |export| (export.name.clone(), Linkage::Export),
            );
            Ok(Function {
                id: self.id,
                name,
                linkage,
                parameters,
                result,
                blocks,
                span: Some(self.source.span),
            })
        } else {
            Err(self.errors)
        }
    }

    fn prepare_locals(&mut self, parameters: &[Parameter]) {
        let mut parameter_index = 0;
        for local in &self.source.locals {
            let lowered_local = match self.type_table.semantic.kind(local.ty).cloned() {
                Some(TypeKind::Buffer(element)) if local.owned_region.is_some() => self
                    .type_table
                    .lower_buffer(element, AddressSpace::Region, true, Some(local.span)),
                Some(TypeKind::Capability {
                    capability: Capability::Read | Capability::Write,
                    inner,
                }) if local.owned_region.is_some() => {
                    self.type_table
                        .lower_borrow_capability(inner, true, Some(local.span))
                }
                _ => self.type_table.lower(local.ty, Some(local.span)),
            };
            let ty = match lowered_local {
                Ok(ty) => ty,
                Err(error) => {
                    self.errors.push(error);
                    self.local_types.push(self.type_table.intern(Type::Unit));
                    self.local_addresses.push(ValueId::new(0));
                    continue;
                }
            };
            let pointer_ty = self.type_table.stack_pointer(ty);
            let address = self.new_value();
            self.prelude.push(Instruction {
                result: Some(TypedValue {
                    value: address,
                    ty: pointer_ty,
                }),
                kind: InstructionKind::StackAlloc { ty, count: None },
                span: Some(local.span),
            });
            self.local_types.push(ty);
            self.local_addresses.push(address);
            if local.is_parameter {
                if let Some(parameter) = parameters.get(parameter_index) {
                    self.prelude.push(Instruction {
                        result: None,
                        kind: InstructionKind::Store {
                            pointer: address,
                            value: parameter.value,
                            alignment: 1,
                            volatile: false,
                        },
                        span: Some(local.span),
                    });
                }
                parameter_index += 1;
            }
        }
    }

    fn lower_disjoint_contract(&mut self) {
        if !self.source.disjoint {
            return;
        }
        let values: Vec<_> = self
            .source
            .locals
            .iter()
            .filter(|local| local.is_parameter)
            .enumerate()
            .filter(|(_, local)| self.is_disjoint_borrow(local.ty))
            .map(|(index, _)| ValueId::new(index))
            .collect();
        for (index, left) in values.iter().enumerate() {
            for right in values.iter().skip(index + 1) {
                self.prelude.push(Instruction {
                    result: None,
                    kind: InstructionKind::AssumeNoAlias {
                        left: *left,
                        right: *right,
                    },
                    span: Some(self.source.span),
                });
            }
        }
    }

    fn is_disjoint_borrow(&self, ty: SemanticTypeId) -> bool {
        let Some(TypeKind::Capability { capability, inner }) = self.type_table.semantic.kind(ty)
        else {
            return false;
        };
        if !matches!(capability, Capability::Read | Capability::Write) {
            return false;
        }
        matches!(
            self.type_table.semantic.kind(*inner),
            Some(TypeKind::Slice(_) | TypeKind::Buffer(_))
        )
    }

    fn lower_statement(&mut self, statement: &MirStatement, instructions: &mut Vec<Instruction>) {
        match statement {
            MirStatement::StorageLive { .. } | MirStatement::StorageDead { .. } => {}
            MirStatement::Assign {
                destination,
                destination_indices,
                value,
                span,
                ..
            } => {
                let Some(value) = value else {
                    self.errors
                        .push(self.error(Some(*span), "MIR assignment has no value"));
                    return;
                };
                let Some(value) = self.lower_operand(value, instructions) else {
                    return;
                };
                let Some(value) = value else {
                    return;
                };
                let Some(pointer) = self.lower_place_address(
                    destination,
                    destination_indices,
                    instructions,
                    Some(*span),
                ) else {
                    return;
                };
                instructions.push(Instruction {
                    result: None,
                    kind: InstructionKind::Store {
                        pointer,
                        value,
                        alignment: 1,
                        volatile: false,
                    },
                    span: Some(*span),
                });
            }
            MirStatement::Evaluate { value, span, .. } => {
                if let Some(value) = value {
                    self.lower_operand(value, instructions);
                } else {
                    self.errors
                        .push(self.error(Some(*span), "MIR evaluation has no value"));
                }
            }
            MirStatement::Borrow {
                destination,
                source,
                span,
                ..
            } => {
                let Some(destination_local) = self.source.locals.get(destination.index()) else {
                    self.errors
                        .push(self.error(Some(*span), "borrow destination local does not exist"));
                    return;
                };
                let Some(TypeKind::Capability { inner, .. }) =
                    self.type_table.semantic.kind(destination_local.ty).cloned()
                else {
                    self.errors.push(
                        self.error(Some(*span), "borrow destination is not a capability type"),
                    );
                    return;
                };
                let Some(borrowed) =
                    self.lower_borrow_from_place(source, inner, instructions, *span)
                else {
                    return;
                };
                let Some(pointer) = self.local_addresses.get(destination.index()).copied() else {
                    self.errors
                        .push(self.error(Some(*span), "borrow destination storage does not exist"));
                    return;
                };
                instructions.push(Instruction {
                    result: None,
                    kind: InstructionKind::Store {
                        pointer,
                        value: borrowed,
                        alignment: 1,
                        volatile: false,
                    },
                    span: Some(*span),
                });
            }
            MirStatement::RegionEnter { region, span } => {
                let region_ty = self.type_table.intern(Type::RegionHandle);
                let handle = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: handle,
                        ty: region_ty,
                    }),
                    kind: InstructionKind::RegionCreate,
                    span: Some(*span),
                });
                let Some(pointer) = self.local_addresses.get(region.index()).copied() else {
                    self.errors
                        .push(self.error(Some(*span), "region storage does not exist"));
                    return;
                };
                instructions.push(Instruction {
                    result: None,
                    kind: InstructionKind::Store {
                        pointer,
                        value: handle,
                        alignment: 1,
                        volatile: false,
                    },
                    span: Some(*span),
                });
            }
            MirStatement::RegionExit { region, span } => {
                let Some((region, _)) =
                    self.load_source_place(&Place::local(*region), &[], instructions, *span)
                else {
                    return;
                };
                instructions.push(Instruction {
                    result: None,
                    kind: InstructionKind::RegionDestroy { region },
                    span: Some(*span),
                });
            }
            MirStatement::Drop { place, span } => {
                let Some((value, _)) = self.load_source_place(place, &[], instructions, *span)
                else {
                    return;
                };
                instructions.push(Instruction {
                    result: None,
                    kind: InstructionKind::Drop { value },
                    span: Some(*span),
                });
            }
        }
    }

    fn lower_terminator(
        &mut self,
        terminator: &MirTerminator,
        instructions: &mut Vec<Instruction>,
    ) -> Terminator {
        match terminator {
            MirTerminator::Goto { target, .. } => Terminator::Jump {
                target: lower_block(*target),
                arguments: Vec::new(),
            },
            MirTerminator::Switch {
                value,
                targets,
                otherwise,
                span,
                ..
            } => {
                let condition = value
                    .as_ref()
                    .and_then(|value| self.lower_operand(value, instructions).flatten());
                match (condition, targets.as_slice()) {
                    (Some(condition), [then_target]) => Terminator::Branch {
                        condition,
                        then_target: lower_block(*then_target),
                        then_arguments: Vec::new(),
                        else_target: lower_block(*otherwise),
                        else_arguments: Vec::new(),
                    },
                    _ => {
                        self.errors.push(self.error(
                            Some(*span),
                            "JIR scalar lowering requires one-target Bool MIR switch",
                        ));
                        Terminator::Unreachable
                    }
                }
            }
            MirTerminator::Return { value, .. } => Terminator::Return {
                value: value
                    .as_ref()
                    .and_then(|value| self.lower_operand(value, instructions).flatten()),
            },
            MirTerminator::Unreachable { .. } => Terminator::Unreachable,
            MirTerminator::Match {
                value,
                pattern,
                matched,
                otherwise,
                span,
                ..
            } => {
                let discriminant = self.lower_required_operand(value, instructions);
                discriminant.map_or(Terminator::Unreachable, |discriminant| {
                    self.lower_pattern_branch(
                        discriminant,
                        value.ty,
                        pattern,
                        PatternTargets {
                            matched: lower_block(*matched),
                            otherwise: lower_block(*otherwise),
                        },
                        instructions,
                        *span,
                    )
                })
            }
            MirTerminator::Propagate {
                value,
                success,
                residual,
                span,
                ..
            } => {
                let carrier = self.lower_required_operand(value, instructions);
                let condition = carrier
                    .map(|carrier| self.emit_variant_condition(carrier, 1, instructions, *span));
                condition.map_or(Terminator::Unreachable, |condition| Terminator::Branch {
                    condition,
                    then_target: lower_block(*success),
                    then_arguments: Vec::new(),
                    else_target: lower_block(*residual),
                    else_arguments: Vec::new(),
                })
            }
        }
    }

    fn lower_pattern_condition(
        &mut self,
        value: ValueId,
        ty: SemanticTypeId,
        pattern: &MirPattern,
        instructions: &mut Vec<Instruction>,
        span: Span,
    ) -> Option<ValueId> {
        match pattern {
            MirPattern::Wildcard | MirPattern::Binding => {
                let bool_ty = self.type_table.intern(Type::Bool);
                let result = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: result,
                        ty: bool_ty,
                    }),
                    kind: InstructionKind::Constant(Constant::Bool(true)),
                    span: Some(span),
                });
                Some(result)
            }
            MirPattern::Literal(literal) => {
                let constant =
                    match lower_constant(&literal.text, self.type_table.semantic.kind(ty)) {
                        Ok(constant) => constant,
                        Err(message) => {
                            self.errors.push(self.error(Some(span), message));
                            return None;
                        }
                    };
                let literal_ty = match self.type_table.lower(ty, Some(span)) {
                    Ok(ty) => ty,
                    Err(error) => {
                        self.errors.push(error);
                        return None;
                    }
                };
                let expected = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: expected,
                        ty: literal_ty,
                    }),
                    kind: InstructionKind::Constant(constant),
                    span: Some(span),
                });
                let bool_ty = self.type_table.intern(Type::Bool);
                let condition = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: condition,
                        ty: bool_ty,
                    }),
                    kind: InstructionKind::Compare {
                        predicate: ComparePredicate::Equal,
                        left: value,
                        right: expected,
                    },
                    span: Some(span),
                });
                Some(condition)
            }
            MirPattern::Path { path, .. } => {
                let (variant, _) =
                    match self
                        .type_table
                        .variant_index_and_fields(ty, path, Some(span))
                    {
                        Ok(variant) => variant,
                        Err(error) => {
                            self.errors.push(error);
                            return None;
                        }
                    };
                Some(self.emit_variant_condition(value, variant, instructions, span))
            }
            MirPattern::Constructor {
                path, arguments, ..
            } => {
                if arguments
                    .iter()
                    .any(|argument| !matches!(argument, MirPattern::Wildcard | MirPattern::Binding))
                {
                    self.errors.push(self.error(
                        Some(span),
                        "nested payload pattern lowering requires synthetic JIR CFG blocks",
                    ));
                    return None;
                }
                let (variant, fields) =
                    match self
                        .type_table
                        .variant_index_and_fields(ty, path, Some(span))
                    {
                        Ok(variant) => variant,
                        Err(error) => {
                            self.errors.push(error);
                            return None;
                        }
                    };
                if arguments.len() != fields.len() {
                    self.errors.push(self.error(
                        Some(span),
                        "MIR constructor pattern payload count differs from enum layout",
                    ));
                    return None;
                }
                Some(self.emit_variant_condition(value, variant, instructions, span))
            }
        }
    }

    fn lower_pattern_branch(
        &mut self,
        value: ValueId,
        ty: SemanticTypeId,
        pattern: &MirPattern,
        targets: PatternTargets,
        instructions: &mut Vec<Instruction>,
        span: Span,
    ) -> Terminator {
        let MirPattern::Constructor {
            path, arguments, ..
        } = pattern
        else {
            return self
                .lower_pattern_condition(value, ty, pattern, instructions, span)
                .map_or(Terminator::Unreachable, |condition| Terminator::Branch {
                    condition,
                    then_target: targets.matched,
                    then_arguments: Vec::new(),
                    else_target: targets.otherwise,
                    else_arguments: Vec::new(),
                });
        };
        let (variant, fields) = match self
            .type_table
            .variant_index_and_fields(ty, path, Some(span))
        {
            Ok(variant) => variant,
            Err(error) => {
                self.errors.push(error);
                return Terminator::Unreachable;
            }
        };
        if arguments.len() != fields.len() {
            self.errors.push(self.error(
                Some(span),
                "MIR constructor pattern payload count differs from enum layout",
            ));
            return Terminator::Unreachable;
        }
        let significant: Vec<_> = arguments
            .iter()
            .enumerate()
            .filter(|(_, pattern)| !matches!(pattern, MirPattern::Wildcard | MirPattern::Binding))
            .collect();
        let condition = self.emit_variant_condition(value, variant, instructions, span);
        if significant.is_empty() {
            return Terminator::Branch {
                condition,
                then_target: targets.matched,
                then_arguments: Vec::new(),
                else_target: targets.otherwise,
                else_arguments: Vec::new(),
            };
        }

        let blocks: Vec<_> = significant
            .iter()
            .map(|_| self.reserve_synthetic_block(span))
            .collect();
        for (position, ((field_index, pattern), block)) in
            significant.into_iter().zip(blocks.iter()).enumerate()
        {
            let field_ty = fields[field_index];
            let jir_ty = match self.type_table.lower(field_ty, Some(span)) {
                Ok(ty) => ty,
                Err(error) => {
                    self.errors.push(error);
                    continue;
                }
            };
            let extracted = self.new_value();
            let mut nested_instructions = vec![Instruction {
                result: Some(TypedValue {
                    value: extracted,
                    ty: jir_ty,
                }),
                kind: InstructionKind::EnumExtract {
                    value,
                    variant,
                    field: field_index as u32,
                },
                span: Some(span),
            }];
            let next = blocks.get(position + 1).copied().unwrap_or(targets.matched);
            let terminator = self.lower_pattern_branch(
                extracted,
                field_ty,
                pattern,
                PatternTargets {
                    matched: next,
                    otherwise: targets.otherwise,
                },
                &mut nested_instructions,
                span,
            );
            self.set_synthetic_block(*block, nested_instructions, terminator, span);
        }
        Terminator::Branch {
            condition,
            then_target: blocks[0],
            then_arguments: Vec::new(),
            else_target: targets.otherwise,
            else_arguments: Vec::new(),
        }
    }

    fn reserve_synthetic_block(&mut self, span: Span) -> BlockId {
        let id = BlockId::new(self.source.blocks.len() + self.synthetic_blocks.len());
        self.synthetic_blocks.push(Block {
            id,
            parameters: Vec::new(),
            instructions: Vec::new(),
            terminator: Terminator::Unreachable,
            span: Some(span),
        });
        id
    }

    fn set_synthetic_block(
        &mut self,
        id: BlockId,
        instructions: Vec<Instruction>,
        terminator: Terminator,
        span: Span,
    ) {
        let index = id.index() - self.source.blocks.len();
        self.synthetic_blocks[index] = Block {
            id,
            parameters: Vec::new(),
            instructions,
            terminator,
            span: Some(span),
        };
    }

    fn emit_variant_condition(
        &mut self,
        value: ValueId,
        variant: u32,
        instructions: &mut Vec<Instruction>,
        span: Span,
    ) -> ValueId {
        let tag_ty = self.type_table.intern(Type::Integer {
            signed: false,
            bits: 32,
        });
        let tag = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue {
                value: tag,
                ty: tag_ty,
            }),
            kind: InstructionKind::EnumTag { value },
            span: Some(span),
        });
        let expected = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue {
                value: expected,
                ty: tag_ty,
            }),
            kind: InstructionKind::Constant(Constant::Integer {
                value: i128::from(variant),
            }),
            span: Some(span),
        });
        let bool_ty = self.type_table.intern(Type::Bool);
        let condition = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue {
                value: condition,
                ty: bool_ty,
            }),
            kind: InstructionKind::Compare {
                predicate: ComparePredicate::Equal,
                left: tag,
                right: expected,
            },
            span: Some(span),
        });
        condition
    }

    fn lower_operand(
        &mut self,
        operand: &MirOperand,
        instructions: &mut Vec<Instruction>,
    ) -> Option<Option<ValueId>> {
        let lowered_ty = match (
            &operand.kind,
            self.type_table.semantic.kind(operand.ty).cloned(),
        ) {
            (MirOperandKind::Place(place), _) if place.projection.is_empty() => self
                .local_types
                .get(place.local.index())
                .copied()
                .ok_or_else(|| LowerError {
                    span: Some(operand.span),
                    message: "MIR place local has no lowered JIR type".to_owned(),
                }),
            (MirOperandKind::RegionAllocate { .. }, Some(TypeKind::Buffer(element))) => self
                .type_table
                .lower_buffer(element, AddressSpace::Region, true, Some(operand.span)),
            _ => self.type_table.lower(operand.ty, Some(operand.span)),
        };
        let ty = match lowered_ty {
            Ok(ty) => ty,
            Err(error) => {
                self.errors.push(error);
                return None;
            }
        };
        if self.type_table.is_unit(ty) && matches!(operand.kind, MirOperandKind::Unit) {
            return Some(None);
        }
        let kind = match &operand.kind {
            MirOperandKind::Unit => return Some(None),
            MirOperandKind::Place(place) => {
                let pointer =
                    self.lower_place_address(place, &[], instructions, Some(operand.span))?;
                InstructionKind::Load {
                    pointer,
                    alignment: 1,
                    volatile: false,
                }
            }
            MirOperandKind::Literal(literal) => {
                if matches!(
                    self.type_table.semantic.kind(operand.ty),
                    Some(TypeKind::String)
                ) {
                    match decode_quoted(&literal.text, '"') {
                        Ok(utf8) => InstructionKind::StringLiteral { utf8 },
                        Err(message) => {
                            self.errors.push(self.error(Some(operand.span), message));
                            return None;
                        }
                    }
                } else {
                    match lower_constant(&literal.text, self.type_table.semantic.kind(operand.ty)) {
                        Ok(constant) => InstructionKind::Constant(constant),
                        Err(message) => {
                            self.errors.push(self.error(Some(operand.span), message));
                            return None;
                        }
                    }
                }
            }
            MirOperandKind::Unary {
                operator,
                operand: inner,
            } => {
                let operand = self.lower_required_operand(inner, instructions)?;
                if *operator == Operator::Plus {
                    return Some(Some(operand));
                }
                let Some(op) = lower_unary(*operator) else {
                    self.errors.push(self.error(
                        Some(inner.span),
                        format!("unsupported MIR unary operator {operator:?}"),
                    ));
                    return None;
                };
                InstructionKind::Unary { op, operand }
            }
            MirOperandKind::Cast { operand: inner } => {
                let source_value = self.lower_required_operand(inner, instructions)?;
                if inner.ty == operand.ty {
                    return Some(Some(source_value));
                }
                let source_kind = self.type_table.semantic.kind(inner.ty);
                let target_kind = self.type_table.semantic.kind(operand.ty);
                let Some(op) =
                    numeric_cast_op(source_kind, target_kind, self.type_table.pointer_bits)
                else {
                    self.errors.push(self.error(
                        Some(operand.span),
                        "MIR cast does not have a supported numeric JIR conversion",
                    ));
                    return None;
                };
                InstructionKind::Cast {
                    op,
                    value: source_value,
                    target: ty,
                }
            }
            MirOperandKind::Binary {
                left,
                operator,
                right,
            } => {
                let left = self.lower_required_operand(left, instructions)?;
                let right = self.lower_required_operand(right, instructions)?;
                if let Some(predicate) = lower_compare(*operator) {
                    if matches!(
                        self.type_table.semantic.kind(operand.ty),
                        Some(TypeKind::Vector { .. })
                    ) {
                        self.errors.push(self.error(
                            Some(operand.span),
                            "vector comparisons are not supported by the source-level SIMD contract",
                        ));
                        return None;
                    }
                    InstructionKind::Compare {
                        predicate,
                        left,
                        right,
                    }
                } else if let Some(op) = lower_binary(*operator) {
                    if matches!(
                        self.type_table.semantic.kind(operand.ty),
                        Some(TypeKind::Vector { .. })
                    ) {
                        InstructionKind::VectorBinary { op, left, right }
                    } else {
                        InstructionKind::Binary { op, left, right }
                    }
                } else {
                    self.errors.push(self.error(
                        Some(operand.span),
                        format!("unsupported MIR binary operator {operator:?}"),
                    ));
                    return None;
                }
            }
            MirOperandKind::Call { callee, arguments } => {
                let Some((name, symbol)) = (match &callee.kind {
                    MirOperandKind::Function { name, symbol } => Some((name, symbol)),
                    _ => None,
                }) else {
                    let callee_value = self.lower_required_operand(callee, instructions)?;
                    let expected_parameters = match self.type_table.semantic.kind(callee.ty) {
                        Some(TypeKind::Function { parameters, .. }) => parameters.to_vec(),
                        _ => {
                            self.errors.push(self.error(
                                Some(callee.span),
                                "indirect call callee is not a function value",
                            ));
                            return None;
                        }
                    };
                    let mut lowered_arguments = Vec::with_capacity(arguments.len());
                    for (index, argument) in arguments.iter().enumerate() {
                        lowered_arguments.push(self.lower_call_argument(
                            argument,
                            expected_parameters.get(index).copied(),
                            instructions,
                        )?);
                    }
                    let instruction = InstructionKind::IndirectCall {
                        callee: callee_value,
                        arguments: lowered_arguments,
                    };
                    if self.type_table.is_unit(ty) {
                        instructions.push(Instruction {
                            result: None,
                            kind: instruction,
                            span: Some(operand.span),
                        });
                        return Some(None);
                    }
                    let value = self.new_value();
                    instructions.push(Instruction {
                        result: Some(TypedValue { value, ty }),
                        kind: instruction,
                        span: Some(operand.span),
                    });
                    return Some(Some(value));
                };
                if is_lowered_vector_intrinsic(name) {
                    let instruction =
                        self.lower_vector_intrinsic(name, arguments, operand.span, instructions)?;
                    if self.type_table.is_unit(ty) {
                        instructions.push(Instruction {
                            result: None,
                            kind: instruction,
                            span: Some(operand.span),
                        });
                        return Some(None);
                    }
                    instruction
                } else if let Ok((variant, fields)) =
                    self.type_table
                        .variant_index_and_fields(operand.ty, name, Some(operand.span))
                {
                    if fields.len() != arguments.len() {
                        self.errors.push(self.error(
                            Some(operand.span),
                            "enum constructor argument count differs from layout",
                        ));
                        return None;
                    }
                    let mut lowered = Vec::with_capacity(arguments.len());
                    for argument in arguments {
                        lowered.push(self.lower_required_operand(argument, instructions)?);
                    }
                    InstructionKind::EnumConstruct {
                        variant,
                        fields: lowered,
                    }
                } else {
                    let function = symbol
                        .and_then(|symbol| self.call_targets.local.get(&symbol).copied())
                        .or_else(|| {
                            self.call_targets
                                .external
                                .get(&ExternalTarget {
                                    symbol: *symbol,
                                    name: name.clone(),
                                    parameters: arguments
                                        .iter()
                                        .map(|argument| argument.ty)
                                        .collect(),
                                    result: operand.ty,
                                })
                                .map(|external| external.id)
                        });
                    let Some(function) = function else {
                        self.errors.push(
                            self.error(Some(callee.span), "JIR call target was not registered"),
                        );
                        return None;
                    };
                    let mut lowered_arguments = Vec::with_capacity(arguments.len());
                    let expected_parameters =
                        match self.type_table.semantic.kind(callee.ty).cloned() {
                            Some(TypeKind::Function { parameters, .. }) => parameters.into_vec(),
                            _ => Vec::new(),
                        };
                    for (index, argument) in arguments.iter().enumerate() {
                        lowered_arguments.push(self.lower_call_argument(
                            argument,
                            expected_parameters.get(index).copied(),
                            instructions,
                        )?);
                    }
                    let instruction = InstructionKind::Call {
                        function,
                        arguments: lowered_arguments,
                    };
                    if self.type_table.is_unit(ty) {
                        instructions.push(Instruction {
                            result: None,
                            kind: instruction,
                            span: Some(operand.span),
                        });
                        return Some(None);
                    }
                    instruction
                }
            }
            MirOperandKind::Array(elements) => {
                let mut lowered = Vec::with_capacity(elements.len());
                for element in elements {
                    lowered.push(self.lower_required_operand(element, instructions)?);
                }
                InstructionKind::Aggregate { elements: lowered }
            }
            MirOperandKind::Struct { fields, .. } => {
                let layout = match self
                    .type_table
                    .record_fields(operand.ty, Some(operand.span))
                {
                    Ok(layout) => layout,
                    Err(error) => {
                        self.errors.push(error);
                        return None;
                    }
                };
                let mut lowered = Vec::with_capacity(layout.len());
                for (name, _) in layout {
                    let Some((_, value)) = fields.iter().find(|(candidate, _)| candidate == &name)
                    else {
                        self.errors.push(self.error(
                            Some(operand.span),
                            format!("record value is missing layout field {name:?}"),
                        ));
                        return None;
                    };
                    lowered.push(self.lower_required_operand(value, instructions)?);
                }
                InstructionKind::Aggregate { elements: lowered }
            }
            MirOperandKind::Field { base, field } => {
                let aggregate = self.lower_required_operand(base, instructions)?;
                let (index, _) =
                    match self
                        .type_table
                        .field_index_and_type(base.ty, field, Some(operand.span))
                    {
                        Ok(field) => field,
                        Err(error) => {
                            self.errors.push(error);
                            return None;
                        }
                    };
                InstructionKind::ExtractValue { aggregate, index }
            }
            MirOperandKind::Index { base, index } => {
                let mut aggregate = self.lower_required_operand(base, instructions)?;
                let index_value = self.lower_required_operand(index, instructions)?;
                let mut base_ty = base.ty;
                let mut capability_view = false;
                if let Some(TypeKind::Capability { inner, .. }) =
                    self.type_table.semantic.kind(base_ty).cloned()
                {
                    base_ty = inner;
                    if let Some(inner_ty) = self.operand_capability_pointee(base) {
                        let loaded = self.new_value();
                        instructions.push(Instruction {
                            result: Some(TypedValue {
                                value: loaded,
                                ty: inner_ty,
                            }),
                            kind: InstructionKind::Load {
                                pointer: aggregate,
                                alignment: 1,
                                volatile: false,
                            },
                            span: Some(base.span),
                        });
                        aggregate = loaded;
                    } else {
                        capability_view = true;
                    }
                }
                match self.type_table.semantic.kind(base_ty).cloned() {
                    Some(TypeKind::Array { length, .. }) => {
                        let index_ty = match self.type_table.lower(index.ty, Some(index.span)) {
                            Ok(index_ty) => index_ty,
                            Err(error) => {
                                self.errors.push(error);
                                return None;
                            }
                        };
                        let length_value = self.new_value();
                        instructions.push(Instruction {
                            result: Some(TypedValue {
                                value: length_value,
                                ty: index_ty,
                            }),
                            kind: InstructionKind::Constant(Constant::Integer {
                                value: i128::from(length),
                            }),
                            span: Some(index.span),
                        });
                        instructions.push(Instruction {
                            result: None,
                            kind: InstructionKind::BoundsCheck {
                                index: index_value,
                                length: length_value,
                            },
                            span: Some(operand.span),
                        });
                        InstructionKind::ExtractElement {
                            aggregate,
                            index: index_value,
                        }
                    }
                    Some(TypeKind::Buffer(element) | TypeKind::Slice(element)) => {
                        let address_space = if capability_view {
                            AddressSpace::Generic
                        } else if self.operand_is_region_owned(base) {
                            AddressSpace::Region
                        } else if matches!(
                            self.type_table.semantic.kind(base_ty),
                            Some(TypeKind::Buffer(_))
                        ) {
                            AddressSpace::Heap
                        } else {
                            AddressSpace::Generic
                        };
                        let element_ty = match self.type_table.lower(element, Some(operand.span)) {
                            Ok(ty) => ty,
                            Err(error) => {
                                self.errors.push(error);
                                return None;
                            }
                        };
                        let data_ty = self.type_table.intern(Type::Pointer {
                            pointee: element_ty,
                            address_space,
                        });
                        let size_ty = self.type_table.intern(Type::Integer {
                            signed: false,
                            bits: self.type_table.pointer_bits,
                        });
                        let data = self.new_value();
                        instructions.push(Instruction {
                            result: Some(TypedValue {
                                value: data,
                                ty: data_ty,
                            }),
                            kind: InstructionKind::ExtractValue {
                                aggregate,
                                index: 0,
                            },
                            span: Some(base.span),
                        });
                        let length = self.new_value();
                        instructions.push(Instruction {
                            result: Some(TypedValue {
                                value: length,
                                ty: size_ty,
                            }),
                            kind: InstructionKind::ExtractValue {
                                aggregate,
                                index: 1,
                            },
                            span: Some(base.span),
                        });
                        let index = self.lower_index_to_size(
                            index_value,
                            index.ty,
                            size_ty,
                            instructions,
                            index.span,
                        )?;
                        instructions.push(Instruction {
                            result: None,
                            kind: InstructionKind::BoundsCheck { index, length },
                            span: Some(operand.span),
                        });
                        let pointer = self.new_value();
                        instructions.push(Instruction {
                            result: Some(TypedValue {
                                value: pointer,
                                ty: data_ty,
                            }),
                            kind: InstructionKind::Offset {
                                base: data,
                                indices: vec![index],
                            },
                            span: Some(operand.span),
                        });
                        InstructionKind::Load {
                            pointer,
                            alignment: 1,
                            volatile: false,
                        }
                    }
                    _ => {
                        self.errors.push(self.error(
                            Some(operand.span),
                            "JIR indexing requires an array, Buffer, or Slice",
                        ));
                        return None;
                    }
                }
            }
            MirOperandKind::Length { base } => {
                let mut base_ty = base.ty;
                if let Some(TypeKind::Capability { inner, .. }) =
                    self.type_table.semantic.kind(base_ty).cloned()
                {
                    base_ty = inner;
                }
                match self.type_table.semantic.kind(base_ty).cloned() {
                    Some(TypeKind::Array { length, .. }) => {
                        InstructionKind::Constant(Constant::Integer {
                            value: i128::from(length),
                        })
                    }
                    Some(TypeKind::Buffer(_) | TypeKind::Slice(_)) => {
                        let aggregate = self.lower_required_operand(base, instructions)?;
                        InstructionKind::ExtractValue {
                            aggregate,
                            index: 1,
                        }
                    }
                    _ => {
                        self.errors.push(self.error(
                            Some(operand.span),
                            "JIR length requires an array, Buffer, or Slice",
                        ));
                        return None;
                    }
                }
            }
            MirOperandKind::Function { name, .. } => {
                match self
                    .type_table
                    .variant_index_and_fields(operand.ty, name, Some(operand.span))
                {
                    Ok((variant, fields)) if fields.is_empty() => InstructionKind::EnumConstruct {
                        variant,
                        fields: Vec::new(),
                    },
                    Ok(_) => {
                        self.errors.push(self.error(
                            Some(operand.span),
                            "payload enum constructor must be called",
                        ));
                        return None;
                    }
                    Err(_) => {
                        let MirOperandKind::Function { symbol, .. } = &operand.kind else {
                            unreachable!("matched function operand");
                        };
                        let target = symbol
                            .as_ref()
                            .and_then(|symbol| self.call_targets.local.get(symbol).copied())
                            .or_else(|| {
                                let Some(TypeKind::Function { parameters, result }) =
                                    self.type_table.semantic.kind(operand.ty)
                                else {
                                    return None;
                                };
                                let target = ExternalTarget {
                                    symbol: *symbol,
                                    name: name.clone(),
                                    parameters: parameters.to_vec(),
                                    result: *result,
                                };
                                self.call_targets
                                    .external
                                    .get(&target)
                                    .map(|external| external.id)
                            });
                        let Some(function) = target else {
                            self.errors.push(self.error(
                                Some(operand.span),
                                "function value target was not registered",
                            ));
                            return None;
                        };
                        InstructionKind::FunctionAddress { function }
                    }
                }
            }
            MirOperandKind::PatternExtract {
                source,
                source_indices,
                path,
                ..
            } => {
                return self
                    .lower_pattern_extract(source, source_indices, path, instructions, operand.span)
                    .map(Some);
            }
            MirOperandKind::CarrierExtract {
                source,
                source_indices,
                part,
            } => {
                let (carrier, carrier_ty) =
                    self.load_source_place(source, source_indices, instructions, operand.span)?;
                let (variant, field) = match (self.type_table.semantic.kind(carrier_ty), part) {
                    (Some(TypeKind::Option(_)), CarrierPart::Success)
                    | (Some(TypeKind::Result { .. }), CarrierPart::Success) => {
                        (CarrierTag::Success.raw(), 0)
                    }
                    (Some(TypeKind::Result { .. }), CarrierPart::Residual) => {
                        (CarrierTag::Residual.raw(), 0)
                    }
                    _ => {
                        self.errors.push(
                            self.error(Some(operand.span), "invalid carrier payload extraction"),
                        );
                        return None;
                    }
                };
                InstructionKind::EnumExtract {
                    value: carrier,
                    variant,
                    field,
                }
            }
            MirOperandKind::PropagateResidual { source, kind, .. } => {
                let (carrier, carrier_ty) =
                    self.load_source_place(source, &[], instructions, operand.span)?;
                match kind {
                    MirPropagationKind::OptionNone => InstructionKind::EnumConstruct {
                        variant: CarrierTag::Residual.raw(),
                        fields: Vec::new(),
                    },
                    MirPropagationKind::ResultError => {
                        let error_ty = match self.type_table.semantic.kind(carrier_ty) {
                            Some(TypeKind::Result { error, .. }) => *error,
                            _ => {
                                self.errors.push(self.error(
                                    Some(operand.span),
                                    "Result residual source has a non-Result type",
                                ));
                                return None;
                            }
                        };
                        let error_jir = match self.type_table.lower(error_ty, Some(operand.span)) {
                            Ok(ty) => ty,
                            Err(error) => {
                                self.errors.push(error);
                                return None;
                            }
                        };
                        let error = self.new_value();
                        instructions.push(Instruction {
                            result: Some(TypedValue {
                                value: error,
                                ty: error_jir,
                            }),
                            kind: InstructionKind::EnumExtract {
                                value: carrier,
                                variant: CarrierTag::Residual.raw(),
                                field: 0,
                            },
                            span: Some(operand.span),
                        });
                        InstructionKind::EnumConstruct {
                            variant: CarrierTag::Residual.raw(),
                            fields: vec![error],
                        }
                    }
                }
            }
            MirOperandKind::RegionAllocate { region, arguments } => {
                let Some(region) = region else {
                    self.errors.push(
                        self.error(Some(operand.span), "region allocation has no region local"),
                    );
                    return None;
                };
                let (region, _) = self.load_source_place(
                    &Place::local(*region),
                    &[],
                    instructions,
                    operand.span,
                )?;
                let Some(count_operand) = arguments.first() else {
                    self.errors.push(
                        self.error(Some(operand.span), "region allocation has no element count"),
                    );
                    return None;
                };
                let count = self.lower_required_operand(count_operand, instructions)?;
                let size_ty = self.type_table.intern(Type::Integer {
                    signed: false,
                    bits: self.type_table.pointer_bits,
                });
                let count = self.lower_index_to_size(
                    count,
                    count_operand.ty,
                    size_ty,
                    instructions,
                    count_operand.span,
                )?;
                let Some(TypeKind::Buffer(element)) =
                    self.type_table.semantic.kind(operand.ty).cloned()
                else {
                    self.errors.push(
                        self.error(Some(operand.span), "region allocation result is not Buffer"),
                    );
                    return None;
                };
                let element_ty = match self.type_table.lower(element, Some(operand.span)) {
                    Ok(ty) => ty,
                    Err(error) => {
                        self.errors.push(error);
                        return None;
                    }
                };
                let pointer_ty = self.type_table.intern(Type::Pointer {
                    pointee: element_ty,
                    address_space: AddressSpace::Region,
                });
                let data = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: data,
                        ty: pointer_ty,
                    }),
                    kind: InstructionKind::RegionAlloc {
                        region,
                        ty: element_ty,
                        count,
                    },
                    span: Some(operand.span),
                });
                // Region runtime storage is raw and therefore has no
                // initialized elements yet. Keep the logical length at zero
                // until a typed initialization operation publishes elements;
                // the requested count remains the capacity.
                let length = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: length,
                        ty: size_ty,
                    }),
                    kind: InstructionKind::Constant(Constant::Integer { value: 0 }),
                    span: Some(operand.span),
                });
                InstructionKind::Aggregate {
                    elements: vec![data, length, count],
                }
            }
            MirOperandKind::HighLevel(_) => {
                self.errors.push(self.error(
                    Some(operand.span),
                    format!(
                        "JIR lowering does not yet support MIR operand {:?}",
                        operand.kind
                    ),
                ));
                return None;
            }
        };
        let value = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue { value, ty }),
            kind,
            span: Some(operand.span),
        });
        Some(Some(value))
    }

    fn lower_required_operand(
        &mut self,
        operand: &MirOperand,
        instructions: &mut Vec<Instruction>,
    ) -> Option<ValueId> {
        match self.lower_operand(operand, instructions)? {
            Some(value) => Some(value),
            None => {
                self.errors.push(self.error(
                    Some(operand.span),
                    "Unit operand cannot be used as an instruction value",
                ));
                None
            }
        }
    }

    fn lower_vector_intrinsic(
        &mut self,
        name: &str,
        arguments: &[MirOperand],
        span: Span,
        instructions: &mut Vec<Instruction>,
    ) -> Option<InstructionKind> {
        let core = self.type_table.semantic.core();
        let float32 = core.float32;
        let lanes = vector_intrinsic_lanes(name)?;
        let vector = match lanes {
            2 => core.float2,
            3 => core.float3,
            4 => core.float4,
            8 => core.float8,
            _ => return None,
        };
        let slice = self.type_table.semantic.intern(TypeKind::Slice(float32));
        let capability = match name {
            "vector_load2" | "vector_load3" | "vector_load4" | "vector_load8" => Capability::Read,
            "vector_store2" | "vector_store3" | "vector_store4" | "vector_store8" => {
                Capability::Write
            }
            "vector_splat2" | "vector_splat3" | "vector_splat4" | "vector_splat8" => {
                Capability::Owned
            }
            _ => return None,
        };
        if matches!(
            name,
            "vector_splat2" | "vector_splat3" | "vector_splat4" | "vector_splat8"
        ) {
            let value = arguments
                .first()
                .and_then(|argument| self.lower_required_operand(argument, instructions))?;
            return Some(InstructionKind::VectorSplat { value, lanes });
        }
        let expected_slice = self.type_table.semantic.intern(TypeKind::Capability {
            capability,
            inner: slice,
        });
        let slice_argument = arguments.first()?;
        let slice_value =
            self.lower_call_argument(slice_argument, Some(expected_slice), instructions)?;
        let index_argument = arguments.get(1)?;
        let index = self.lower_required_operand(index_argument, instructions)?;
        let slice_ty = self.type_table.lower(slice, Some(span)).ok()?;
        let fields = match self.type_table.types.get(slice_ty.index()) {
            Some(Type::Struct { fields }) if fields.len() >= 2 => fields.clone(),
            _ => {
                self.errors.push(self.error(
                    Some(span),
                    "vector intrinsic slice does not have a pointer/length layout",
                ));
                return None;
            }
        };
        let data = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue {
                value: data,
                ty: fields[0],
            }),
            kind: InstructionKind::ExtractValue {
                aggregate: slice_value,
                index: 0,
            },
            span: Some(span),
        });
        let length = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue {
                value: length,
                ty: fields[1],
            }),
            kind: InstructionKind::ExtractValue {
                aggregate: slice_value,
                index: 1,
            },
            span: Some(span),
        });
        instructions.push(Instruction {
            result: None,
            kind: InstructionKind::VectorBoundsCheck {
                index,
                length,
                lanes,
            },
            span: Some(span),
        });
        let offset = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue {
                value: offset,
                ty: fields[0],
            }),
            kind: InstructionKind::Offset {
                base: data,
                indices: vec![index],
            },
            span: Some(span),
        });
        let vector_ty = self.type_table.lower(vector, Some(span)).ok()?;
        let vector_pointer_ty = self.type_table.pointer(vector_ty, AddressSpace::Generic);
        let vector_pointer = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue {
                value: vector_pointer,
                ty: vector_pointer_ty,
            }),
            kind: InstructionKind::Cast {
                op: CastOp::PointerCast,
                value: offset,
                target: vector_pointer_ty,
            },
            span: Some(span),
        });
        if matches!(
            name,
            "vector_load2" | "vector_load3" | "vector_load4" | "vector_load8"
        ) {
            return Some(InstructionKind::Load {
                pointer: vector_pointer,
                alignment: 4,
                volatile: false,
            });
        }
        let value = arguments
            .get(2)
            .and_then(|argument| self.lower_required_operand(argument, instructions))?;
        Some(InstructionKind::Store {
            pointer: vector_pointer,
            value,
            alignment: 4,
            volatile: false,
        })
    }

    fn lower_call_argument(
        &mut self,
        argument: &MirOperand,
        expected: Option<SemanticTypeId>,
        instructions: &mut Vec<Instruction>,
    ) -> Option<ValueId> {
        let Some(TypeKind::Capability {
            capability: Capability::Read | Capability::Write,
            inner,
        }) = expected.and_then(|ty| self.type_table.semantic.kind(ty).cloned())
        else {
            return self.lower_required_operand(argument, instructions);
        };
        if matches!(
            self.type_table.semantic.kind(argument.ty),
            Some(TypeKind::Capability { .. })
        ) {
            return self.lower_required_operand(argument, instructions);
        }
        let MirOperandKind::Place(place) = &argument.kind else {
            self.errors.push(self.error(
                Some(argument.span),
                "call-scoped capability coercion requires an addressable MIR place",
            ));
            return None;
        };
        self.lower_borrow_from_place(place, inner, instructions, argument.span)
    }

    fn lower_borrow_from_place(
        &mut self,
        source: &Place,
        inner: SemanticTypeId,
        instructions: &mut Vec<Instruction>,
        span: Span,
    ) -> Option<ValueId> {
        let source_pointer = self.lower_place_address(source, &[], instructions, Some(span))?;
        match self.type_table.semantic.kind(inner).cloned() {
            Some(TypeKind::Buffer(element) | TypeKind::Slice(element)) => {
                let source_ty = if source.projection.is_empty() {
                    self.local_types.get(source.local.index()).copied()
                } else {
                    self.type_table.lower(inner, Some(span)).ok()
                }?;
                let fields = match self.type_table.types.get(source_ty.index()) {
                    Some(Type::Struct { fields }) if fields.len() >= 2 => fields.clone(),
                    _ => {
                        self.errors.push(self.error(
                            Some(span),
                            "Buffer/Slice borrow source has no aggregate representation",
                        ));
                        return None;
                    }
                };
                let source_value = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: source_value,
                        ty: source_ty,
                    }),
                    kind: InstructionKind::Load {
                        pointer: source_pointer,
                        alignment: 1,
                        volatile: false,
                    },
                    span: Some(span),
                });
                let data = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: data,
                        ty: fields[0],
                    }),
                    kind: InstructionKind::ExtractValue {
                        aggregate: source_value,
                        index: 0,
                    },
                    span: Some(span),
                });
                let length = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: length,
                        ty: fields[1],
                    }),
                    kind: InstructionKind::ExtractValue {
                        aggregate: source_value,
                        index: 1,
                    },
                    span: Some(span),
                });
                let element_ty = match self.type_table.lower(element, Some(span)) {
                    Ok(ty) => ty,
                    Err(error) => {
                        self.errors.push(error);
                        return None;
                    }
                };
                let generic_data_ty = self.type_table.pointer(element_ty, AddressSpace::Generic);
                let data = if fields[0] == generic_data_ty {
                    data
                } else {
                    let cast = self.new_value();
                    instructions.push(Instruction {
                        result: Some(TypedValue {
                            value: cast,
                            ty: generic_data_ty,
                        }),
                        kind: InstructionKind::Cast {
                            op: crate::CastOp::PointerCast,
                            value: data,
                            target: generic_data_ty,
                        },
                        span: Some(span),
                    });
                    cast
                };
                let view_ty =
                    match self
                        .type_table
                        .lower_borrow_capability(inner, false, Some(span))
                    {
                        Ok(ty) => ty,
                        Err(error) => {
                            self.errors.push(error);
                            return None;
                        }
                    };
                let view = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: view,
                        ty: view_ty,
                    }),
                    kind: InstructionKind::Aggregate {
                        elements: vec![data, length],
                    },
                    span: Some(span),
                });
                Some(view)
            }
            _ => {
                let target = match self
                    .type_table
                    .lower_borrow_capability(inner, false, Some(span))
                {
                    Ok(ty) => ty,
                    Err(error) => {
                        self.errors.push(error);
                        return None;
                    }
                };
                let borrowed = self.new_value();
                instructions.push(Instruction {
                    result: Some(TypedValue {
                        value: borrowed,
                        ty: target,
                    }),
                    kind: InstructionKind::Cast {
                        op: crate::CastOp::PointerCast,
                        value: source_pointer,
                        target,
                    },
                    span: Some(span),
                });
                Some(borrowed)
            }
        }
    }

    fn operand_is_region_owned(&self, operand: &MirOperand) -> bool {
        match &operand.kind {
            MirOperandKind::Place(place) => self
                .source
                .locals
                .get(place.local.index())
                .is_some_and(|local| local.owned_region.is_some()),
            MirOperandKind::Field { base, .. } | MirOperandKind::Index { base, .. } => {
                self.operand_is_region_owned(base)
            }
            _ => false,
        }
    }

    fn operand_capability_pointee(&self, operand: &MirOperand) -> Option<TypeId> {
        let MirOperandKind::Place(place) = &operand.kind else {
            return None;
        };
        if !place.projection.is_empty() {
            return None;
        }
        let local_ty = *self.local_types.get(place.local.index())?;
        match self.type_table.types.get(local_ty.index()) {
            Some(Type::Pointer { pointee, .. }) => Some(*pointee),
            _ => None,
        }
    }

    fn load_source_place(
        &mut self,
        place: &Place,
        dynamic_indices: &[MirOperand],
        instructions: &mut Vec<Instruction>,
        span: Span,
    ) -> Option<(ValueId, SemanticTypeId)> {
        let mut semantic_ty = self.source.locals.get(place.local.index())?.ty;
        let provided_indices = dynamic_indices;
        let mut dynamic_indices = provided_indices.iter();
        for projection in &place.projection {
            if let Some(TypeKind::Capability { inner, .. }) =
                self.type_table.semantic.kind(semantic_ty).cloned()
            {
                semantic_ty = inner;
            }
            semantic_ty = match projection {
                Projection::Field(field) => {
                    match self
                        .type_table
                        .field_index_and_type(semantic_ty, field, Some(span))
                    {
                        Ok((_, field_ty)) => field_ty,
                        Err(error) => {
                            self.errors.push(error);
                            return None;
                        }
                    }
                }
                Projection::Dereference => {
                    let Some(TypeKind::Pointer(pointee)) =
                        self.type_table.semantic.kind(semantic_ty).cloned()
                    else {
                        self.errors.push(self.error(
                            Some(span),
                            "pattern/carrier source dereference requires a Pointer<T> place",
                        ));
                        return None;
                    };
                    pointee
                }
                Projection::Index => {
                    let Some(index_operand) = dynamic_indices.next() else {
                        self.errors.push(self.error(
                            Some(span),
                            "pattern/carrier source index projection requires an explicit index operand",
                        ));
                        return None;
                    };
                    let element = match self.type_table.semantic.kind(semantic_ty).cloned() {
                        Some(TypeKind::Array { element, .. })
                        | Some(TypeKind::Buffer(element))
                        | Some(TypeKind::Slice(element)) => element,
                        _ => {
                            self.errors.push(self.error(
                                Some(span),
                                "pattern/carrier source indexing requires an array, Buffer, or Slice",
                            ));
                            return None;
                        }
                    };
                    let _ = index_operand;
                    element
                }
            };
        }
        if dynamic_indices.next().is_some() {
            self.errors.push(self.error(
                Some(span),
                "pattern/carrier source has more index operands than index projections",
            ));
            return None;
        }
        let ty = match self.type_table.lower(semantic_ty, Some(span)) {
            Ok(ty) => ty,
            Err(error) => {
                self.errors.push(error);
                return None;
            }
        };
        let pointer =
            self.lower_place_address(place, provided_indices, instructions, Some(span))?;
        let value = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue { value, ty }),
            kind: InstructionKind::Load {
                pointer,
                alignment: 1,
                volatile: false,
            },
            span: Some(span),
        });
        Some((value, semantic_ty))
    }

    fn lower_pattern_extract(
        &mut self,
        source: &Place,
        source_indices: &[MirOperand],
        path: &[String],
        instructions: &mut Vec<Instruction>,
        span: Span,
    ) -> Option<ValueId> {
        let (mut value, mut ty) =
            self.load_source_place(source, source_indices, instructions, span)?;
        let mut variant: Option<(u32, Vec<SemanticTypeId>)> = None;
        for segment in path {
            if let Some(path) = segment.strip_prefix("$variant:") {
                variant = match self
                    .type_table
                    .variant_index_and_fields(ty, path, Some(span))
                {
                    Ok(variant) => Some(variant),
                    Err(error) => {
                        self.errors.push(error);
                        return None;
                    }
                };
                continue;
            }
            let Some(index) = segment
                .strip_prefix("$payload")
                .and_then(|index| index.parse::<usize>().ok())
            else {
                self.errors.push(self.error(
                    Some(span),
                    format!("invalid MIR pattern extraction segment {segment:?}"),
                ));
                return None;
            };
            let Some((variant_index, fields)) = variant.take() else {
                self.errors.push(self.error(
                    Some(span),
                    "pattern payload segment has no preceding variant",
                ));
                return None;
            };
            let Some(field_ty) = fields.get(index).copied() else {
                self.errors
                    .push(self.error(Some(span), "pattern payload index exceeds enum layout"));
                return None;
            };
            let jir_ty = match self.type_table.lower(field_ty, Some(span)) {
                Ok(ty) => ty,
                Err(error) => {
                    self.errors.push(error);
                    return None;
                }
            };
            let extracted = self.new_value();
            instructions.push(Instruction {
                result: Some(TypedValue {
                    value: extracted,
                    ty: jir_ty,
                }),
                kind: InstructionKind::EnumExtract {
                    value,
                    variant: variant_index,
                    field: index as u32,
                },
                span: Some(span),
            });
            value = extracted;
            ty = field_ty;
        }
        Some(value)
    }

    fn lower_index_to_size(
        &mut self,
        value: ValueId,
        semantic_ty: SemanticTypeId,
        size_ty: TypeId,
        instructions: &mut Vec<Instruction>,
        span: Span,
    ) -> Option<ValueId> {
        let Some(TypeKind::Integer { signedness, width }) =
            self.type_table.semantic.kind(semantic_ty).cloned()
        else {
            self.errors
                .push(self.error(Some(span), "buffer index is not an integer"));
            return None;
        };
        let bits = integer_bits(width, self.type_table.pointer_bits);
        if bits == self.type_table.pointer_bits && signedness == Signedness::Unsigned {
            return Some(value);
        }
        if bits > self.type_table.pointer_bits {
            self.errors.push(self.error(
                Some(span),
                "index wider than the target pointer size requires a checked narrowing pass",
            ));
            return None;
        }
        let op = if bits < self.type_table.pointer_bits {
            crate::CastOp::IntegerExtend
        } else {
            crate::CastOp::Bitcast
        };
        let converted = self.new_value();
        instructions.push(Instruction {
            result: Some(TypedValue {
                value: converted,
                ty: size_ty,
            }),
            kind: InstructionKind::Cast {
                op,
                value,
                target: size_ty,
            },
            span: Some(span),
        });
        Some(converted)
    }

    fn lower_place_address(
        &mut self,
        place: &Place,
        dynamic_indices: &[MirOperand],
        instructions: &mut Vec<Instruction>,
        span: Option<Span>,
    ) -> Option<ValueId> {
        let mut address = self
            .local_addresses
            .get(place.local.index())
            .copied()
            .or_else(|| {
                self.errors.push(self.error(
                    span,
                    format!("MIR local #{} does not exist", place.local.index()),
                ));
                None
            })?;
        let mut current_ty = self.source.locals.get(place.local.index())?.ty;
        let mut current_address_space = AddressSpace::Stack;
        let mut specialized_current_jir = None;
        let mut capability_view = None;
        if !place.projection.is_empty()
            && let Some(TypeKind::Capability { inner, .. }) =
                self.type_table.semantic.kind(current_ty).cloned()
        {
            let capability_ty = self.local_types.get(place.local.index()).copied()?;
            specialized_current_jir = match self.type_table.types.get(capability_ty.index()) {
                Some(Type::Pointer { pointee, .. }) => Some(*pointee),
                Some(Type::Struct { .. }) => Some(capability_ty),
                _ => None,
            };
            let dereferenced = self.new_value();
            instructions.push(Instruction {
                result: Some(TypedValue {
                    value: dereferenced,
                    ty: capability_ty,
                }),
                kind: InstructionKind::Load {
                    pointer: address,
                    alignment: 1,
                    volatile: false,
                },
                span,
            });
            if matches!(
                self.type_table.types.get(capability_ty.index()),
                Some(Type::Pointer { .. })
            ) {
                address = dereferenced;
            } else {
                capability_view = Some(dereferenced);
            }
            current_ty = inner;
            current_address_space = AddressSpace::Generic;
        }
        let mut dynamic_indices = dynamic_indices.iter();
        for projection in &place.projection {
            match projection {
                Projection::Field(field) => {
                    let (field_index, field_ty) = match self
                        .type_table
                        .field_index_and_type(current_ty, field, span)
                    {
                        Ok(field) => field,
                        Err(error) => {
                            self.errors.push(error);
                            return None;
                        }
                    };
                    let index_ty = self.type_table.intern(Type::Integer {
                        signed: false,
                        bits: self.type_table.pointer_bits,
                    });
                    let index = self.new_value();
                    instructions.push(Instruction {
                        result: Some(TypedValue {
                            value: index,
                            ty: index_ty,
                        }),
                        kind: InstructionKind::Constant(Constant::Integer {
                            value: i128::from(field_index),
                        }),
                        span,
                    });
                    let pointee = match self.type_table.lower(field_ty, span) {
                        Ok(ty) => ty,
                        Err(error) => {
                            self.errors.push(error);
                            return None;
                        }
                    };
                    let pointer_ty = self.type_table.pointer(pointee, current_address_space);
                    let projected = self.new_value();
                    instructions.push(Instruction {
                        result: Some(TypedValue {
                            value: projected,
                            ty: pointer_ty,
                        }),
                        kind: InstructionKind::Offset {
                            base: address,
                            indices: vec![index],
                        },
                        span,
                    });
                    address = projected;
                    current_ty = field_ty;
                    specialized_current_jir = None;
                }
                Projection::Index => {
                    let Some(index_operand) = dynamic_indices.next() else {
                        self.errors.push(self.error(
                            span,
                            "projected MIR place is missing its dynamic index operand",
                        ));
                        return None;
                    };
                    let index = self.lower_required_operand(index_operand, instructions)?;
                    match self.type_table.semantic.kind(current_ty).cloned() {
                        Some(TypeKind::Array { element, length }) => {
                            let index_ty = match self.type_table.lower(index_operand.ty, span) {
                                Ok(ty) => ty,
                                Err(error) => {
                                    self.errors.push(error);
                                    return None;
                                }
                            };
                            let length_value = self.new_value();
                            instructions.push(Instruction {
                                result: Some(TypedValue {
                                    value: length_value,
                                    ty: index_ty,
                                }),
                                kind: InstructionKind::Constant(Constant::Integer {
                                    value: i128::from(length),
                                }),
                                span,
                            });
                            instructions.push(Instruction {
                                result: None,
                                kind: InstructionKind::BoundsCheck {
                                    index,
                                    length: length_value,
                                },
                                span,
                            });
                            let pointee = match self.type_table.lower(element, span) {
                                Ok(ty) => ty,
                                Err(error) => {
                                    self.errors.push(error);
                                    return None;
                                }
                            };
                            let pointer_ty =
                                self.type_table.pointer(pointee, current_address_space);
                            let projected = self.new_value();
                            instructions.push(Instruction {
                                result: Some(TypedValue {
                                    value: projected,
                                    ty: pointer_ty,
                                }),
                                kind: InstructionKind::Offset {
                                    base: address,
                                    indices: vec![index],
                                },
                                span,
                            });
                            address = projected;
                            current_ty = element;
                        }
                        Some(TypeKind::Buffer(element) | TypeKind::Slice(element)) => {
                            let aggregate_ty = if let Some(ty) = specialized_current_jir.take() {
                                ty
                            } else {
                                match self.type_table.lower(current_ty, span) {
                                    Ok(ty) => ty,
                                    Err(error) => {
                                        self.errors.push(error);
                                        return None;
                                    }
                                }
                            };
                            let aggregate = if let Some(view) = capability_view.take() {
                                view
                            } else {
                                let aggregate = self.new_value();
                                instructions.push(Instruction {
                                    result: Some(TypedValue {
                                        value: aggregate,
                                        ty: aggregate_ty,
                                    }),
                                    kind: InstructionKind::Load {
                                        pointer: address,
                                        alignment: 1,
                                        volatile: false,
                                    },
                                    span,
                                });
                                aggregate
                            };
                            let address_space = if capability_view.is_some()
                                || matches!(
                                    self.type_table.types.get(aggregate_ty.index()),
                                    Some(Type::Struct { fields })
                                        if fields.len() == 2
                                            && matches!(
                                                self.type_table.types.get(fields[0].index()),
                                                Some(Type::Pointer {
                                                    address_space: AddressSpace::Generic,
                                                    ..
                                                })
                                            )
                                ) {
                                AddressSpace::Generic
                            } else if self
                                .source
                                .locals
                                .get(place.local.index())
                                .is_some_and(|local| local.owned_region.is_some())
                            {
                                AddressSpace::Region
                            } else if matches!(
                                self.type_table.semantic.kind(current_ty),
                                Some(TypeKind::Buffer(_))
                            ) {
                                AddressSpace::Heap
                            } else {
                                AddressSpace::Generic
                            };
                            let pointee = match self.type_table.lower(element, span) {
                                Ok(ty) => ty,
                                Err(error) => {
                                    self.errors.push(error);
                                    return None;
                                }
                            };
                            let data_ty = self.type_table.intern(Type::Pointer {
                                pointee,
                                address_space,
                            });
                            let size_ty = self.type_table.intern(Type::Integer {
                                signed: false,
                                bits: self.type_table.pointer_bits,
                            });
                            let data = self.new_value();
                            instructions.push(Instruction {
                                result: Some(TypedValue {
                                    value: data,
                                    ty: data_ty,
                                }),
                                kind: InstructionKind::ExtractValue {
                                    aggregate,
                                    index: 0,
                                },
                                span,
                            });
                            let length = self.new_value();
                            instructions.push(Instruction {
                                result: Some(TypedValue {
                                    value: length,
                                    ty: size_ty,
                                }),
                                kind: InstructionKind::ExtractValue {
                                    aggregate,
                                    index: 1,
                                },
                                span,
                            });
                            let index = self.lower_index_to_size(
                                index,
                                index_operand.ty,
                                size_ty,
                                instructions,
                                index_operand.span,
                            )?;
                            instructions.push(Instruction {
                                result: None,
                                kind: InstructionKind::BoundsCheck { index, length },
                                span,
                            });
                            let projected = self.new_value();
                            instructions.push(Instruction {
                                result: Some(TypedValue {
                                    value: projected,
                                    ty: data_ty,
                                }),
                                kind: InstructionKind::Offset {
                                    base: data,
                                    indices: vec![index],
                                },
                                span,
                            });
                            address = projected;
                            current_ty = element;
                            current_address_space = address_space;
                        }
                        _ => {
                            self.errors.push(self.error(
                                span,
                                "projected JIR place indexing requires an array, Buffer, or Slice",
                            ));
                            return None;
                        }
                    }
                }
                Projection::Dereference => {
                    let Some(TypeKind::Pointer(pointee)) =
                        self.type_table.semantic.kind(current_ty).cloned()
                    else {
                        self.errors.push(self.error(
                            span,
                            "pointer dereference projection requires a Pointer<T> place",
                        ));
                        return None;
                    };
                    let pointer_ty = match self.type_table.lower(current_ty, span) {
                        Ok(ty) => ty,
                        Err(error) => {
                            self.errors.push(error);
                            return None;
                        }
                    };
                    let loaded = self.new_value();
                    instructions.push(Instruction {
                        result: Some(TypedValue {
                            value: loaded,
                            ty: pointer_ty,
                        }),
                        kind: InstructionKind::Load {
                            pointer: address,
                            alignment: 1,
                            volatile: false,
                        },
                        span,
                    });
                    address = loaded;
                    current_ty = pointee;
                    current_address_space = AddressSpace::Generic;
                    specialized_current_jir = None;
                    capability_view = None;
                }
            }
        }
        if dynamic_indices.next().is_some() {
            self.errors.push(self.error(
                span,
                "MIR assignment has more dynamic indices than place projections",
            ));
            return None;
        }
        Some(address)
    }

    fn new_value(&mut self) -> ValueId {
        let value = ValueId::new(self.next_value);
        self.next_value += 1;
        value
    }

    fn error(&self, span: Option<Span>, message: impl Into<String>) -> LowerError {
        LowerError {
            span,
            message: message.into(),
        }
    }
}

fn lower_block(block: MirBlockId) -> BlockId {
    BlockId::new(block.index())
}

fn numeric_cast_op(
    source: Option<&TypeKind>,
    target: Option<&TypeKind>,
    pointer_bits: u16,
) -> Option<CastOp> {
    match (source, target) {
        (
            Some(TypeKind::Integer { width: source, .. }),
            Some(TypeKind::Integer { width: target, .. }),
        ) => {
            let source = integer_bits(*source, pointer_bits);
            let target = integer_bits(*target, pointer_bits);
            Some(if source < target {
                CastOp::IntegerExtend
            } else if source > target {
                CastOp::IntegerTruncate
            } else {
                CastOp::Bitcast
            })
        }
        (Some(TypeKind::Integer { .. }), Some(TypeKind::Float { .. })) => {
            Some(CastOp::IntegerToFloat)
        }
        (Some(TypeKind::Float { .. }), Some(TypeKind::Integer { .. })) => {
            Some(CastOp::FloatToInteger)
        }
        (Some(TypeKind::Float(source)), Some(TypeKind::Float(target))) => {
            let source = float_bits(*source);
            let target = float_bits(*target);
            Some(if source < target {
                CastOp::FloatExtend
            } else if source > target {
                CastOp::FloatTruncate
            } else {
                CastOp::Bitcast
            })
        }
        _ => None,
    }
}

const fn integer_bits(width: IntegerWidth, pointer_bits: u16) -> u16 {
    match width {
        IntegerWidth::Bits8 => 8,
        IntegerWidth::Bits16 => 16,
        IntegerWidth::Bits32 => 32,
        IntegerWidth::Bits64 => 64,
        IntegerWidth::Pointer => pointer_bits,
    }
}

const fn float_bits(width: FloatWidth) -> u16 {
    match width {
        FloatWidth::Bits16 => 16,
        FloatWidth::Bits32 => 32,
        FloatWidth::Bits64 => 64,
    }
}

fn lower_constant(text: &str, kind: Option<&TypeKind>) -> Result<Constant, String> {
    match kind {
        Some(TypeKind::Bool) => match text {
            "true" => Ok(Constant::Bool(true)),
            "false" => Ok(Constant::Bool(false)),
            _ => Err(format!("invalid Bool literal {text:?}")),
        },
        Some(TypeKind::Integer { .. }) => {
            parse_integer(text).map(|value| Constant::Integer { value })
        }
        Some(TypeKind::Char) => {
            let utf8 = decode_quoted(text, '\'')?;
            let decoded = std::str::from_utf8(&utf8)
                .map_err(|error| format!("invalid character UTF-8: {error}"))?;
            let mut characters = decoded.chars();
            let value = characters
                .next()
                .ok_or_else(|| "empty character literal".to_owned())?;
            if characters.next().is_some() {
                return Err("character literal contains more than one scalar".to_owned());
            }
            Ok(Constant::Integer {
                value: i128::from(u32::from(value)),
            })
        }
        Some(TypeKind::Float(width)) => parse_float_bits(text, *width),
        _ => Err(format!("unsupported literal type {kind:?}")),
    }
}

fn decode_quoted(text: &str, quote: char) -> Result<Vec<u8>, String> {
    let body = text
        .strip_prefix(quote)
        .and_then(|text| text.strip_suffix(quote))
        .ok_or_else(|| format!("literal is not enclosed by {quote:?}"))?;
    let mut decoded = String::new();
    let mut characters = body.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        let escaped = characters
            .next()
            .ok_or_else(|| "literal ends with an incomplete escape".to_owned())?;
        decoded.push(match escaped {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            '0' => '\0',
            '\\' => '\\',
            '"' => '"',
            '\'' => '\'',
            _ => return Err(format!("unsupported literal escape \\{escaped}")),
        });
    }
    Ok(decoded.into_bytes())
}

fn parse_integer(text: &str) -> Result<i128, String> {
    let text = strip_suffix(
        text,
        &[
            "isize", "usize", "i64", "u64", "i32", "u32", "i16", "u16", "i8", "u8",
        ],
    );
    let digits = text.replace('_', "");
    let (radix, digits) = if let Some(digits) = digits.strip_prefix("0x") {
        (16, digits)
    } else if let Some(digits) = digits.strip_prefix("0o") {
        (8, digits)
    } else if let Some(digits) = digits.strip_prefix("0b") {
        (2, digits)
    } else {
        (10, digits.as_str())
    };
    i128::from_str_radix(digits, radix).map_err(|error| format!("invalid integer literal: {error}"))
}

fn parse_float_bits(text: &str, width: FloatWidth) -> Result<Constant, String> {
    let text = strip_suffix(text, &["f64", "f32", "f16"]);
    let normalized = text.replace('_', "");
    match width {
        FloatWidth::Bits64 => normalized
            .parse::<f64>()
            .map(|value| Constant::FloatBits {
                bits: value.to_bits(),
            })
            .map_err(|error| format!("invalid Float64 literal: {error}")),
        FloatWidth::Bits32 => normalized
            .parse::<f32>()
            .map(|value| Constant::FloatBits {
                bits: u64::from(value.to_bits()),
            })
            .map_err(|error| format!("invalid Float32 literal: {error}")),
        FloatWidth::Bits16 => normalized
            .parse::<f32>()
            .map(|value| Constant::FloatBits {
                bits: u64::from(f32_to_f16_bits(value)),
            })
            .map_err(|error| format!("invalid Float16 literal: {error}")),
    }
}

fn f32_to_f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x7f_ffff;
    if exponent == 0xff {
        let payload = (mantissa >> 13) as u16;
        return sign | 0x7c00 | if payload == 0 { 0 } else { payload | 1 };
    }
    let mut half_exponent = exponent - 127 + 15;
    if half_exponent >= 31 {
        return sign | 0x7c00;
    }
    if half_exponent <= 0 {
        if half_exponent < -10 {
            return sign;
        }
        let mantissa = mantissa | 0x80_0000;
        let shift = (14 - half_exponent) as u32;
        let mut rounded = mantissa >> shift;
        let round_bit = 1_u32 << (shift - 1);
        if mantissa & round_bit != 0 && (mantissa & (round_bit - 1) != 0 || rounded & 1 != 0) {
            rounded += 1;
        }
        return sign | rounded as u16;
    }
    let mut rounded_mantissa = mantissa + 0x1000;
    if rounded_mantissa & 0x80_0000 != 0 {
        rounded_mantissa = 0;
        half_exponent += 1;
        if half_exponent >= 31 {
            return sign | 0x7c00;
        }
    }
    sign | ((half_exponent as u16) << 10) | ((rounded_mantissa >> 13) as u16)
}

fn strip_suffix<'a>(text: &'a str, suffixes: &[&str]) -> &'a str {
    suffixes
        .iter()
        .find_map(|suffix| text.strip_suffix(suffix))
        .unwrap_or(text)
}

const fn lower_unary(operator: Operator) -> Option<UnaryOp> {
    match operator {
        Operator::Minus => Some(UnaryOp::Negate),
        Operator::Bang => Some(UnaryOp::Not),
        Operator::Tilde => Some(UnaryOp::BitNot),
        _ => None,
    }
}

const fn lower_binary(operator: Operator) -> Option<BinaryOp> {
    match operator {
        Operator::Plus => Some(BinaryOp::Add),
        Operator::Minus => Some(BinaryOp::Subtract),
        Operator::Star => Some(BinaryOp::Multiply),
        Operator::Slash => Some(BinaryOp::Divide),
        Operator::Percent => Some(BinaryOp::Remainder),
        Operator::Ampersand | Operator::And => Some(BinaryOp::BitAnd),
        Operator::Pipe | Operator::Or => Some(BinaryOp::BitOr),
        Operator::Caret => Some(BinaryOp::BitXor),
        _ => None,
    }
}

const fn lower_compare(operator: Operator) -> Option<ComparePredicate> {
    match operator {
        Operator::Equal => Some(ComparePredicate::Equal),
        Operator::NotEqual => Some(ComparePredicate::NotEqual),
        Operator::Less => Some(ComparePredicate::Less),
        Operator::LessEqual => Some(ComparePredicate::LessEqual),
        Operator::Greater => Some(ComparePredicate::Greater),
        Operator::GreaterEqual => Some(ComparePredicate::GreaterEqual),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use jadren_hir::lower_hir;
    use jadren_lexer::lex;
    use jadren_mir::{
        CarrierPart, MirOperand, MirOperandKind, Place, Projection, Terminator, elaborate_drops,
        elaborate_region_cleanup, infer_lifetimes, lower_mir, materialize_returns,
    };
    use jadren_parser::parse;
    use jadren_resolve::resolve;
    use jadren_source::SourceManager;
    use jadren_typeck::check_types;
    use jadren_types::NominalLayoutKind;

    use super::{LowerOptions, lower_from_mir};

    #[test]
    fn converts_float16_literals_with_ieee_rounding() {
        assert_eq!(super::f32_to_f16_bits(1.5), 0x3e00);
        assert_eq!(super::f32_to_f16_bits(f32::INFINITY), 0x7c00);
        assert_eq!(super::f32_to_f16_bits(f32::NEG_INFINITY), 0xfc00);
        assert_eq!(super::f32_to_f16_bits(0.0), 0);
    }

    #[test]
    fn lowers_scalar_if_cfg_and_values_to_deterministic_jir() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "test.jdn",
                "module test; fn choose(flag: Bool) -> Int32 { return if flag { 1 + 2 } else { 3 } }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("scalar MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("branch %v"));
        assert!(text.contains(" = add "));
        assert!(text.contains("jump ^bb3()"));
        assert!(text.contains("return %v"));
        assert_eq!(jir.functions[0].blocks.len(), mir.functions[0].blocks.len());
    }

    #[test]
    fn lowers_internal_pointer_dereference_projection_to_load() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "pointer-dereference.jdn",
                "module test; fn deref_value(value: Pointer<Int32>) -> Int32 { return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        let function = mir.functions.first_mut().expect("pointer function");
        let pointer_local = function
            .locals
            .iter()
            .find(|local| local.is_parameter)
            .expect("pointer parameter")
            .id;
        let mut replaced = false;
        for block in &mut function.blocks {
            if let Terminator::Return {
                value: Some(value), ..
            } = &mut block.terminator
            {
                let ty = value.ty;
                let span = value.span;
                *value = MirOperand {
                    ty,
                    kind: MirOperandKind::Place(Place {
                        local: pointer_local,
                        projection: vec![Projection::Dereference],
                    }),
                    span,
                };
                replaced = true;
                break;
            }
        }
        assert!(replaced, "return operand must be replaced");
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("pointer dereference place must lower");
        let errors = crate::verify(&jir);
        assert!(errors.is_empty(), "{errors:?}");
        let text = jir.to_text();
        assert!(text.matches(" = load ").count() >= 2, "{text}");
        assert!(text.contains("ptr<generic,"), "{text}");
    }

    #[test]
    fn rejects_internal_pointer_dereference_projection_on_scalar_place() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "invalid-pointer-dereference.jdn",
                "module test; fn scalar(value: Int32) -> Int32 { return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        let function = mir.functions.first_mut().expect("scalar function");
        let scalar_local = function
            .locals
            .iter()
            .find(|local| local.is_parameter)
            .expect("scalar parameter")
            .id;
        for block in &mut function.blocks {
            if let Terminator::Return {
                value: Some(value), ..
            } = &mut block.terminator
            {
                let ty = value.ty;
                let span = value.span;
                *value = MirOperand {
                    ty,
                    kind: MirOperandKind::Place(Place {
                        local: scalar_local,
                        projection: vec![Projection::Dereference],
                    }),
                    span,
                };
                break;
            }
        }
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let errors = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect_err("scalar dereference must be rejected");
        assert!(errors.iter().any(|error| {
            error
                .message
                .contains("pointer dereference projection requires a Pointer<T> place")
        }));
    }

    #[test]
    fn lowers_pattern_extract_from_projected_record_place() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "pattern-projected-record.jdn",
                "module test; struct Pair { value: Int32 } fn project(pair: Pair) -> Int32 { return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        let function = mir
            .functions
            .iter_mut()
            .find(|function| function.name == "project")
            .expect("record function");
        let pair_local = function
            .locals
            .iter()
            .find(|local| local.is_parameter)
            .expect("record parameter")
            .id;
        let mut replaced = false;
        for block in &mut function.blocks {
            if let Terminator::Return {
                value: Some(value), ..
            } = &mut block.terminator
            {
                let ty = value.ty;
                let span = value.span;
                *value = MirOperand {
                    ty,
                    kind: MirOperandKind::PatternExtract {
                        source: Place {
                            local: pair_local,
                            projection: vec![Projection::Field("value".to_owned())],
                        },
                        source_indices: Vec::new(),
                        path: Vec::new(),
                        borrowed: false,
                    },
                    span,
                };
                replaced = true;
                break;
            }
        }
        assert!(replaced, "return operand must be replaced");
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("projected pattern source must lower");
        let errors = crate::verify(&jir);
        assert!(errors.is_empty(), "{errors:?}");
        let text = jir.to_text();
        assert!(text.contains(" = offset "), "{text}");
        assert!(text.matches(" = load ").count() >= 2, "{text}");
    }

    #[test]
    fn rejects_pattern_extract_from_indexed_source_without_index_operand() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "pattern-indexed-source.jdn",
                "module test; fn project(values: Buffer<Int32>) -> Int32 { return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        let function = mir
            .functions
            .iter_mut()
            .find(|function| function.name == "project")
            .expect("buffer function");
        let values_local = function
            .locals
            .iter()
            .find(|local| local.is_parameter)
            .expect("buffer parameter")
            .id;
        for block in &mut function.blocks {
            if let Terminator::Return {
                value: Some(value), ..
            } = &mut block.terminator
            {
                let ty = value.ty;
                let span = value.span;
                *value = MirOperand {
                    ty,
                    kind: MirOperandKind::PatternExtract {
                        source: Place {
                            local: values_local,
                            projection: vec![Projection::Index],
                        },
                        source_indices: Vec::new(),
                        path: Vec::new(),
                        borrowed: false,
                    },
                    span,
                };
                break;
            }
        }
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let errors = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect_err("indexed pattern source without an operand must be rejected");
        assert!(errors.iter().any(|error| {
            error.message.contains(
                "pattern/carrier source index projection requires an explicit index operand",
            )
        }));
    }

    #[test]
    fn lowers_pattern_extract_from_indexed_source_with_index_operand() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "pattern-indexed-source-valid.jdn",
                "module test; fn project(values: Buffer<Int32>) -> Int32 { return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        let function = mir
            .functions
            .iter_mut()
            .find(|function| function.name == "project")
            .expect("buffer function");
        let values_local = function
            .locals
            .iter()
            .find(|local| local.is_parameter)
            .expect("buffer parameter")
            .id;
        for block in &mut function.blocks {
            if let Terminator::Return {
                value: Some(value), ..
            } = &mut block.terminator
            {
                let ty = value.ty;
                let span = value.span;
                *value = MirOperand {
                    ty,
                    kind: MirOperandKind::PatternExtract {
                        source: Place {
                            local: values_local,
                            projection: vec![Projection::Index],
                        },
                        source_indices: vec![MirOperand {
                            ty,
                            kind: MirOperandKind::Literal(jadren_hir::HirLiteral {
                                kind: jadren_parser::LiteralKind::Integer,
                                text: "0".to_owned(),
                            }),
                            span,
                        }],
                        path: Vec::new(),
                        borrowed: false,
                    },
                    span,
                };
                break;
            }
        }
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("indexed pattern source must lower");
        let errors = crate::verify(&jir);
        assert!(errors.is_empty(), "{errors:?}");
        let text = jir.to_text();
        assert!(text.contains("bounds_check "), "{text}");
        assert!(text.contains(" = offset "), "{text}");
        assert!(text.matches(" = load ").count() >= 2, "{text}");
    }

    #[test]
    fn lowers_carrier_extract_from_indexed_source_with_index_operand() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "carrier-indexed-source-valid.jdn",
                "module test; fn project(values: Buffer<Option<Int32>>) -> Int32 { return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        let function = mir
            .functions
            .iter_mut()
            .find(|function| function.name == "project")
            .expect("carrier function");
        let values_local = function
            .locals
            .iter()
            .find(|local| local.is_parameter)
            .expect("carrier parameter")
            .id;
        for block in &mut function.blocks {
            if let Terminator::Return {
                value: Some(value), ..
            } = &mut block.terminator
            {
                let ty = value.ty;
                let span = value.span;
                *value = MirOperand {
                    ty,
                    kind: MirOperandKind::CarrierExtract {
                        source: Place {
                            local: values_local,
                            projection: vec![Projection::Index],
                        },
                        source_indices: vec![MirOperand {
                            ty,
                            kind: MirOperandKind::Literal(jadren_hir::HirLiteral {
                                kind: jadren_parser::LiteralKind::Integer,
                                text: "0".to_owned(),
                            }),
                            span,
                        }],
                        part: CarrierPart::Success,
                    },
                    span,
                };
                break;
            }
        }
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("indexed carrier source must lower");
        let errors = crate::verify(&jir);
        assert!(errors.is_empty(), "{errors:?}");
        let text = jir.to_text();
        assert!(text.contains("bounds_check "), "{text}");
        assert!(text.contains("enum_extract "), "{text}");
    }

    #[test]
    fn lowers_first_class_function_values_to_address_and_indirect_call() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "function-pointer.jdn",
                "fn increment(value: Int32) -> Int32 { return value + 1; }\
                 pub fn apply_increment(value: Int32) -> Int32 {\
                     let callback = increment; return callback(value);\
                 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("function value MIR must lower");
        let errors = crate::verify(&jir);
        assert!(errors.is_empty(), "{errors:?}");
        let text = jir.to_text();
        assert!(text.contains("function_address @f0"), "{text}");
        assert!(text.contains("indirect_call"), "{text}");
    }

    #[test]
    fn lowers_external_function_values_to_import_address_and_indirect_call() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "external-function-pointer.jdn",
                "extern \"C\" { fn external_adjust(value: Int32) -> Int32; }\
                 pub fn apply_external(value: Int32) -> Int32 {\
                     let callback = external_adjust; return callback(value);\
                 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("external function value MIR must lower");
        let errors = crate::verify(&jir);
        assert!(errors.is_empty(), "{errors:?}");
        let text = jir.to_text();
        assert!(text.contains("function_address @f"), "{text}");
        assert!(text.contains("indirect_call"), "{text}");
        assert!(text.contains("fn import"), "{text}");
    }

    #[test]
    fn lowers_nonterminated_assignment_in_if_block_to_store() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "assignment-in-if.jdn",
                "module test; fn update(flag: Bool) { var count: Int32 = 0; if flag { count = count + 1 } }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("assignment in a unit if block must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("branch %v"));
        assert!(text.contains(" = add "));
        assert!(text.contains("store "));
    }

    #[test]
    fn lowers_compound_assignment_in_loop_to_binary_store() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "compound-assignment.jdn",
                "module test; fn update() { var count: Int32 = 3; while count > 0 { count -= 1 } }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("compound assignment must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains(" = sub "));
        assert!(text.contains("store "));
        assert!(text.contains("jump ^bb"));
    }

    #[test]
    fn lowers_indexed_compound_assignment_with_one_captured_index() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "indexed-compound-assignment.jdn",
                "module test; struct Agent { position: Int32 } fn update(agents: write Slice<Agent>, index: UIntSize) { agents[index].position += 1 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("indexed compound assignment must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("bounds_check "));
        assert!(text.contains(" = add "));
        assert!(text.contains("store "));
    }

    #[test]
    fn lowers_local_and_deduplicated_external_calls() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "calls.jdn",
                "module test; fn twice(value: Int32) -> Int32 { return value + value } fn run() { assert_eq(twice(2), 4); assert_eq(twice(3), 6) }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("direct calls must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert_eq!(jir.functions.len(), 3);
        assert!(text.contains("call @f0("));
        assert_eq!(text.matches("fn import @f2 \"assert_eq\"").count(), 1);
        assert_eq!(text.matches("call @f2(").count(), 2);
    }

    #[test]
    fn lowers_checked_float4_slice_intrinsics_to_packed_jir() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "vector-slice.jdn",
                "module test; @noalloc fn probe(values: write Slice<Float32>, index: UIntSize, delta: Float32) { let current: Float4 = vector_load4(values, index); let amount: Float4 = vector_splat4(delta); vector_store4(values, index, current + amount) }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("vector slice MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("vector_bounds_check "));
        assert!(text.contains("vector_splat 4"));
        assert!(text.contains("vector.add "));
        assert!(text.contains("load %v"));
        assert!(text.contains("store %v"));
    }

    #[test]
    fn lowers_checked_float8_slice_intrinsics_to_packed_jir() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "vector-slice8.jdn",
                "module test; @noalloc fn probe(values: write Slice<Float32>, index: UIntSize, delta: Float32) { let current: Float8 = vector_load8(values, index); let amount: Float8 = vector_splat8(delta); vector_store8(values, index, current + amount) }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("vector slice MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("vector_bounds_check "));
        assert!(text.contains("vector_splat 8"));
        assert!(text.contains("vector.add "));
        assert!(text.contains("load %v"));
        assert!(text.contains("store %v"));
    }

    #[test]
    fn lowers_checked_float2_and_float3_slice_intrinsics_to_packed_jir() {
        for lanes in [2_u16, 3_u16] {
            let mut sources = SourceManager::new();
            let id = sources
                .add(
                    format!("vector-slice{lanes}.jdn"),
                    format!(
                        "module test; @noalloc fn probe(values: write Slice<Float32>, index: UIntSize, delta: Float32) {{ let current: Float{lanes} = vector_load{lanes}(values, index); let amount: Float{lanes} = vector_splat{lanes}(delta); vector_store{lanes}(values, index, current + amount) }}"
                    ),
                )
                .expect("source");
            let source = sources.get(id).expect("source");
            let lexed = lex(source);
            let parsed = parse(source, &lexed.tokens);
            let resolved = resolve(source, &parsed.file);
            let checked = check_types(source, &parsed.file, &resolved);
            assert!(
                !checked.has_errors(),
                "Float{lanes}: {:?}",
                checked.diagnostics
            );
            let hir = lower_hir(source, &parsed.file, &resolved, &checked);
            let mut mir = lower_mir(&hir.module, &checked.types);
            materialize_returns(&mut mir, &checked.types);
            elaborate_region_cleanup(&mut mir);
            elaborate_drops(&mut mir, &checked.types);

            let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
                .expect("vector slice MIR must lower");
            assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
            let text = jir.to_text();
            assert!(text.contains("vector_bounds_check "));
            assert!(text.contains(&format!("vector_splat {lanes}")));
            assert!(text.contains("vector.add "));
            assert!(text.contains("load %v"));
            assert!(text.contains("store %v"));
        }
    }

    #[test]
    fn lowers_arrays_indexed_assignment_records_and_strings() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "aggregates.jdn",
                "module test; struct Pair { z: Int32, a: Int32 } struct Box<T> { value: T } struct Node { next: Pointer<Node>, value: Int32 } enum Choice { First, Second(Int32) } fn array(index: Int32) -> Int32 { let values: [Int32; 3] = [1, 2, 3]; values[index] = 9; return values[index] } fn record() -> Int32 { let pair = Pair { a: 2, z: 1 }; return pair.a } fn record_value() -> Int32 { return (Pair { a: 2, z: 1 }).a } fn generic_record() -> Int32 { let boxed: Box<Int32> = Box { value: 7 }; return boxed.value } fn buffer(values: Buffer<Int32>, index: Int32) -> Int32 { return values[index] } fn buffer_set(values: Buffer<Int32>, index: Int32) { values[index] = 5 } fn slice(values: Slice<Int32>, index: Int32) -> Int32 { return values[index] } fn inspect(value: Choice) {} fn inspect_node(value: Node) {} fn text() { print(\"A\\n\") }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        assert_eq!(checked.nominal_layouts.len(), 4);
        assert!(checked.nominal_layouts.iter().any(|layout| matches!(
            &layout.kind,
            NominalLayoutKind::Record { fields }
                if fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>()
                    == ["z", "a"]
        )));
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("aggregate MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("array<3,"));
        assert!(text.contains("aggregate "));
        assert!(text.contains("bounds_check "));
        assert!(text.contains("extract_element "));
        assert!(text.contains("extract_value "));
        assert!(text.contains("string \"A\\n\""));
        assert!(text.contains("nominal_enum<"));
        assert!(text.contains("ptr<heap,"));
        assert!(text.contains("cast.int_extend "));
        assert!(jir.types.iter().enumerate().any(|(index, ty)| {
            let crate::Type::NominalStruct { fields, .. } = ty else {
                return false;
            };
            fields.iter().any(|field| {
                matches!(
                    jir.types.get(field.index()),
                    Some(crate::Type::Pointer { pointee, .. }) if pointee.index() == index
                )
            })
        }));
    }

    #[test]
    fn lowers_dynamic_for_iteration_length_and_indexing() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "dynamic_for.jdn",
                "module test; fn buffer(values: Buffer<Int32>) { for value in values { print(value) } } fn slice(values: Slice<Int32>) { for value in values { print(value) } }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("dynamic for MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("extract_value "));
        assert!(text.contains("bounds_check "));
        assert!(text.contains("extract_element ") || text.contains("load "));
    }

    #[test]
    fn lowers_slice_index_iteration_and_projected_field_updates() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "slice_indices.jdn",
                "module test; struct Agent { position: Int32, velocity: Int32 } fn update(agents: write Slice<Agent>) { for index in agents.indices { agents[index].position = agents[index].position + agents[index].velocity } }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("slice index iteration must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("bounds_check "), "{text}");
        assert!(text.contains("offset "), "{text}");
        assert!(text.contains("store "), "{text}");
    }

    #[test]
    fn lowers_numeric_casts_to_verified_jir_operations() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "casts.jdn",
                "module test; fn widen(value: Int32) -> Int64 { return value as Int64 } fn narrow(value: Int64) -> Int32 { return value as Int32 } fn real(value: Int32) -> Float64 { return value as Float64 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("numeric casts must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("cast.int_extend "), "{text}");
        assert!(text.contains("cast.int_truncate "), "{text}");
        assert!(text.contains("cast.int_to_float "), "{text}");
    }

    #[test]
    fn lowers_enum_match_and_option_result_propagation() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "carriers.jdn",
                "module test; enum Choice { First(Int32), Second(Int32) } enum Inner { Value(Int32) } enum Outer { Wrap(Inner) } fn choose(value: Choice) -> Int32 { return match value { Choice.First(item) if item > 0 => item, Choice.Second(item) => item, _ => 0 } } fn load(flag: Bool) -> Result<Int32, String> { return if flag { Ok(4) } else { Error(\"bad\") } } fn run(flag: Bool) -> Result<Int32, String> { let value = load(flag)?; return Ok(value + 1) } fn optional(value: Option<Int32>) -> Option<Int32> { let item = value?; return Some(item) } fn nested(value: Outer) -> Int32 { return match value { Outer.Wrap(Inner.Value(7)) => 1, _ => 0 } }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("enum and carrier MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert_eq!(jir.functions.len(), 5);
        assert!(text.contains("enum_construct 0("));
        assert!(text.contains("enum_construct 1("));
        assert!(text.contains("enum_tag "));
        assert!(text.contains("enum_extract "));
        assert!(text.contains("branch %v"));
        let nested_mir = mir
            .functions
            .iter()
            .find(|function| function.name == "nested")
            .expect("nested MIR function");
        let nested_jir = jir
            .functions
            .iter()
            .find(|function| function.name == "nested")
            .expect("nested JIR function");
        assert!(nested_jir.blocks.len() > nested_mir.blocks.len());
    }

    #[test]
    fn lowers_capabilities_regions_allocations_and_drops() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "memory.jdn",
                "module test; fn inspect(value: read Buffer<Int32>) {} fn borrow(data: Buffer<Int32>) { let view: read Buffer<Int32> = data; inspect(view) } fn read_at(values: read Buffer<Int32>, index: Int32) -> Int32 { return values[index] } fn release(data: Buffer<Int32>) {} fn region_run() { region frame { let values: Buffer<Int32> = frame.allocate(4); let view: read Buffer<Int32> = values; let first = view[0]; assert_eq(first, 0) } }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolved = resolve(source, &parsed.file);
        let checked = check_types(source, &parsed.file, &resolved);
        assert!(!checked.has_errors(), "{:?}", checked.diagnostics);
        let hir = lower_hir(source, &parsed.file, &resolved, &checked);
        let mut mir = lower_mir(&hir.module, &checked.types);
        materialize_returns(&mut mir, &checked.types);
        elaborate_region_cleanup(&mut mir);
        infer_lifetimes(&mut mir);
        elaborate_drops(&mut mir, &checked.types);

        let jir = lower_from_mir(&mir, &checked.types, LowerOptions::default())
            .expect("elaborated memory MIR must lower");
        assert!(crate::verify(&jir).is_empty(), "{:?}", crate::verify(&jir));
        let text = jir.to_text();
        assert!(text.contains("region_handle"));
        assert!(text.contains("region_create"));
        assert!(text.contains("region_alloc "));
        assert!(text.contains("region_destroy "));
        assert!(text.contains("ptr<region,"));
        assert!(text.contains("cast.pointer "));
        assert!(text.contains("drop %v"));
    }
}
