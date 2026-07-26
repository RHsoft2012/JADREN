//! Conservative pointer provenance and alias classification for JIR.

use std::collections::{BTreeMap, BTreeSet};

use crate::{FunctionId, InstructionKind, Module, Type, TypeId, ValueId};

/// Relationship between two pointer values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AliasRelation {
    /// The two values are the same pointer identity.
    MustAlias,
    /// The two values cannot refer to the same allocation in the current proof.
    NoAlias,
    /// The available provenance is insufficient to prove separation.
    MayAlias,
}

/// Per-module alias facts. Missing pairs intentionally default to `MayAlias`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AliasAnalysis {
    relations: BTreeMap<(FunctionId, ValueId, ValueId), AliasRelation>,
}

impl AliasAnalysis {
    /// Returns the conservative relation for two values in one function.
    #[must_use]
    pub fn relation(&self, function: FunctionId, left: ValueId, right: ValueId) -> AliasRelation {
        if left == right {
            return AliasRelation::MustAlias;
        }
        let (left, right) = ordered(left, right);
        self.relations
            .get(&(function, left, right))
            .copied()
            .unwrap_or(AliasRelation::MayAlias)
    }

    /// Number of materialized pointer-pair facts.
    #[must_use]
    pub fn pair_count(&self) -> usize {
        self.relations.len()
    }
}

