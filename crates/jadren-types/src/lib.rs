//! Canonical, interned core type representation for Jadren semantic phases.

use std::fmt;

use jadren_determinism::{DeterministicMap, Fingerprint, StableHasher};

/// Opaque canonical type identity inside one [`TypeStore`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TypeId(usize);

impl TypeId {
    /// Returns the deterministic zero-based interner index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Stable identity of a user-defined struct, component, enum, or type constructor.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NominalTypeId(Fingerprint);

impl NominalTypeId {
    /// Derives an identity directly from a canonical module-qualified path.
    #[must_use]
    pub fn from_path(canonical_path: &str) -> Self {
        let mut hasher = StableHasher::with_domain("jadren-nominal-type-v1");
        hasher.write_str(canonical_path);
        Self(hasher.finish())
    }

    /// Wraps an already stable symbol fingerprint.
    #[must_use]
    pub const fn from_symbol_fingerprint(fingerprint: Fingerprint) -> Self {
        Self(fingerprint)
    }

    /// Returns the stable fingerprint representation.
    #[must_use]
    pub const fn fingerprint(self) -> Fingerprint {
        self.0
    }
}

/// Backend-relevant declaration layout for one nominal type constructor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NominalLayout {
    /// Stable constructor identity used by [`TypeKind::Nominal`].
    pub constructor: NominalTypeId,
    /// Generic parameters substituted by concrete nominal type arguments.
    pub generic_parameters: Vec<GenericParameterId>,
    /// Source-level ABI representation contract.
    pub repr: AbiRepr,
    /// Record or enum declaration shape in source declaration order.
    pub kind: NominalLayoutKind,
}

/// ABI representation selected for a nominal declaration.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AbiRepr {
    /// Jadren-native representation; layout may be target-specific.
    Jadren,
    /// Stable C-compatible field/variant representation.
    C,
}

/// Nominal value representation before target-specific ABI layout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NominalLayoutKind {
    /// Named aggregate fields in declaration order.
    Record { fields: Vec<NominalFieldLayout> },
    /// Tagged alternatives in declaration order.
    Enum { variants: Vec<NominalVariantLayout> },
}

/// One named record/component field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NominalFieldLayout {
    pub name: String,
    pub ty: TypeId,
}

/// One named enum variant and its ordered payload fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NominalVariantLayout {
    pub name: String,
    pub fields: Vec<TypeId>,
}

/// Identity of a type inference variable inside one type-checking context.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TypeVariableId(usize);

impl TypeVariableId {
    /// Creates a deterministic context-local variable identity.
    #[must_use]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    /// Returns the zero-based context-local index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Stable generic parameter position owned by a nominal or function declaration.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GenericParameterId {
    /// Stable owning declaration identity.
    pub owner: Fingerprint,
    /// Zero-based declaration-order position.
    pub index: usize,
}

/// Compiler-known trait usable as a generic bound in language version 0.1.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum BuiltinTrait {
    /// Supports `+` with a value of the same type.
    Addable,
    /// Integer or floating-point scalar.
    Numeric,
    /// Signed or unsigned integer scalar.
    Integer,
    /// Floating-point scalar.
    Floating,
    /// Supports value equality.
    Equatable,
    /// Supports language-level relational ordering operations.
    Ordered,
}

impl BuiltinTrait {
    /// Ordered source spelling of every compiler-known trait.
    pub const ALL: [Self; 6] = [
        Self::Addable,
        Self::Numeric,
        Self::Integer,
        Self::Floating,
        Self::Equatable,
        Self::Ordered,
    ];

    /// Returns the canonical source name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Addable => "Addable",
            Self::Numeric => "Numeric",
            Self::Integer => "Integer",
            Self::Floating => "Floating",
            Self::Equatable => "Equatable",
            Self::Ordered => "Ordered",
        }
    }

    /// Resolves one unqualified core trait name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.name() == name)
    }

    /// Returns whether this bound guarantees another core bound.
    #[must_use]
    pub const fn implies(self, required: Self) -> bool {
        self as u8 == required as u8
            || matches!(
                (self, required),
                (Self::Integer | Self::Floating, Self::Numeric)
                    | (
                        Self::Integer | Self::Floating | Self::Numeric,
                        Self::Addable
                    )
                    | (
                        Self::Integer | Self::Floating | Self::Numeric,
                        Self::Equatable
                    )
                    | (
                        Self::Integer | Self::Floating | Self::Numeric,
                        Self::Ordered
                    )
            )
    }
}

/// Signed or unsigned integer family.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Signedness {
    /// Signed two's-complement integer.
    Signed,
    /// Unsigned integer.
    Unsigned,
}

/// Fixed or target-pointer-sized integer width.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum IntegerWidth {
    /// Eight bits.
    Bits8,
    /// Sixteen bits.
    Bits16,
    /// Thirty-two bits.
    Bits32,
    /// Sixty-four bits.
    Bits64,
    /// Width of the selected target pointer.
    Pointer,
}

/// Supported IEEE-style floating-point widths.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FloatWidth {
    /// Sixteen-bit floating point.
    Bits16,
    /// Thirty-two-bit floating point.
    Bits32,
    /// Sixty-four-bit floating point.
    Bits64,
}

/// Explicit ownership or access capability attached to a type.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Capability {
    /// Unique owning value.
    Owned,
    /// Shared read-only borrow.
    Read,
    /// Exclusive writable borrow.
    Write,
}

