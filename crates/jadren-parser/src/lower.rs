use jadren_diagnostics::Diagnostic;
use jadren_lexer::{Keyword, Operator, Punctuation, Token, TokenKind};
use jadren_source::{SourceFile, Span};
use jadren_syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxTree};

use crate::{
    Annotation, AnnotationArgument, AstFile, Block, EnumDeclaration, EnumVariant, EnumVariantField,
    Expression, ExternBlock, ExternFunction, Field, Function, GenericParameter, Item, LiteralKind,
    MatchArm, Name, Parameter, Path, Pattern, RecordDeclaration, Statement, StructFieldValue,
    TypeCapability, TypeRef,
};

/// Result of lowering a lossless syntax tree into the semantic AST shape.
#[derive(Clone, Debug)]
pub struct AstLoweringOutput {
    /// Lowered AST, partial when an internal syntax invariant is missing.
    pub file: AstFile,
    /// Internal lowering invariant diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// Lowers a lossless syntax tree without reparsing the original token stream.
#[must_use]
pub fn lower_syntax_tree(source: &SourceFile, syntax: &SyntaxTree) -> AstLoweringOutput {
    let mut lowerer = Lowerer {
        source,
        diagnostics: Vec::new(),
    };
    let file = lowerer.lower_file(syntax.root());
    AstLoweringOutput {
        file,
        diagnostics: lowerer.diagnostics,
    }
}

struct Lowerer<'source> {
    source: &'source SourceFile,
    diagnostics: Vec<Diagnostic>,
}