/// Computes conservative pointer provenance for every defined pointer value.
///
/// Distinct `StackAlloc` roots are proven disjoint. Validated `AssumeNoAlias`
/// contract pairs propagate through aggregate copies, field extraction, casts,
/// and offsets. Offsets derived from one root retain that root but are
/// classified as `MayAlias` with the base because index arithmetic has not yet
/// been range-proven. Entry parameters are distinct from stack allocations
/// created by the current invocation. Loads, calls, and unknown casts remain
/// `MayAlias` with all other roots.
#[must_use]
pub fn analyze_aliases(module: &Module) -> AliasAnalysis {
    let mut analysis = AliasAnalysis::default();
    for function in &module.functions {
        let mut provenance = BTreeMap::<ValueId, Provenance>::new();
        let mut memory_provenance = BTreeMap::<ValueId, Provenance>::new();
        let mut contract_noalias = BTreeSet::<(ValueId, ValueId)>::new();
        let entry_parameters = function
            .parameters
            .iter()
            .map(|parameter| parameter.value)
            .collect::<BTreeSet<_>>();
        let mut pointers = BTreeMap::<ValueId, TypeId>::new();
        for parameter in &function.parameters {
            // Parameters exist before this invocation creates any `StackAlloc`
            // allocation. This remains true for aggregate parameters whose
            // pointer field is extracted after a lowering-generated stack copy.
            provenance.insert(parameter.value, Provenance::Parameter(parameter.value));
            if is_pointer_type(module, parameter.ty) {
                pointers.insert(parameter.value, parameter.ty);
            }
        }
        // The lowering prelude copies parameters into stack slots before it
        // emits `AssumeNoAlias`. Precollect all markers so those stores and
        // subsequent loads receive contract provenance during the main walk.
        for block in &function.blocks {
            for instruction in &block.instructions {
                if let InstructionKind::AssumeNoAlias { left, right } = &instruction.kind {
                    contract_noalias.insert(ordered(*left, *right));
                    provenance.insert(*left, contract_provenance(*left, &entry_parameters));
                    provenance.insert(*right, contract_provenance(*right, &entry_parameters));
                }
            }
        }
        for block in &function.blocks {
            for parameter in &block.parameters {
                provenance.insert(parameter.value, Provenance::Unknown);
                if is_pointer_type(module, parameter.ty) {
                    pointers.insert(parameter.value, parameter.ty);
                }
            }
            for instruction in &block.instructions {
                match &instruction.kind {
                    InstructionKind::AssumeNoAlias { left, right } => {
                        contract_noalias.insert(ordered(*left, *right));
                        provenance.insert(*left, contract_provenance(*left, &entry_parameters));
                        provenance.insert(*right, contract_provenance(*right, &entry_parameters));
                    }
                    InstructionKind::Store { pointer, value, .. } => {
                        if let Some(value_provenance) = provenance.get(value).copied() {
                            memory_provenance.insert(*pointer, value_provenance);
                        } else {
                            memory_provenance.remove(pointer);
                        }
                    }
                    _ => {}
                }
                let Some(result) = instruction.result else {
                    continue;
                };
                let value_provenance = match &instruction.kind {
                    InstructionKind::StackAlloc { .. } => Provenance::Unique(result.value),
                    InstructionKind::RegionAlloc { region, .. } => Provenance::Shared(*region),
                    InstructionKind::ExtractValue { aggregate, .. } => provenance
                        .get(aggregate)
                        .copied()
                        .unwrap_or(Provenance::Unknown),
                    InstructionKind::Load { pointer, .. } => memory_provenance
                        .get(pointer)
                        .copied()
                        .unwrap_or(Provenance::Unknown),
                    InstructionKind::Offset { base, .. } => provenance
                        .get(base)
                        .copied()
                        .map(Provenance::derived)
                        .unwrap_or(Provenance::Unknown),
                    InstructionKind::Cast { value, .. } => provenance
                        .get(value)
                        .copied()
                        .unwrap_or(Provenance::Unknown),
                    InstructionKind::Select {
                        when_true,
                        when_false,
                        ..
                    } => {
                        let left = provenance.get(when_true).copied();
                        let right = provenance.get(when_false).copied();
                        if left.is_some() && left == right {
                            left.unwrap_or(Provenance::Unknown)
                        } else {
                            Provenance::Unknown
                        }
                    }
                    _ => Provenance::Unknown,
                };
                provenance.insert(result.value, value_provenance);
                if is_pointer_type(module, result.ty) {
                    pointers.insert(result.value, result.ty);
                }
            }
        }

        let values: Vec<_> = pointers.keys().copied().collect();
        for (left_index, left) in values.iter().enumerate() {
            for right in values.iter().skip(left_index + 1) {
                let relation = alias_relation(
                    provenance.get(left).copied().unwrap_or(Provenance::Unknown),
                    provenance
                        .get(right)
                        .copied()
                        .unwrap_or(Provenance::Unknown),
                    &contract_noalias,
                );
                let (left, right) = ordered(*left, *right);
                analysis
                    .relations
                    .insert((function.id, left, right), relation);
            }
        }
    }
    analysis
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Provenance {
    Parameter(ValueId),
    ContractParameter(ValueId),
    Unique(ValueId),
    DerivedUnique(ValueId),
    Contract(ValueId),
    Shared(ValueId),
    Unknown,
}

impl Provenance {
    fn derived(self) -> Self {
        match self {
            Self::Parameter(root) => Self::Parameter(root),
            Self::ContractParameter(root) => Self::ContractParameter(root),
            Self::Unique(root) | Self::DerivedUnique(root) => Self::DerivedUnique(root),
            Self::Contract(root) => Self::Contract(root),
            Self::Shared(root) => Self::Shared(root),
            Self::Unknown => Self::Unknown,
        }
    }
}

fn contract_provenance(value: ValueId, entry_parameters: &BTreeSet<ValueId>) -> Provenance {
    if entry_parameters.contains(&value) {
        Provenance::ContractParameter(value)
    } else {
        Provenance::Contract(value)
    }
}

fn alias_relation(
    left: Provenance,
    right: Provenance,
    contract_noalias: &BTreeSet<(ValueId, ValueId)>,
) -> AliasRelation {
    match (left, right) {
        (Provenance::Parameter(_), Provenance::Unique(_))
        | (Provenance::Parameter(_), Provenance::DerivedUnique(_))
        | (Provenance::Unique(_), Provenance::Parameter(_))
        | (Provenance::DerivedUnique(_), Provenance::Parameter(_))
        | (Provenance::ContractParameter(_), Provenance::Unique(_))
        | (Provenance::ContractParameter(_), Provenance::DerivedUnique(_))
        | (Provenance::Unique(_), Provenance::ContractParameter(_))
        | (Provenance::DerivedUnique(_), Provenance::ContractParameter(_)) => {
            AliasRelation::NoAlias
        }
        (Provenance::Unique(left), Provenance::Unique(right))
        | (Provenance::Unique(left), Provenance::DerivedUnique(right))
        | (Provenance::DerivedUnique(left), Provenance::Unique(right))
        | (Provenance::DerivedUnique(left), Provenance::DerivedUnique(right))
            if left != right =>
        {
            AliasRelation::NoAlias
        }
        (
            Provenance::Contract(left) | Provenance::ContractParameter(left),
            Provenance::Contract(right) | Provenance::ContractParameter(right),
        ) if left != right && contract_noalias.contains(&ordered(left, right)) => {
            AliasRelation::NoAlias
        }
        _ => AliasRelation::MayAlias,
    }
}

fn ordered(left: ValueId, right: ValueId) -> (ValueId, ValueId) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

fn is_pointer_type(module: &Module, ty: TypeId) -> bool {
    matches!(module.types.get(ty.index()), Some(Type::Pointer { .. }))
}

#[cfg(test)]
mod tests {
    use super::{AliasRelation, analyze_aliases};
    use crate::{
        AddressSpace, Block, BlockId, Constant, Function, FunctionId, Instruction, InstructionKind,
        Linkage, Module, Terminator, Type, TypeId, TypedValue, ValueId,
    };

    fn value(index: usize, ty: usize) -> TypedValue {
        TypedValue {
            value: ValueId::new(index),
            ty: TypeId::new(ty),
        }
    }

    #[test]
    fn separates_distinct_stack_roots_and_keeps_offsets_conservative() {
        let mut module = Module {
            types: vec![
                Type::Integer {
                    signed: false,
                    bits: 32,
                },
                Type::Pointer {
                    pointee: TypeId::new(0),
                    address_space: AddressSpace::Stack,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "alias".to_owned(),
                linkage: Linkage::Internal,
                parameters: Vec::new(),
                result: TypeId::new(0),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        Instruction {
                            result: Some(value(0, 1)),
                            kind: InstructionKind::StackAlloc {
                                ty: TypeId::new(0),
                                count: None,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(1, 1)),
                            kind: InstructionKind::StackAlloc {
                                ty: TypeId::new(0),
                                count: None,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(2, 0)),
                            kind: InstructionKind::Constant(Constant::Integer { value: 0 }),
                            span: None,
                        },
                        Instruction {
                            result: Some(value(3, 1)),
                            kind: InstructionKind::Offset {
                                base: ValueId::new(0),
                                indices: vec![ValueId::new(2)],
                            },
                            span: None,
                        },
                    ],
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        };
        let analysis = analyze_aliases(&module);
        assert_eq!(analysis.pair_count(), 3);
        assert_eq!(
            analysis.relation(FunctionId::new(0), ValueId::new(0), ValueId::new(1)),
            AliasRelation::NoAlias
        );
        assert_eq!(
            analysis.relation(FunctionId::new(0), ValueId::new(0), ValueId::new(3)),
            AliasRelation::MayAlias
        );
        assert_eq!(
            analysis.relation(FunctionId::new(0), ValueId::new(0), ValueId::new(0)),
            AliasRelation::MustAlias
        );
        module.functions[0].parameters.push(crate::Parameter {
            value: ValueId::new(4),
            ty: TypeId::new(1),
            name: None,
        });
        let with_unknown = analyze_aliases(&module);
        assert_eq!(
            with_unknown.relation(FunctionId::new(0), ValueId::new(1), ValueId::new(4)),
            AliasRelation::NoAlias
        );
    }

    #[test]
    fn propagates_disjoint_contract_through_slice_pointer_extraction() {
        let mut module = Module {
            types: vec![
                Type::Float { bits: 32 },
                Type::Pointer {
                    pointee: TypeId::new(0),
                    address_space: AddressSpace::Generic,
                },
                Type::Integer {
                    signed: false,
                    bits: 64,
                },
                Type::Struct {
                    fields: vec![TypeId::new(1), TypeId::new(2)],
                },
                Type::Unit,
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "contract".to_owned(),
                linkage: Linkage::Internal,
                parameters: vec![
                    crate::Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(3),
                        name: Some("left".to_owned()),
                    },
                    crate::Parameter {
                        value: ValueId::new(1),
                        ty: TypeId::new(3),
                        name: Some("right".to_owned()),
                    },
                ],
                result: TypeId::new(4),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        Instruction {
                            result: None,
                            kind: InstructionKind::AssumeNoAlias {
                                left: ValueId::new(0),
                                right: ValueId::new(1),
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(2, 1)),
                            kind: InstructionKind::ExtractValue {
                                aggregate: ValueId::new(0),
                                index: 0,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(3, 1)),
                            kind: InstructionKind::ExtractValue {
                                aggregate: ValueId::new(1),
                                index: 0,
                            },
                            span: None,
                        },
                    ],
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        };
        let analysis = analyze_aliases(&module);
        assert_eq!(
            analysis.relation(FunctionId::new(0), ValueId::new(2), ValueId::new(3)),
            AliasRelation::NoAlias
        );
        module.functions[0].blocks[0].instructions[0].kind = InstructionKind::AssumeNoAlias {
            left: ValueId::new(0),
            right: ValueId::new(0),
        };
        let without_contract = analyze_aliases(&module);
        assert_eq!(
            without_contract.relation(FunctionId::new(0), ValueId::new(2), ValueId::new(3)),
            AliasRelation::MayAlias
        );
    }

    #[test]
    fn propagates_disjoint_contract_through_stack_copies() {
        let mut module = Module {
            types: vec![
                Type::Float { bits: 32 },
                Type::Pointer {
                    pointee: TypeId::new(0),
                    address_space: AddressSpace::Generic,
                },
                Type::Integer {
                    signed: false,
                    bits: 64,
                },
                Type::Struct {
                    fields: vec![TypeId::new(1), TypeId::new(2)],
                },
                Type::Unit,
                Type::Pointer {
                    pointee: TypeId::new(3),
                    address_space: AddressSpace::Stack,
                },
            ],
            functions: vec![Function {
                id: FunctionId::new(0),
                name: "stack_contract".to_owned(),
                linkage: Linkage::Internal,
                parameters: vec![
                    crate::Parameter {
                        value: ValueId::new(0),
                        ty: TypeId::new(3),
                        name: Some("left".to_owned()),
                    },
                    crate::Parameter {
                        value: ValueId::new(1),
                        ty: TypeId::new(3),
                        name: Some("right".to_owned()),
                    },
                ],
                result: TypeId::new(4),
                blocks: vec![Block {
                    id: BlockId::new(0),
                    parameters: Vec::new(),
                    instructions: vec![
                        Instruction {
                            result: Some(value(2, 5)),
                            kind: InstructionKind::StackAlloc {
                                ty: TypeId::new(3),
                                count: None,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(3, 5)),
                            kind: InstructionKind::StackAlloc {
                                ty: TypeId::new(3),
                                count: None,
                            },
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::Store {
                                pointer: ValueId::new(2),
                                value: ValueId::new(0),
                                alignment: 8,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: None,
                            kind: InstructionKind::Store {
                                pointer: ValueId::new(3),
                                value: ValueId::new(1),
                                alignment: 8,
                                volatile: false,
                            },
                            span: None,
                        },
                        // The real lowering emits the contract after these
                        // stores; precollection must still preserve it.
                        Instruction {
                            result: None,
                            kind: InstructionKind::AssumeNoAlias {
                                left: ValueId::new(0),
                                right: ValueId::new(1),
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(4, 3)),
                            kind: InstructionKind::Load {
                                pointer: ValueId::new(2),
                                alignment: 8,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(5, 3)),
                            kind: InstructionKind::Load {
                                pointer: ValueId::new(3),
                                alignment: 8,
                                volatile: false,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(6, 1)),
                            kind: InstructionKind::ExtractValue {
                                aggregate: ValueId::new(4),
                                index: 0,
                            },
                            span: None,
                        },
                        Instruction {
                            result: Some(value(7, 1)),
                            kind: InstructionKind::ExtractValue {
                                aggregate: ValueId::new(5),
                                index: 0,
                            },
                            span: None,
                        },
                    ],
                    terminator: Terminator::Return { value: None },
                    span: None,
                }],
                span: None,
            }],
        };
        let analysis = analyze_aliases(&module);
        assert_eq!(
            analysis.relation(FunctionId::new(0), ValueId::new(6), ValueId::new(7)),
            AliasRelation::NoAlias
        );
        assert_eq!(
            analysis.relation(FunctionId::new(0), ValueId::new(2), ValueId::new(6)),
            AliasRelation::NoAlias
        );
        module.functions[0].blocks[0].instructions[4].kind = InstructionKind::AssumeNoAlias {
            left: ValueId::new(0),
            right: ValueId::new(0),
        };
        let without_contract = analyze_aliases(&module);
        assert_eq!(
            without_contract.relation(FunctionId::new(0), ValueId::new(6), ValueId::new(7)),
            AliasRelation::MayAlias
        );
    }
}