/// Canonical structural description interned by [`TypeStore`].
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TypeKind {
    /// Error recovery type that suppresses derivative diagnostics.
    Error,
    /// Boolean value.
    Bool,
    /// Unicode scalar value.
    Char,
    /// UTF-8 string value.
    String,
    /// Empty value returned by procedures.
    Unit,
    /// Uninhabited type of diverging expressions.
    Never,
    /// Integer scalar.
    Integer {
        /// Signedness family.
        signedness: Signedness,
        /// Storage width.
        width: IntegerWidth,
    },
    /// Floating-point scalar.
    Float(FloatWidth),
    /// Fixed-width target-neutral SIMD vector.
    Vector {
        /// Scalar element type.
        element: TypeId,
        /// Number of lanes.
        lanes: u16,
    },
    /// Fixed-size inline array.
    Array {
        /// Element type.
        element: TypeId,
        /// Compile-time element count.
        length: u64,
    },
    /// Owning growable contiguous storage.
    Buffer(TypeId),
    /// Checked non-owning contiguous view.
    Slice(TypeId),
    /// Raw pointer used only at explicit unsafe/FFI boundaries.
    Pointer(TypeId),
    /// Optional value without implicit null.
    Option(TypeId),
    /// Success or expected-error value.
    Result {
        /// Success payload.
        ok: TypeId,
        /// Error payload.
        error: TypeId,
    },
    /// User-defined or library nominal type application.
    Nominal {
        /// Stable constructor identity.
        constructor: NominalTypeId,
        /// Canonical generic arguments.
        arguments: Box<[TypeId]>,
    },
    /// Generic parameter before monomorphization.
    GenericParameter(GenericParameterId),
    /// Local variable awaiting JAD-405 unification.
    InferenceVariable(TypeVariableId),
    /// First-class semantic function signature.
    Function {
        /// Parameter types in declaration order.
        parameters: Box<[TypeId]>,
        /// Return type.
        result: TypeId,
    },
    /// Explicit ownership/access capability.
    Capability {
        /// Capability kind.
        capability: Capability,
        /// Wrapped value type.
        inner: TypeId,
    },
}

impl TypeKind {
    /// Returns whether this is any signed or unsigned integer scalar.
    #[must_use]
    pub const fn is_integer(&self) -> bool {
        matches!(self, Self::Integer { .. })
    }

    /// Returns whether this is an integer, floating-point scalar, or numeric vector.
    #[must_use]
    pub const fn is_numeric(&self) -> bool {
        matches!(
            self,
            Self::Integer { .. } | Self::Float(_) | Self::Vector { .. }
        )
    }

    /// Returns whether this is a fixed-width SIMD vector.
    #[must_use]
    pub const fn is_vector(&self) -> bool {
        matches!(self, Self::Vector { .. })
    }
}

/// Stable semantic discriminants shared by the built-in Option/Result
/// carriers and every typed IR lowering.
///
/// Both carriers use the same two-way convention: tag `0` is the residual
/// branch (`None` or `Error`) and tag `1` is the successful branch (`Some` or
/// `Ok`). Payload layout remains type- and target-dependent; only these
/// discriminants are part of the core semantic contract.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CarrierTag {
    /// `None` for Option or `Error` for Result.
    Residual = 0,
    /// `Some` for Option or `Ok` for Result.
    Success = 1,
}

impl CarrierTag {
    /// Converts a raw semantic discriminant without accepting invalid tags.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::Residual),
            1 => Some(Self::Success),
            _ => None,
        }
    }

    /// Returns the stable raw discriminant used by JIR and core helpers.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self as u32
    }

    /// Returns whether this tag denotes the success payload.
    #[must_use]
    pub const fn is_success(self) -> bool {
        matches!(self, Self::Success)
    }
}

/// Built-in carrier family using [`CarrierTag`] discriminants.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CarrierKind {
    /// Optional value with `None`/`Some` variants.
    Option,
    /// Expected value with `Error`/`Ok` variants.
    Result,
}

impl CarrierKind {
    /// Returns the residual (`None`/`Error`) tag.
    #[must_use]
    pub const fn residual_tag(self) -> CarrierTag {
        CarrierTag::Residual
    }

    /// Returns the success (`Some`/`Ok`) tag.
    #[must_use]
    pub const fn success_tag(self) -> CarrierTag {
        CarrierTag::Success
    }

    /// Classifies a structural built-in carrier type.
    #[must_use]
    pub fn from_type(kind: &TypeKind) -> Option<Self> {
        match kind {
            TypeKind::Option(_) => Some(Self::Option),
            TypeKind::Result { .. } => Some(Self::Result),
            _ => None,
        }
    }
}

/// Pre-interned core scalar and fixed-width vector identities.
#[derive(Clone, Copy, Debug)]
pub struct CoreTypes {
    /// Error recovery type.
    pub error: TypeId,
    /// Boolean.
    pub bool_: TypeId,
    /// Character.
    pub char_: TypeId,
    /// UTF-8 string.
    pub string: TypeId,
    /// Unit.
    pub unit: TypeId,
    /// Never.
    pub never: TypeId,
    /// Signed integer types.
    pub int8: TypeId,
    pub int16: TypeId,
    pub int32: TypeId,
    pub int64: TypeId,
    pub int_size: TypeId,
    /// Unsigned integer types.
    pub uint8: TypeId,
    pub uint16: TypeId,
    pub uint32: TypeId,
    pub uint64: TypeId,
    pub uint_size: TypeId,
    /// Floating-point types.
    pub float16: TypeId,
    pub float32: TypeId,
    pub float64: TypeId,
    /// Fixed-width two-lane f32 vector.
    pub float2: TypeId,
    /// Fixed-width three-lane f32 vector.
    pub float3: TypeId,
    /// Fixed-width four-lane f32 vector.
    pub float4: TypeId,
    /// Fixed-width eight-lane f32 vector.
    pub float8: TypeId,
}