impl Lowerer<'_> {
    fn lower_file(&mut self, root: &SyntaxNode) -> AstFile {
        let mut file = AstFile::default();
        for node in root.child_nodes() {
            match node.kind() {
                SyntaxKind::ModuleDeclaration => file.module = Some(self.lower_path_node(node)),
                SyntaxKind::ImportDeclaration => file.imports.push(self.lower_path_node(node)),
                SyntaxKind::FunctionDeclaration => {
                    file.items.push(Item::Function(self.lower_function(node)));
                }
                SyntaxKind::ExternBlock => {
                    file.items
                        .push(Item::ExternBlock(self.lower_extern_block(node)));
                }
                SyntaxKind::StructDeclaration => {
                    file.items
                        .push(Item::Struct(self.lower_record(node, false)));
                }
                SyntaxKind::ComponentDeclaration => {
                    file.items
                        .push(Item::Component(self.lower_record(node, true)));
                }
                SyntaxKind::EnumDeclaration => {
                    file.items.push(Item::Enum(self.lower_enum(node)));
                }
                _ => {}
            }
        }
        file
    }

    fn lower_function(&mut self, node: &SyntaxNode) -> Function {
        let annotations = node
            .child_nodes()
            .filter(|child| child.kind() == SyntaxKind::Annotation)
            .map(|child| self.lower_annotation(child))
            .collect();
        let generic_parameters = node
            .child_nodes()
            .find(|child| child.kind() == SyntaxKind::GenericParameterList)
            .map_or_else(Vec::new, |child| self.lower_generic_parameters(child));
        let parameters = node
            .child_nodes()
            .find(|child| child.kind() == SyntaxKind::ParameterList)
            .map_or_else(Vec::new, |child| self.lower_parameters(child));
        let return_type = node
            .child_nodes()
            .find(|child| is_type_kind(child.kind()))
            .map(|child| self.lower_type(child));
        let body_node = node
            .child_nodes()
            .find(|child| child.kind() == SyntaxKind::Block);
        let body = match body_node {
            Some(child) => self.lower_block(child),
            None => self.empty_block(node.span()),
        };

        Function {
            annotations,
            is_public: has_direct_keyword(node, Keyword::Pub),
            name: self.required_direct_name(node, "function name"),
            generic_parameters,
            parameters,
            return_type,
            body,
            span: node.span(),
        }
    }

    fn lower_extern_block(&mut self, node: &SyntaxNode) -> ExternBlock {
        let direct = significant_direct_tokens(node);
        let abi = direct
            .iter()
            .find(|token| token.kind == TokenKind::StringLiteral)
            .and_then(|token| self.source.slice(token.span))
            .unwrap_or_default()
            .trim_matches('"')
            .to_owned();
        let functions = node
            .child_nodes()
            .filter(|child| child.kind() == SyntaxKind::ExternFunctionDeclaration)
            .map(|child| self.lower_extern_function(child))
            .collect();
        ExternBlock {
            abi,
            functions,
            span: node.span(),
        }
    }

    fn lower_extern_function(&mut self, node: &SyntaxNode) -> ExternFunction {
        let parameters = node
            .child_nodes()
            .find(|child| child.kind() == SyntaxKind::ParameterList)
            .map_or_else(Vec::new, |child| self.lower_parameters(child));
        let return_type = node
            .child_nodes()
            .find(|child| is_type_kind(child.kind()))
            .map(|child| self.lower_type(child));
        ExternFunction {
            is_unsafe: has_direct_keyword(node, Keyword::Unsafe),
            name: self.required_direct_name(node, "extern function name"),
            parameters,
            return_type,
            span: node.span(),
        }
    }

    fn lower_record(&mut self, node: &SyntaxNode, component: bool) -> RecordDeclaration {
        let annotations = node
            .child_nodes()
            .filter(|child| child.kind() == SyntaxKind::Annotation)
            .map(|child| self.lower_annotation(child))
            .collect();
        let generic_parameters = node
            .child_nodes()
            .find(|child| child.kind() == SyntaxKind::GenericParameterList)
            .map_or_else(Vec::new, |child| self.lower_generic_parameters(child));
        let fields = node
            .child_nodes()
            .filter(|child| child.kind() == SyntaxKind::FieldDeclaration)
            .map(|child| self.lower_field(child))
            .collect();
        let expected = if component {
            "component name"
        } else {
            "struct name"
        };

        RecordDeclaration {
            annotations,
            is_public: has_direct_keyword(node, Keyword::Pub),
            name: self.required_direct_name(node, expected),
            generic_parameters,
            fields,
            span: node.span(),
        }
    }

    fn lower_enum(&mut self, node: &SyntaxNode) -> EnumDeclaration {
        let annotations = node
            .child_nodes()
            .filter(|child| child.kind() == SyntaxKind::Annotation)
            .map(|child| self.lower_annotation(child))
            .collect();
        let generic_parameters = node
            .child_nodes()
            .find(|child| child.kind() == SyntaxKind::GenericParameterList)
            .map_or_else(Vec::new, |child| self.lower_generic_parameters(child));
        let variants = node
            .child_nodes()
            .filter(|child| child.kind() == SyntaxKind::EnumVariant)
            .map(|child| self.lower_enum_variant(child))
            .collect();

        EnumDeclaration {
            annotations,
            is_public: has_direct_keyword(node, Keyword::Pub),
            name: self.required_direct_name(node, "enum name"),
            generic_parameters,
            variants,
            span: node.span(),
        }
    }

    fn lower_annotation(&mut self, node: &SyntaxNode) -> Annotation {
        let direct = significant_direct_tokens(node);
        let path_end = direct
            .iter()
            .find(|token| token.kind == TokenKind::Punctuation(Punctuation::LeftParen))
            .map_or(node.span().end, |token| token.span.start);
        let name = self.path_from_direct_tokens(node, path_end);
        let mut region_start = direct
            .iter()
            .find(|token| token.kind == TokenKind::Punctuation(Punctuation::LeftParen))
            .map_or(path_end, |token| token.span.end);
        let mut arguments = Vec::new();

        for child in node
            .child_nodes()
            .filter(|child| is_expression_kind(child.kind()))
        {
            let prefix: Vec<_> = direct
                .iter()
                .copied()
                .filter(|token| {
                    token.span.start >= region_start && token.span.end <= child.span().start
                })
                .collect();
            let argument_name = prefix.windows(2).find_map(|pair| {
                (pair[0].kind == TokenKind::Identifier
                    && pair[1].kind == TokenKind::Punctuation(Punctuation::Colon))
                .then(|| self.name_from_token(pair[0]))
            });
            let start = argument_name
                .as_ref()
                .map_or(child.span().start, |name| name.span.start);
            arguments.push(AnnotationArgument {
                name: argument_name,
                value: self.lower_expression(child),
                span: self.span(start, child.span().end),
            });
            region_start = child.span().end;
        }

        Annotation {
            name,
            arguments,
            span: node.span(),
        }
    }

    fn lower_generic_parameters(&mut self, node: &SyntaxNode) -> Vec<GenericParameter> {
        node.child_nodes()
            .filter(|child| child.kind() == SyntaxKind::GenericParameter)
            .map(|child| GenericParameter {
                name: self.required_direct_name(child, "generic parameter name"),
                bounds: child
                    .child_nodes()
                    .filter(|nested| is_type_kind(nested.kind()))
                    .map(|nested| self.lower_type(nested))
                    .collect(),
                span: child.span(),
            })
            .collect()
    }

    fn lower_parameters(&mut self, node: &SyntaxNode) -> Vec<Parameter> {
        node.child_nodes()
            .filter(|child| child.kind() == SyntaxKind::Parameter)
            .map(|child| {
                let type_node = child
                    .child_nodes()
                    .find(|nested| is_type_kind(nested.kind()));
                let ty = match type_node {
                    Some(nested) => self.lower_type(nested),
                    None => self.error_type(child),
                };
                Parameter {
                    name: self.required_direct_name(child, "parameter name"),
                    ty,
                    span: child.span(),
                }
            })
            .collect()
    }

    fn lower_field(&mut self, node: &SyntaxNode) -> Field {
        let type_node = node.child_nodes().find(|child| is_type_kind(child.kind()));
        let ty = match type_node {
            Some(child) => self.lower_type(child),
            None => self.error_type(node),
        };
        Field {
            is_public: has_direct_keyword(node, Keyword::Pub),
            name: self.required_direct_name(node, "field name"),
            ty,
            span: node.span(),
        }
    }

    fn lower_enum_variant(&mut self, node: &SyntaxNode) -> EnumVariant {
        let mut fields = Vec::new();
        for child in node.child_nodes() {
            match child.kind() {
                SyntaxKind::EnumVariantField => fields.push(self.lower_enum_variant_field(child)),
                kind if is_type_kind(kind) => fields.push(EnumVariantField {
                    name: None,
                    ty: self.lower_type(child),
                    span: child.span(),
                }),
                kind => self.invariant(
                    child.span(),
                    format!("unexpected enum payload node {kind:?}"),
                ),
            }
        }
        EnumVariant {
            name: self.required_direct_name(node, "enum variant name"),
            fields,
            span: node.span(),
        }
    }

    fn lower_enum_variant_field(&mut self, node: &SyntaxNode) -> EnumVariantField {
        let type_node = node.child_nodes().find(|child| is_type_kind(child.kind()));
        let ty = match type_node {
            Some(child) => self.lower_type(child),
            None => self.error_type(node),
        };
        EnumVariantField {
            name: Some(self.required_direct_name(node, "enum payload field name")),
            ty,
            span: node.span(),
        }
    }

    fn lower_type(&mut self, node: &SyntaxNode) -> TypeRef {
        match node.kind() {
            SyntaxKind::PathType => TypeRef::Path {
                path: self.path_from_direct_tokens(node, node.span().end),
                arguments: node
                    .child_nodes()
                    .filter(|child| is_type_kind(child.kind()))
                    .map(|child| self.lower_type(child))
                    .collect(),
                span: node.span(),
            },
            SyntaxKind::ArrayType => {
                let element_node = node.child_nodes().find(|child| is_type_kind(child.kind()));
                let element = match element_node {
                    Some(child) => self.lower_type(child),
                    None => self.error_type(node),
                };
                let direct = significant_direct_tokens(node);
                let semicolon = direct
                    .iter()
                    .position(|token| token.kind == TokenKind::Punctuation(Punctuation::Semicolon));
                let right = direct.iter().position(|token| {
                    token.kind == TokenKind::Punctuation(Punctuation::RightBracket)
                });
                let length = match (semicolon, right) {
                    (Some(semicolon), Some(right)) if semicolon < right => {
                        let values = &direct[semicolon + 1..right];
                        values.first().map_or_else(
                            || self.span(direct[right].span.start, direct[right].span.start),
                            |first| {
                                self.span(
                                    first.span.start,
                                    values.last().map_or(first.span.end, |last| last.span.end),
                                )
                            },
                        )
                    }
                    _ => self.span(node.span().end, node.span().end),
                };
                TypeRef::Array {
                    element: Box::new(element),
                    length,
                    span: node.span(),
                }
            }
            SyntaxKind::CapabilityType => {
                let capability = if has_direct_keyword(node, Keyword::Owned) {
                    TypeCapability::Owned
                } else if has_direct_keyword(node, Keyword::Write) {
                    TypeCapability::Write
                } else {
                    TypeCapability::Read
                };
                let inner = match node.child_nodes().find(|child| is_type_kind(child.kind())) {
                    Some(child) => self.lower_type(child),
                    None => self.error_type(node),
                };
                TypeRef::Capability {
                    capability,
                    inner: Box::new(inner),
                    span: node.span(),
                }
            }
            SyntaxKind::FunctionType => {
                let direct = significant_direct_tokens(node);
                let arrow_start = direct
                    .iter()
                    .find(|token| token.kind == TokenKind::Operator(Operator::Arrow))
                    .map(|token| token.span.start);
                let nested: Vec<_> = node
                    .child_nodes()
                    .filter(|child| is_type_kind(child.kind()))
                    .collect();
                let split = arrow_start.map_or(nested.len(), |start| {
                    nested
                        .iter()
                        .position(|child| child.span().start >= start)
                        .unwrap_or(nested.len())
                });
                let parameters = nested[..split]
                    .iter()
                    .map(|child| self.lower_type(child))
                    .collect();
                let return_type = nested
                    .get(split)
                    .map(|child| Box::new(self.lower_type(child)));
                TypeRef::Function {
                    parameters,
                    return_type,
                    span: node.span(),
                }
            }
            kind => {
                self.invariant(node.span(), format!("expected type node, found {kind:?}"));
                self.error_type(node)
            }
        }
    }

    fn lower_block(&mut self, node: &SyntaxNode) -> Block {
        let mut statements = Vec::new();
        for (index, element) in node.children().iter().enumerate() {
            let SyntaxElement::Node(child) = element else {
                continue;
            };
            match child.kind() {
                SyntaxKind::BindingStatement => statements.push(self.lower_binding(child)),
                SyntaxKind::ReturnStatement => statements.push(self.lower_return(child)),
                SyntaxKind::RegionStatement => statements.push(self.lower_region(child)),
                SyntaxKind::WhileStatement => statements.push(self.lower_while(child)),
                SyntaxKind::ForStatement => statements.push(self.lower_for(child)),
                SyntaxKind::BreakStatement => {
                    statements.push(Statement::Break { span: child.span() })
                }
                SyntaxKind::ContinueStatement => {
                    statements.push(Statement::Continue { span: child.span() })
                }
                kind if is_expression_kind(kind) => {
                    let terminated = node.children()[index + 1..]
                        .iter()
                        .take_while(|next| matches!(next, SyntaxElement::Token(_)))
                        .any(|next| {
                            matches!(
                                next,
                                SyntaxElement::Token(token)
                                    if token.kind == TokenKind::Punctuation(Punctuation::Semicolon)
                            )
                        });
                    statements.push(Statement::Expression {
                        expression: self.lower_expression(child),
                        terminated,
                    });
                }
                kind => self.invariant(child.span(), format!("unexpected block node {kind:?}")),
            }
        }
        Block {
            statements,
            span: node.span(),
        }
    }

    fn lower_binding(&mut self, node: &SyntaxNode) -> Statement {
        Statement::Binding {
            mutable: has_direct_keyword(node, Keyword::Var),
            name: self.required_direct_name(node, "binding name"),
            ty: node
                .child_nodes()
                .find(|child| is_type_kind(child.kind()))
                .map(|child| self.lower_type(child)),
            value: node
                .child_nodes()
                .find(|child| is_expression_kind(child.kind()))
                .map(|child| self.lower_expression(child)),
            span: node.span(),
        }
    }

    fn lower_return(&mut self, node: &SyntaxNode) -> Statement {
        Statement::Return {
            value: node
                .child_nodes()
                .find(|child| is_expression_kind(child.kind()))
                .map(|child| self.lower_expression(child)),
            span: node.span(),
        }
    }

    fn lower_region(&mut self, node: &SyntaxNode) -> Statement {
        let body = match node
            .child_nodes()
            .find(|child| child.kind() == SyntaxKind::Block)
        {
            Some(child) => self.lower_block(child),
            None => self.empty_block(node.span()),
        };
        Statement::Region {
            name: self.required_direct_name(node, "region name"),
            body,
            span: node.span(),
        }
    }

    fn lower_while(&mut self, node: &SyntaxNode) -> Statement {
        let condition = match node
            .child_nodes()
            .find(|child| is_expression_kind(child.kind()))
        {
            Some(child) => self.lower_expression(child),
            None => self.error_expression(node),
        };
        let body = match node
            .child_nodes()
            .find(|child| child.kind() == SyntaxKind::Block)
        {
            Some(child) => self.lower_block(child),
            None => self.empty_block(node.span()),
        };
        Statement::While {
            condition,
            body,
            span: node.span(),
        }
    }

    fn lower_for(&mut self, node: &SyntaxNode) -> Statement {
        let binding = self.required_direct_name(node, "for binding name");
        let iterable = match node
            .child_nodes()
            .find(|child| is_expression_kind(child.kind()))
        {
            Some(child) => self.lower_expression(child),
            None => self.error_expression(node),
        };
        let body = match node
            .child_nodes()
            .find(|child| child.kind() == SyntaxKind::Block)
        {
            Some(child) => self.lower_block(child),
            None => self.empty_block(node.span()),
        };
        Statement::For {
            binding,
            iterable,
            body,
            span: node.span(),
        }
    }

    fn lower_expression(&mut self, node: &SyntaxNode) -> Expression {
        match node.kind() {
            SyntaxKind::NameExpression => {
                Expression::Name(self.required_direct_name(node, "name expression"))
            }
            SyntaxKind::LiteralExpression => {
                let kind = significant_direct_tokens(node)
                    .first()
                    .map_or(LiteralKind::Integer, |token| literal_kind(token.kind));
                Expression::Literal {
                    kind,
                    span: node.span(),
                }
            }
            SyntaxKind::UnaryExpression => {
                let operand = self.required_expression_child(node, 0);
                Expression::Unary {
                    operator: self.required_direct_operator(node),
                    operand: Box::new(operand),
                    span: node.span(),
                }
            }
            SyntaxKind::BinaryExpression => Expression::Binary {
                left: Box::new(self.required_expression_child(node, 0)),
                operator: self.required_direct_operator(node),
                right: Box::new(self.required_expression_child(node, 1)),
                span: node.span(),
            },
            SyntaxKind::CallExpression => {
                let mut children = expression_children(node).into_iter();
                let callee = match children.next() {
                    Some(child) => self.lower_expression(child),
                    None => self.error_expression(node),
                };
                Expression::Call {
                    callee: Box::new(callee),
                    arguments: children.map(|child| self.lower_expression(child)).collect(),
                    span: node.span(),
                }
            }
            SyntaxKind::FieldExpression => Expression::Field {
                base: Box::new(self.required_expression_child(node, 0)),
                field: self.required_direct_name(node, "field access name"),
                span: node.span(),
            },
            SyntaxKind::IndexExpression => Expression::Index {
                base: Box::new(self.required_expression_child(node, 0)),
                index: Box::new(self.required_expression_child(node, 1)),
                span: node.span(),
            },
            SyntaxKind::TryExpression => Expression::Try {
                operand: Box::new(self.required_expression_child(node, 0)),
                span: node.span(),
            },
            SyntaxKind::CastExpression => {
                let expression = match node
                    .child_nodes()
                    .find(|child| is_expression_kind(child.kind()))
                {
                    Some(child) => self.lower_expression(child),
                    None => self.error_expression(node),
                };
                let target = match node.child_nodes().find(|child| is_type_kind(child.kind())) {
                    Some(child) => self.lower_type(child),
                    None => self.error_type(node),
                };
                Expression::Cast {
                    expression: Box::new(expression),
                    target,
                    span: node.span(),
                }
            }
            SyntaxKind::ArrayExpression => Expression::Array {
                elements: expression_children(node)
                    .into_iter()
                    .map(|child| self.lower_expression(child))
                    .collect(),
                span: node.span(),
            },
            SyntaxKind::StructExpression => self.lower_struct_expression(node),
            SyntaxKind::GroupExpression => Expression::Group {
                expression: Box::new(self.required_expression_child(node, 0)),
                span: node.span(),
            },
            SyntaxKind::BlockExpression => Expression::Block(self.lower_block(node)),
            SyntaxKind::IfExpression => self.lower_if_expression(node),
            SyntaxKind::MatchExpression => self.lower_match_expression(node),
            SyntaxKind::Error => Expression::Error(node.span()),
            kind => {
                self.invariant(
                    node.span(),
                    format!("expected expression node, found {kind:?}"),
                );
                Expression::Error(node.span())
            }
        }
    }

    fn lower_struct_expression(&mut self, node: &SyntaxNode) -> Expression {
        let type_node = node
            .child_nodes()
            .find(|child| is_expression_kind(child.kind()));
        let ty = match type_node {
            Some(child) => self.lower_expression(child),
            None => self.error_expression(node),
        };
        let fields = node
            .child_nodes()
            .filter(|child| child.kind() == SyntaxKind::StructFieldInitializer)
            .map(|child| StructFieldValue {
                name: self.required_direct_name(child, "struct initializer field name"),
                value: self.required_expression_child(child, 0),
                span: child.span(),
            })
            .collect();
        Expression::StructLiteral {
            ty: Box::new(ty),
            fields,
            span: node.span(),
        }
    }

    fn lower_if_expression(&mut self, node: &SyntaxNode) -> Expression {
        let condition_node = node
            .child_nodes()
            .find(|child| is_expression_kind(child.kind()));
        let condition = match condition_node {
            Some(child) => self.lower_expression(child),
            None => self.error_expression(node),
        };
        let then_node = node
            .child_nodes()
            .find(|child| child.kind() == SyntaxKind::Block);
        let then_block = match then_node {
            Some(child) => self.lower_block(child),
            None => self.empty_block(node.span()),
        };
        let else_branch = node
            .child_nodes()
            .find(|child| {
                matches!(
                    child.kind(),
                    SyntaxKind::IfExpression | SyntaxKind::BlockExpression
                )
            })
            .map(|child| Box::new(self.lower_expression(child)));
        Expression::If {
            condition: Box::new(condition),
            then_block,
            else_branch,
            span: node.span(),
        }
    }

    fn lower_match_expression(&mut self, node: &SyntaxNode) -> Expression {
        let value_node = node
            .child_nodes()
            .find(|child| is_expression_kind(child.kind()));
        let value = match value_node {
            Some(child) => self.lower_expression(child),
            None => self.error_expression(node),
        };
        let arms = node
            .child_nodes()
            .filter(|child| child.kind() == SyntaxKind::MatchArm)
            .map(|child| self.lower_match_arm(child))
            .collect();
        Expression::Match {
            value: Box::new(value),
            arms,
            span: node.span(),
        }
    }

    fn lower_match_arm(&mut self, node: &SyntaxNode) -> MatchArm {
        let children: Vec<_> = node.child_nodes().collect();
        let pattern_index = children
            .iter()
            .position(|child| is_pattern_kind(child.kind()))
            .unwrap_or(0);
        let pattern = match children.get(pattern_index) {
            Some(child) => self.lower_pattern(child),
            None => self.error_pattern(node),
        };
        let expressions: Vec<_> = children
            .iter()
            .enumerate()
            .filter(|(index, child)| *index != pattern_index && is_expression_kind(child.kind()))
            .map(|(_, child)| *child)
            .collect();
        let has_guard = has_direct_keyword(node, Keyword::If);
        let (guard, value_index) = if has_guard {
            (
                expressions
                    .first()
                    .map(|child| self.lower_expression(child)),
                1,
            )
        } else {
            (None, 0)
        };
        let value = match expressions.get(value_index) {
            Some(child) => self.lower_expression(child),
            None => self.error_expression(node),
        };
        MatchArm {
            pattern,
            guard,
            value,
            span: node.span(),
        }
    }

    fn lower_pattern(&mut self, node: &SyntaxNode) -> Pattern {
        match node.kind() {
            SyntaxKind::WildcardPattern => Pattern::Wildcard(node.span()),
            SyntaxKind::PathPattern => Pattern::Path(self.lower_path_node(node)),
            SyntaxKind::ConstructorPattern => Pattern::Constructor {
                path: self.lower_path_node(node),
                arguments: node
                    .child_nodes()
                    .filter(|child| is_pattern_kind(child.kind()))
                    .map(|child| self.lower_pattern(child))
                    .collect(),
                span: node.span(),
            },
            SyntaxKind::LiteralPattern => Pattern::Literal {
                kind: significant_direct_tokens(node)
                    .first()
                    .map_or(LiteralKind::Integer, |token| literal_kind(token.kind)),
                span: node.span(),
            },
            SyntaxKind::Error => Pattern::Error(node.span()),
            kind => {
                self.invariant(
                    node.span(),
                    format!("expected pattern node, found {kind:?}"),
                );
                Pattern::Error(node.span())
            }
        }
    }

    fn lower_path_node(&mut self, node: &SyntaxNode) -> Path {
        let direct = significant_direct_tokens(node);
        let end = direct
            .iter()
            .find(|token| {
                matches!(
                    token.kind,
                    TokenKind::Punctuation(Punctuation::LeftParen)
                        | TokenKind::Operator(Operator::Less)
                )
            })
            .map_or(node.span().end, |token| token.span.start);
        self.path_from_direct_tokens(node, end)
    }

    fn path_from_direct_tokens(&mut self, node: &SyntaxNode, end: usize) -> Path {
        let segments: Vec<_> = significant_direct_tokens(node)
            .into_iter()
            .filter(|token| token.kind == TokenKind::Identifier && token.span.end <= end)
            .map(|token| self.name_from_token(token))
            .collect();
        let span = segments.first().map_or(node.span(), |first| {
            self.span(
                first.span.start,
                segments.last().map_or(first.span.end, |last| last.span.end),
            )
        });
        Path { segments, span }
    }

    fn required_expression_child(&mut self, node: &SyntaxNode, index: usize) -> Expression {
        match expression_children(node).get(index) {
            Some(child) => self.lower_expression(child),
            None => self.error_expression(node),
        }
    }

    fn required_direct_operator(&mut self, node: &SyntaxNode) -> Operator {
        significant_direct_tokens(node)
            .into_iter()
            .find_map(|token| match token.kind {
                TokenKind::Operator(operator) => Some(operator),
                _ => None,
            })
            .unwrap_or_else(|| {
                self.invariant(node.span(), "expression without operator token");
                Operator::Plus
            })
    }

    fn required_direct_name(&mut self, node: &SyntaxNode, context: &str) -> Name {
        let token = significant_direct_tokens(node)
            .into_iter()
            .find(|token| token.kind == TokenKind::Identifier);
        match token {
            Some(token) => self.name_from_token(token),
            None => Name {
                text: format!("<error:{context}>"),
                span: node.span(),
            },
        }
    }

    fn name_from_token(&self, token: &Token) -> Name {
        Name {
            text: self.source.slice(token.span).unwrap_or_default().to_owned(),
            span: token.span,
        }
    }

    fn error_expression(&mut self, node: &SyntaxNode) -> Expression {
        Expression::Error(node.span())
    }

    fn error_pattern(&mut self, node: &SyntaxNode) -> Pattern {
        Pattern::Error(node.span())
    }

    fn error_type(&mut self, node: &SyntaxNode) -> TypeRef {
        TypeRef::Path {
            path: Path {
                segments: Vec::new(),
                span: node.span(),
            },
            arguments: Vec::new(),
            span: node.span(),
        }
    }

    fn empty_block(&mut self, span: Span) -> Block {
        Block {
            statements: Vec::new(),
            span,
        }
    }

    fn invariant(&mut self, span: Span, message: impl Into<String>) {
        let message = message.into();
        self.diagnostics.push(
            Diagnostic::error(
                "J0197",
                "syntax-to-AST lowering invariant failed",
                span,
                message.clone(),
            )
            .with_note(message),
        );
    }

    fn span(&self, start: usize, end: usize) -> Span {
        Span::new(self.source.id(), start.min(end), start.max(end))
            .unwrap_or_else(|| Span::empty(self.source.id(), start))
    }
}