/// Deterministic canonical type interner.
#[derive(Clone, Debug)]
pub struct TypeStore {
    kinds: Vec<TypeKind>,
    interned: DeterministicMap<TypeKind, TypeId>,
    core: CoreTypes,
}

impl TypeStore {
    /// Creates a store with every scalar core type pre-interned in stable order.
    #[must_use]
    pub fn new() -> Self {
        let mut store = Self {
            kinds: Vec::new(),
            interned: DeterministicMap::new(),
            core: CoreTypes {
                error: TypeId(0),
                bool_: TypeId(0),
                char_: TypeId(0),
                string: TypeId(0),
                unit: TypeId(0),
                never: TypeId(0),
                int8: TypeId(0),
                int16: TypeId(0),
                int32: TypeId(0),
                int64: TypeId(0),
                int_size: TypeId(0),
                uint8: TypeId(0),
                uint16: TypeId(0),
                uint32: TypeId(0),
                uint64: TypeId(0),
                uint_size: TypeId(0),
                float16: TypeId(0),
                float32: TypeId(0),
                float64: TypeId(0),
                float2: TypeId(0),
                float3: TypeId(0),
                float4: TypeId(0),
                float8: TypeId(0),
            },
        };
        let error = store.intern(TypeKind::Error);
        let bool_ = store.intern(TypeKind::Bool);
        let char_ = store.intern(TypeKind::Char);
        let string = store.intern(TypeKind::String);
        let unit = store.intern(TypeKind::Unit);
        let never = store.intern(TypeKind::Never);
        let int8 = store.integer(Signedness::Signed, IntegerWidth::Bits8);
        let int16 = store.integer(Signedness::Signed, IntegerWidth::Bits16);
        let int32 = store.integer(Signedness::Signed, IntegerWidth::Bits32);
        let int64 = store.integer(Signedness::Signed, IntegerWidth::Bits64);
        let int_size = store.integer(Signedness::Signed, IntegerWidth::Pointer);
        let uint8 = store.integer(Signedness::Unsigned, IntegerWidth::Bits8);
        let uint16 = store.integer(Signedness::Unsigned, IntegerWidth::Bits16);
        let uint32 = store.integer(Signedness::Unsigned, IntegerWidth::Bits32);
        let uint64 = store.integer(Signedness::Unsigned, IntegerWidth::Bits64);
        let uint_size = store.integer(Signedness::Unsigned, IntegerWidth::Pointer);
        let float16 = store.intern(TypeKind::Float(FloatWidth::Bits16));
        let float32 = store.intern(TypeKind::Float(FloatWidth::Bits32));
        let float64 = store.intern(TypeKind::Float(FloatWidth::Bits64));
        let float4 = store.intern(TypeKind::Vector {
            element: float32,
            lanes: 4,
        });
        let float8 = store.intern(TypeKind::Vector {
            element: float32,
            lanes: 8,
        });
        // Append newly promoted core vectors after the existing Float4/Float8
        // identities so canonical IDs used by the 0.1 compatibility fixtures
        // remain unchanged.
        let float2 = store.intern(TypeKind::Vector {
            element: float32,
            lanes: 2,
        });
        let float3 = store.intern(TypeKind::Vector {
            element: float32,
            lanes: 3,
        });
        store.core = CoreTypes {
            error,
            bool_,
            char_,
            string,
            unit,
            never,
            int8,
            int16,
            int32,
            int64,
            int_size,
            uint8,
            uint16,
            uint32,
            uint64,
            uint_size,
            float16,
            float32,
            float64,
            float2,
            float3,
            float4,
            float8,
        };
        store
    }

    /// Returns the pre-interned core scalar identities.
    #[must_use]
    pub const fn core(&self) -> CoreTypes {
        self.core
    }

    /// Returns the canonical description for an identity.
    #[must_use]
    pub fn kind(&self, id: TypeId) -> Option<&TypeKind> {
        self.kinds.get(id.index())
    }

    /// Interns a structural description, returning an existing identity when equal.
    pub fn intern(&mut self, kind: TypeKind) -> TypeId {
        if let Some(id) = self.interned.get(&kind) {
            return *id;
        }
        let id = TypeId(self.kinds.len());
        self.kinds.push(kind.clone());
        self.interned.insert(kind, id);
        id
    }

    /// Returns the number of unique canonical types.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.kinds.len()
    }

    /// Returns whether no types are interned.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    /// Interns an integer scalar.
    pub fn integer(&mut self, signedness: Signedness, width: IntegerWidth) -> TypeId {
        self.intern(TypeKind::Integer { signedness, width })
    }

    /// Resolves and applies a language core type name with exact generic arity.
    pub fn apply_builtin(
        &mut self,
        name: &str,
        arguments: &[TypeId],
    ) -> Result<TypeId, BuiltinTypeError> {
        let core = self.core;
        let scalar = match name {
            "Bool" => Some(core.bool_),
            "Char" => Some(core.char_),
            "String" => Some(core.string),
            "Status" => Some(core.int32),
            "Unit" => Some(core.unit),
            "Never" => Some(core.never),
            "Int8" => Some(core.int8),
            "Int16" => Some(core.int16),
            "Int32" => Some(core.int32),
            "Int64" => Some(core.int64),
            "IntSize" => Some(core.int_size),
            "UInt8" => Some(core.uint8),
            "UInt16" => Some(core.uint16),
            "UInt32" => Some(core.uint32),
            "UInt64" => Some(core.uint64),
            "UIntSize" => Some(core.uint_size),
            "Float16" => Some(core.float16),
            "Float32" => Some(core.float32),
            "Float64" => Some(core.float64),
            "Float2" => Some(core.float2),
            "Float3" => Some(core.float3),
            "Float4" => Some(core.float4),
            "Float8" => Some(core.float8),
            _ => None,
        };
        if let Some(id) = scalar {
            require_arity(name, arguments, 0)?;
            return Ok(id);
        }
        match name {
            "Buffer" => {
                require_arity(name, arguments, 1)?;
                Ok(self.intern(TypeKind::Buffer(arguments[0])))
            }
            "Slice" => {
                require_arity(name, arguments, 1)?;
                Ok(self.intern(TypeKind::Slice(arguments[0])))
            }
            "Pointer" => {
                require_arity(name, arguments, 1)?;
                Ok(self.intern(TypeKind::Pointer(arguments[0])))
            }
            "Option" => {
                require_arity(name, arguments, 1)?;
                Ok(self.intern(TypeKind::Option(arguments[0])))
            }
            "Result" => {
                require_arity(name, arguments, 2)?;
                Ok(self.intern(TypeKind::Result {
                    ok: arguments[0],
                    error: arguments[1],
                }))
            }
            _ => Err(BuiltinTypeError::Unknown(name.to_owned())),
        }
    }
}

impl Default for TypeStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Context-local inference-variable bindings and structural unification.
#[derive(Clone, Debug, Default)]
pub struct UnificationTable {
    bindings: Vec<Option<TypeId>>,
}

impl UnificationTable {
    /// Creates an empty unification table.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bindings: Vec::new(),
        }
    }

    /// Allocates and interns a fresh unconstrained inference variable.
    pub fn fresh(&mut self, store: &mut TypeStore) -> TypeId {
        let index = self.bindings.len();
        self.bindings.push(None);
        store.intern(TypeKind::InferenceVariable(TypeVariableId::new(index)))
    }

    /// Follows inference bindings without recursively rebuilding compound types.
    #[must_use]
    pub fn resolve_shallow(&self, store: &TypeStore, mut ty: TypeId) -> TypeId {
        while let Some(TypeKind::InferenceVariable(variable)) = store.kind(ty) {
            let Some(Some(bound)) = self.bindings.get(variable.index()) else {
                break;
            };
            if *bound == ty {
                break;
            }
            ty = *bound;
        }
        ty
    }

    /// Unifies two types structurally and returns their canonical representative.
    pub fn unify(
        &mut self,
        store: &mut TypeStore,
        left: TypeId,
        right: TypeId,
    ) -> Result<TypeId, UnifyError> {
        let left = self.resolve_shallow(store, left);
        let right = self.resolve_shallow(store, right);
        if left == right {
            return Ok(left);
        }
        let left_kind = store
            .kind(left)
            .cloned()
            .ok_or(UnifyError::InvalidType(left))?;
        let right_kind = store
            .kind(right)
            .cloned()
            .ok_or(UnifyError::InvalidType(right))?;
        match (left_kind, right_kind) {
            (TypeKind::Error, _) | (_, TypeKind::Error) => Ok(store.core().error),
            (TypeKind::Never, _) => Ok(right),
            (_, TypeKind::Never) => Ok(left),
            (TypeKind::InferenceVariable(variable), _) => {
                self.bind(store, variable, right)?;
                Ok(right)
            }
            (_, TypeKind::InferenceVariable(variable)) => {
                self.bind(store, variable, left)?;
                Ok(left)
            }
            (
                TypeKind::Array {
                    element: left_element,
                    length: left_length,
                },
                TypeKind::Array {
                    element: right_element,
                    length: right_length,
                },
            ) if left_length == right_length => {
                let element = self.unify(store, left_element, right_element)?;
                Ok(store.intern(TypeKind::Array {
                    element,
                    length: left_length,
                }))
            }
            (
                TypeKind::Vector {
                    element: left_element,
                    lanes: left_lanes,
                },
                TypeKind::Vector {
                    element: right_element,
                    lanes: right_lanes,
                },
            ) if left_lanes == right_lanes => {
                let element = self.unify(store, left_element, right_element)?;
                Ok(store.intern(TypeKind::Vector {
                    element,
                    lanes: left_lanes,
                }))
            }
            (TypeKind::Buffer(left), TypeKind::Buffer(right)) => {
                let element = self.unify(store, left, right)?;
                Ok(store.intern(TypeKind::Buffer(element)))
            }
            (TypeKind::Slice(left), TypeKind::Slice(right)) => {
                let element = self.unify(store, left, right)?;
                Ok(store.intern(TypeKind::Slice(element)))
            }
            (TypeKind::Pointer(left), TypeKind::Pointer(right)) => {
                let element = self.unify(store, left, right)?;
                Ok(store.intern(TypeKind::Pointer(element)))
            }
            (TypeKind::Option(left), TypeKind::Option(right)) => {
                let inner = self.unify(store, left, right)?;
                Ok(store.intern(TypeKind::Option(inner)))
            }
            (
                TypeKind::Result {
                    ok: left_ok,
                    error: left_error,
                },
                TypeKind::Result {
                    ok: right_ok,
                    error: right_error,
                },
            ) => {
                let ok = self.unify(store, left_ok, right_ok)?;
                let error = self.unify(store, left_error, right_error)?;
                Ok(store.intern(TypeKind::Result { ok, error }))
            }
            (
                TypeKind::Nominal {
                    constructor: left_constructor,
                    arguments: left_arguments,
                },
                TypeKind::Nominal {
                    constructor: right_constructor,
                    arguments: right_arguments,
                },
            ) if left_constructor == right_constructor
                && left_arguments.len() == right_arguments.len() =>
            {
                let arguments = self.unify_lists(store, &left_arguments, &right_arguments)?;
                Ok(store.intern(TypeKind::Nominal {
                    constructor: left_constructor,
                    arguments,
                }))
            }
            (
                TypeKind::Function {
                    parameters: left_parameters,
                    result: left_result,
                },
                TypeKind::Function {
                    parameters: right_parameters,
                    result: right_result,
                },
            ) if left_parameters.len() == right_parameters.len() => {
                let parameters = self.unify_lists(store, &left_parameters, &right_parameters)?;
                let result = self.unify(store, left_result, right_result)?;
                Ok(store.intern(TypeKind::Function { parameters, result }))
            }
            (
                TypeKind::Capability {
                    capability: left_capability,
                    inner: left_inner,
                },
                TypeKind::Capability {
                    capability: right_capability,
                    inner: right_inner,
                },
            ) if left_capability == right_capability => {
                let inner = self.unify(store, left_inner, right_inner)?;
                Ok(store.intern(TypeKind::Capability {
                    capability: left_capability,
                    inner,
                }))
            }
            _ => Err(UnifyError::Mismatch { left, right }),
        }
    }

    /// Recursively substitutes all currently bound variables and re-interns the result.
    pub fn resolve_deep(&self, store: &mut TypeStore, ty: TypeId) -> Result<TypeId, UnifyError> {
        let ty = self.resolve_shallow(store, ty);
        let kind = store.kind(ty).cloned().ok_or(UnifyError::InvalidType(ty))?;
        let resolved = match kind {
            TypeKind::Array { element, length } => TypeKind::Array {
                element: self.resolve_deep(store, element)?,
                length,
            },
            TypeKind::Vector { element, lanes } => TypeKind::Vector {
                element: self.resolve_deep(store, element)?,
                lanes,
            },
            TypeKind::Buffer(element) => TypeKind::Buffer(self.resolve_deep(store, element)?),
            TypeKind::Slice(element) => TypeKind::Slice(self.resolve_deep(store, element)?),
            TypeKind::Pointer(element) => TypeKind::Pointer(self.resolve_deep(store, element)?),
            TypeKind::Option(inner) => TypeKind::Option(self.resolve_deep(store, inner)?),
            TypeKind::Result { ok, error } => TypeKind::Result {
                ok: self.resolve_deep(store, ok)?,
                error: self.resolve_deep(store, error)?,
            },
            TypeKind::Nominal {
                constructor,
                arguments,
            } => TypeKind::Nominal {
                constructor,
                arguments: arguments
                    .iter()
                    .map(|argument| self.resolve_deep(store, *argument))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_boxed_slice(),
            },
            TypeKind::Function { parameters, result } => TypeKind::Function {
                parameters: parameters
                    .iter()
                    .map(|parameter| self.resolve_deep(store, *parameter))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_boxed_slice(),
                result: self.resolve_deep(store, result)?,
            },
            TypeKind::Capability { capability, inner } => TypeKind::Capability {
                capability,
                inner: self.resolve_deep(store, inner)?,
            },
            other => other,
        };
        Ok(store.intern(resolved))
    }

    fn unify_lists(
        &mut self,
        store: &mut TypeStore,
        left: &[TypeId],
        right: &[TypeId],
    ) -> Result<Box<[TypeId]>, UnifyError> {
        left.iter()
            .zip(right)
            .map(|(left, right)| self.unify(store, *left, *right))
            .collect::<Result<Vec<_>, _>>()
            .map(Vec::into_boxed_slice)
    }

    fn bind(
        &mut self,
        store: &TypeStore,
        variable: TypeVariableId,
        ty: TypeId,
    ) -> Result<(), UnifyError> {
        if self.occurs(store, variable, ty) {
            return Err(UnifyError::Occurs {
                variable,
                within: ty,
            });
        }
        let slot = self
            .bindings
            .get_mut(variable.index())
            .ok_or(UnifyError::UnknownVariable(variable))?;
        *slot = Some(ty);
        Ok(())
    }

    fn occurs(&self, store: &TypeStore, variable: TypeVariableId, ty: TypeId) -> bool {
        let ty = self.resolve_shallow(store, ty);
        match store.kind(ty) {
            Some(TypeKind::InferenceVariable(other)) => *other == variable,
            Some(TypeKind::Array { element, .. })
            | Some(TypeKind::Vector { element, .. })
            | Some(TypeKind::Buffer(element))
            | Some(TypeKind::Slice(element))
            | Some(TypeKind::Pointer(element))
            | Some(TypeKind::Option(element)) => self.occurs(store, variable, *element),
            Some(TypeKind::Result { ok, error }) => {
                self.occurs(store, variable, *ok) || self.occurs(store, variable, *error)
            }
            Some(TypeKind::Nominal { arguments, .. }) => arguments
                .iter()
                .any(|argument| self.occurs(store, variable, *argument)),
            Some(TypeKind::Function { parameters, result }) => {
                parameters
                    .iter()
                    .any(|parameter| self.occurs(store, variable, *parameter))
                    || self.occurs(store, variable, *result)
            }
            Some(TypeKind::Capability { inner, .. }) => self.occurs(store, variable, *inner),
            _ => false,
        }
    }
}