fn significant_direct_tokens(node: &SyntaxNode) -> Vec<&Token> {
    node.child_tokens()
        .filter(|token| !token.kind.is_trivia() && token.kind != TokenKind::Eof)
        .collect()
}

fn has_direct_keyword(node: &SyntaxNode, keyword: Keyword) -> bool {
    node.child_tokens()
        .any(|token| token.kind == TokenKind::Keyword(keyword))
}

fn expression_children(node: &SyntaxNode) -> Vec<&SyntaxNode> {
    node.child_nodes()
        .filter(|child| is_expression_kind(child.kind()))
        .collect()
}

const fn is_type_kind(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::PathType
            | SyntaxKind::ArrayType
            | SyntaxKind::CapabilityType
            | SyntaxKind::FunctionType
    )
}

const fn is_expression_kind(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::NameExpression
            | SyntaxKind::LiteralExpression
            | SyntaxKind::UnaryExpression
            | SyntaxKind::BinaryExpression
            | SyntaxKind::CallExpression
            | SyntaxKind::FieldExpression
            | SyntaxKind::IndexExpression
            | SyntaxKind::TryExpression
            | SyntaxKind::CastExpression
            | SyntaxKind::ArrayExpression
            | SyntaxKind::StructExpression
            | SyntaxKind::GroupExpression
            | SyntaxKind::BlockExpression
            | SyntaxKind::IfExpression
            | SyntaxKind::MatchExpression
            | SyntaxKind::Error
    )
}

const fn is_pattern_kind(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::WildcardPattern
            | SyntaxKind::PathPattern
            | SyntaxKind::ConstructorPattern
            | SyntaxKind::LiteralPattern
            | SyntaxKind::Error
    )
}

const fn literal_kind(kind: TokenKind) -> LiteralKind {
    match kind {
        TokenKind::IntegerLiteral => LiteralKind::Integer,
        TokenKind::FloatLiteral => LiteralKind::Float,
        TokenKind::StringLiteral => LiteralKind::String,
        TokenKind::CharLiteral => LiteralKind::Char,
        TokenKind::Keyword(Keyword::True | Keyword::False) => LiteralKind::Bool,
        _ => LiteralKind::Integer,
    }
}