/// Structural unification failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnifyError {
    /// Two concrete structures cannot be equal.
    Mismatch { left: TypeId, right: TypeId },
    /// Binding would construct an infinite recursive type.
    Occurs {
        variable: TypeVariableId,
        within: TypeId,
    },
    /// A type identity does not belong to the supplied store.
    InvalidType(TypeId),
    /// A variable does not belong to this unification context.
    UnknownVariable(TypeVariableId),
}

/// Stable fingerprint of one fully concrete canonical type.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TypeFingerprint(Fingerprint);

impl TypeFingerprint {
    /// Returns the underlying stable fingerprint.
    #[must_use]
    pub const fn fingerprint(self) -> Fingerprint {
        self.0
    }
}

/// Stable identity of one concrete generic declaration instantiation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MonomorphizationKey(Fingerprint);

impl MonomorphizationKey {
    /// Builds a key from a stable declaration identity and ordered concrete arguments.
    #[must_use]
    pub fn new(declaration: Fingerprint, arguments: &[TypeFingerprint]) -> Self {
        let mut hasher = StableHasher::with_domain("jadren-monomorphization-v1");
        hasher.write_u64(declaration.as_u64());
        hasher.write_u64(arguments.len() as u64);
        for argument in arguments {
            hasher.write_u64(argument.fingerprint().as_u64());
        }
        Self(hasher.finish())
    }

    /// Returns the underlying stable fingerprint.
    #[must_use]
    pub const fn fingerprint(self) -> Fingerprint {
        self.0
    }
}

/// Ordered generic parameter replacement map.
#[derive(Clone, Debug, Default)]
pub struct Substitution {
    entries: DeterministicMap<GenericParameterId, TypeId>,
}

impl Substitution {
    /// Creates an empty substitution.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: DeterministicMap::new(),
        }
    }

    /// Adds or replaces one generic parameter mapping.
    pub fn insert(&mut self, parameter: GenericParameterId, ty: TypeId) -> Option<TypeId> {
        self.entries.insert(parameter, ty)
    }

    /// Returns one mapped type.
    #[must_use]
    pub fn get(&self, parameter: GenericParameterId) -> Option<TypeId> {
        self.entries.get(&parameter).copied()
    }

    /// Returns mappings in stable parameter order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (GenericParameterId, TypeId)> + '_ {
        self.entries.iter().map(|(parameter, ty)| (*parameter, *ty))
    }

    /// Recursively applies this substitution and interns the resulting structure.
    pub fn apply(&self, store: &mut TypeStore, ty: TypeId) -> Result<TypeId, TypeTransformError> {
        let kind = store
            .kind(ty)
            .cloned()
            .ok_or(TypeTransformError::InvalidType(ty))?;
        let replaced = match kind {
            TypeKind::GenericParameter(parameter) => return Ok(self.get(parameter).unwrap_or(ty)),
            TypeKind::Array { element, length } => TypeKind::Array {
                element: self.apply(store, element)?,
                length,
            },
            TypeKind::Vector { element, lanes } => TypeKind::Vector {
                element: self.apply(store, element)?,
                lanes,
            },
            TypeKind::Buffer(element) => TypeKind::Buffer(self.apply(store, element)?),
            TypeKind::Slice(element) => TypeKind::Slice(self.apply(store, element)?),
            TypeKind::Pointer(element) => TypeKind::Pointer(self.apply(store, element)?),
            TypeKind::Option(inner) => TypeKind::Option(self.apply(store, inner)?),
            TypeKind::Result { ok, error } => TypeKind::Result {
                ok: self.apply(store, ok)?,
                error: self.apply(store, error)?,
            },
            TypeKind::Nominal {
                constructor,
                arguments,
            } => TypeKind::Nominal {
                constructor,
                arguments: arguments
                    .iter()
                    .map(|argument| self.apply(store, *argument))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_boxed_slice(),
            },
            TypeKind::Function { parameters, result } => TypeKind::Function {
                parameters: parameters
                    .iter()
                    .map(|parameter| self.apply(store, *parameter))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_boxed_slice(),
                result: self.apply(store, result)?,
            },
            TypeKind::Capability { capability, inner } => TypeKind::Capability {
                capability,
                inner: self.apply(store, inner)?,
            },
            other => other,
        };
        Ok(store.intern(replaced))
    }
}

/// Type transformation/fingerprinting failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TypeTransformError {
    /// The type identity does not belong to the supplied store.
    InvalidType(TypeId),
    /// A concrete fingerprint was requested for an inference variable.
    InferenceVariable(TypeVariableId),
    /// A concrete fingerprint was requested for an unsubstituted generic parameter.
    GenericParameter(GenericParameterId),
}

impl TypeStore {
    /// Computes a stable structural fingerprint for a fully concrete type.
    pub fn stable_fingerprint(&self, ty: TypeId) -> Result<TypeFingerprint, TypeTransformError> {
        let mut hasher = StableHasher::with_domain("jadren-concrete-type-v1");
        self.hash_type(ty, &mut hasher)?;
        Ok(TypeFingerprint(hasher.finish()))
    }

    fn hash_type(&self, ty: TypeId, hasher: &mut StableHasher) -> Result<(), TypeTransformError> {
        let kind = self.kind(ty).ok_or(TypeTransformError::InvalidType(ty))?;
        match kind {
            TypeKind::Error => hasher.write_u64(0),
            TypeKind::Bool => hasher.write_u64(1),
            TypeKind::Char => hasher.write_u64(2),
            TypeKind::String => hasher.write_u64(3),
            TypeKind::Unit => hasher.write_u64(4),
            TypeKind::Never => hasher.write_u64(5),
            TypeKind::Integer { signedness, width } => {
                hasher.write_u64(6);
                hasher.write_u64(*signedness as u64);
                hasher.write_u64(*width as u64);
            }
            TypeKind::Float(width) => {
                hasher.write_u64(7);
                hasher.write_u64(*width as u64);
            }
            TypeKind::Vector { element, lanes } => {
                hasher.write_u64(17);
                hasher.write_u64(u64::from(*lanes));
                self.hash_type(*element, hasher)?;
            }
            TypeKind::Array { element, length } => {
                hasher.write_u64(8);
                hasher.write_u64(*length);
                self.hash_type(*element, hasher)?;
            }
            TypeKind::Buffer(element) => {
                hasher.write_u64(9);
                self.hash_type(*element, hasher)?;
            }
            TypeKind::Slice(element) => {
                hasher.write_u64(10);
                self.hash_type(*element, hasher)?;
            }
            TypeKind::Pointer(element) => {
                hasher.write_u64(11);
                self.hash_type(*element, hasher)?;
            }
            TypeKind::Option(inner) => {
                hasher.write_u64(12);
                self.hash_type(*inner, hasher)?;
            }
            TypeKind::Result { ok, error } => {
                hasher.write_u64(13);
                self.hash_type(*ok, hasher)?;
                self.hash_type(*error, hasher)?;
            }
            TypeKind::Nominal {
                constructor,
                arguments,
            } => {
                hasher.write_u64(14);
                hasher.write_u64(constructor.fingerprint().as_u64());
                hasher.write_u64(arguments.len() as u64);
                for argument in arguments {
                    self.hash_type(*argument, hasher)?;
                }
            }
            TypeKind::GenericParameter(parameter) => {
                return Err(TypeTransformError::GenericParameter(*parameter));
            }
            TypeKind::InferenceVariable(variable) => {
                return Err(TypeTransformError::InferenceVariable(*variable));
            }
            TypeKind::Function { parameters, result } => {
                hasher.write_u64(15);
                hasher.write_u64(parameters.len() as u64);
                for parameter in parameters {
                    self.hash_type(*parameter, hasher)?;
                }
                self.hash_type(*result, hasher)?;
            }
            TypeKind::Capability { capability, inner } => {
                hasher.write_u64(16);
                hasher.write_u64(*capability as u64);
                self.hash_type(*inner, hasher)?;
            }
        }
        Ok(())
    }
}

/// Invalid use of a core type constructor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BuiltinTypeError {
    /// The name is not a language core type.
    Unknown(String),
    /// Generic argument count does not match the constructor.
    Arity {
        /// Constructor name.
        name: String,
        /// Required argument count.
        expected: usize,
        /// Supplied argument count.
        actual: usize,
    },
}

impl fmt::Display for BuiltinTypeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(name) => write!(formatter, "unknown core type `{name}`"),
            Self::Arity {
                name,
                expected,
                actual,
            } => write!(
                formatter,
                "core type `{name}` expects {expected} type arguments but received {actual}"
            ),
        }
    }
}

impl std::error::Error for BuiltinTypeError {}

fn require_arity(
    name: &str,
    arguments: &[TypeId],
    expected: usize,
) -> Result<(), BuiltinTypeError> {
    if arguments.len() == expected {
        Ok(())
    } else {
        Err(BuiltinTypeError::Arity {
            name: name.to_owned(),
            expected,
            actual: arguments.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BuiltinTrait, BuiltinTypeError, Capability, CarrierKind, CarrierTag, FloatWidth,
        GenericParameterId, MonomorphizationKey, NominalTypeId, Substitution, TypeKind, TypeStore,
        TypeTransformError, TypeVariableId, UnificationTable, UnifyError,
    };

    #[test]
    fn preinterns_stable_core_types_and_classifies_numeric_kinds() {
        let store = TypeStore::new();
        let core = store.core();
        assert_eq!(core.error.index(), 0);
        assert_eq!(store.kind(core.bool_), Some(&TypeKind::Bool));
        assert!(store.kind(core.int32).is_some_and(TypeKind::is_integer));
        assert!(store.kind(core.float32).is_some_and(TypeKind::is_numeric));
        assert!(matches!(
            store.kind(core.float2),
            Some(TypeKind::Vector { lanes: 2, .. })
        ));
        assert!(matches!(
            store.kind(core.float3),
            Some(TypeKind::Vector { lanes: 3, .. })
        ));
        assert!(matches!(
            store.kind(core.float8),
            Some(TypeKind::Vector { lanes: 8, .. })
        ));
        assert!(!store.is_empty());
    }

    #[test]
    fn structurally_equal_types_share_one_identity() {
        let mut store = TypeStore::new();
        let core = store.core();
        let first = store.intern(TypeKind::Array {
            element: core.float32,
            length: 4,
        });
        let second = store.intern(TypeKind::Array {
            element: core.float32,
            length: 4,
        });
        let different = store.intern(TypeKind::Array {
            element: core.float32,
            length: 8,
        });
        assert_eq!(first, second);
        assert_ne!(first, different);
    }

    #[test]
    fn applies_builtin_constructors_with_exact_arity() {
        let mut store = TypeStore::new();
        let core = store.core();
        let slice = store
            .apply_builtin("Slice", &[core.float32])
            .expect("valid Slice");
        assert_eq!(store.kind(slice), Some(&TypeKind::Slice(core.float32)));
        assert_eq!(
            store.apply_builtin("Result", &[core.int32]),
            Err(BuiltinTypeError::Arity {
                name: "Result".to_owned(),
                expected: 2,
                actual: 1,
            })
        );
        assert_eq!(
            store.apply_builtin("Missing", &[]),
            Err(BuiltinTypeError::Unknown("Missing".to_owned()))
        );
    }

    #[test]
    fn keeps_option_result_carrier_tags_stable_and_rejects_invalid_values() {
        assert_eq!(CarrierTag::Residual.raw(), 0);
        assert_eq!(CarrierTag::Success.raw(), 1);
        assert_eq!(CarrierTag::from_raw(0), Some(CarrierTag::Residual));
        assert_eq!(CarrierTag::from_raw(1), Some(CarrierTag::Success));
        assert_eq!(CarrierTag::from_raw(2), None);
        assert!(!CarrierTag::Residual.is_success());
        assert!(CarrierTag::Success.is_success());

        let mut store = TypeStore::new();
        let core = store.core();
        let option = store.intern(TypeKind::Option(core.int32));
        let result = store.intern(TypeKind::Result {
            ok: core.int32,
            error: core.string,
        });
        assert_eq!(
            CarrierKind::from_type(store.kind(option).expect("Option type")),
            Some(CarrierKind::Option)
        );
        assert_eq!(
            CarrierKind::from_type(store.kind(result).expect("Result type")),
            Some(CarrierKind::Result)
        );
        assert_eq!(CarrierKind::Option.residual_tag(), CarrierTag::Residual);
        assert_eq!(CarrierKind::Result.success_tag(), CarrierTag::Success);
        assert_eq!(CarrierKind::from_type(&TypeKind::Bool), None);
    }

    #[test]
    fn core_trait_names_roundtrip_and_implications_are_explicit() {
        for bound in BuiltinTrait::ALL {
            assert_eq!(BuiltinTrait::from_name(bound.name()), Some(bound));
            assert!(bound.implies(bound));
        }
        assert!(BuiltinTrait::Integer.implies(BuiltinTrait::Numeric));
        assert!(BuiltinTrait::Numeric.implies(BuiltinTrait::Addable));
        assert!(!BuiltinTrait::Equatable.implies(BuiltinTrait::Numeric));
        assert_eq!(BuiltinTrait::from_name("Missing"), None);
    }

    #[test]
    fn represents_nominal_function_capability_and_inference_types() {
        let mut store = TypeStore::new();
        let core = store.core();
        let vector = store.intern(TypeKind::Nominal {
            constructor: NominalTypeId::from_path("math.Vector"),
            arguments: vec![core.float32].into_boxed_slice(),
        });
        let read_vector = store.intern(TypeKind::Capability {
            capability: Capability::Read,
            inner: vector,
        });
        let function = store.intern(TypeKind::Function {
            parameters: vec![read_vector].into_boxed_slice(),
            result: core.float32,
        });
        let variable = store.intern(TypeKind::InferenceVariable(TypeVariableId::new(0)));

        assert!(matches!(
            store.kind(function),
            Some(TypeKind::Function { .. })
        ));
        assert!(matches!(
            store.kind(variable),
            Some(TypeKind::InferenceVariable(_))
        ));
        assert_eq!(
            store.kind(core.float64),
            Some(&TypeKind::Float(FloatWidth::Bits64))
        );
    }

    #[test]
    fn unifies_variables_through_nested_structural_types() {
        let mut store = TypeStore::new();
        let core = store.core();
        let mut table = UnificationTable::new();
        let variable = table.fresh(&mut store);
        let inferred = store.intern(TypeKind::Slice(variable));
        let concrete = store.intern(TypeKind::Slice(core.float32));

        table
            .unify(&mut store, inferred, concrete)
            .expect("compatible slices");
        assert_eq!(
            table
                .resolve_deep(&mut store, inferred)
                .expect("valid resolution"),
            concrete
        );
    }

    #[test]
    fn rejects_mismatches_and_infinite_inference_types() {
        let mut store = TypeStore::new();
        let core = store.core();
        let mut table = UnificationTable::new();
        assert!(matches!(
            table.unify(&mut store, core.bool_, core.int32),
            Err(UnifyError::Mismatch { .. })
        ));

        let variable = table.fresh(&mut store);
        let recursive = store.intern(TypeKind::Option(variable));
        assert!(matches!(
            table.unify(&mut store, variable, recursive),
            Err(UnifyError::Occurs { .. })
        ));
    }

    #[test]
    fn recursively_substitutes_generic_parameters() {
        let mut store = TypeStore::new();
        let core = store.core();
        let parameter = GenericParameterId {
            owner: NominalTypeId::from_path("test.identity").fingerprint(),
            index: 0,
        };
        let generic = store.intern(TypeKind::GenericParameter(parameter));
        let slice = store.intern(TypeKind::Slice(generic));
        let mut substitution = Substitution::new();
        substitution.insert(parameter, core.int32);

        let concrete = substitution
            .apply(&mut store, slice)
            .expect("valid substitution");
        assert_eq!(store.kind(concrete), Some(&TypeKind::Slice(core.int32)));
    }

    #[test]
    fn concrete_fingerprints_build_stable_monomorphization_keys() {
        let mut first = TypeStore::new();
        let first_core = first.core();
        let first_slice = first.intern(TypeKind::Slice(first_core.int32));
        let first_fingerprint = first
            .stable_fingerprint(first_slice)
            .expect("concrete type");

        let mut second = TypeStore::new();
        let second_core = second.core();
        let second_slice = second.intern(TypeKind::Slice(second_core.int32));
        let second_fingerprint = second
            .stable_fingerprint(second_slice)
            .expect("concrete type");
        assert_eq!(first_fingerprint, second_fingerprint);

        let declaration = NominalTypeId::from_path("test.identity").fingerprint();
        assert_eq!(
            MonomorphizationKey::new(declaration, &[first_fingerprint]),
            MonomorphizationKey::new(declaration, &[second_fingerprint])
        );

        let variable = first.intern(TypeKind::InferenceVariable(TypeVariableId::new(0)));
        assert!(matches!(
            first.stable_fingerprint(variable),
            Err(TypeTransformError::InferenceVariable(_))
        ));
    }
}
