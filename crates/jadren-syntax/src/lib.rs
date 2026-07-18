//! Lossless hierarchical syntax representation for Jadren source.

use std::fmt;
use std::fmt::Write as _;

use jadren_lexer::Token;
use jadren_source::{SourceFile, SourceId, Span};

/// Grammar-oriented syntax node category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyntaxKind {
    /// Whole source file.
    Root,
    /// Module declaration.
    ModuleDeclaration,
    /// Import declaration.
    ImportDeclaration,
    /// Declaration annotation.
    Annotation,
    /// Function declaration.
    FunctionDeclaration,
    /// Top-level `extern "C" { ... }` declaration block.
    ExternBlock,
    /// One function declaration inside an extern block.
    ExternFunctionDeclaration,
    /// Struct declaration.
    StructDeclaration,
    /// Component declaration.
    ComponentDeclaration,
    /// Enum declaration.
    EnumDeclaration,
    /// Generic parameter list including angle brackets.
    GenericParameterList,
    /// One generic parameter and its bounds.
    GenericParameter,
    /// Function parameter list including parentheses.
    ParameterList,
    /// One function parameter.
    Parameter,
    /// Path type, optionally with generic arguments.
    PathType,
    /// Fixed-size array type.
    ArrayType,
    /// `owned`, `read`, or `write` capability wrapper type.
    CapabilityType,
    /// Explicit function pointer type such as `fn(Int32) -> Int32`.
    FunctionType,
    /// Struct or component field declaration.
    FieldDeclaration,
    /// Enum variant declaration.
    EnumVariant,
    /// Named enum payload field.
    EnumVariantField,
    /// Braced block.
    Block,
    /// Immutable or mutable binding statement.
    BindingStatement,
    /// Return statement.
    ReturnStatement,
    /// Lexical region statement with a named allocator handle.
    RegionStatement,
    /// `while` loop statement.
    WhileStatement,
    /// `for binding in iterable` loop statement.
    ForStatement,
    /// `break` loop-control statement.
    BreakStatement,
    /// `continue` loop-control statement.
    ContinueStatement,
    /// Name expression.
    NameExpression,
    /// Literal expression.
    LiteralExpression,
    /// Unary expression.
    UnaryExpression,
    /// Binary or assignment expression.
    BinaryExpression,
    /// Call expression.
    CallExpression,
    /// Field access expression.
    FieldExpression,
    /// Index expression.
    IndexExpression,
    /// Postfix error-propagation expression.
    TryExpression,
    /// Explicit numeric cast expression using `as`.
    CastExpression,
    /// Array literal expression.
    ArrayExpression,
    /// Struct literal expression.
    StructExpression,
    /// One named field initializer inside a struct literal.
    StructFieldInitializer,
    /// Parenthesized expression.
    GroupExpression,
    /// Block expression.
    BlockExpression,
    /// If expression.
    IfExpression,
    /// Match expression.
    MatchExpression,
    /// Match arm.
    MatchArm,
    /// Wildcard match pattern.
    WildcardPattern,
    /// Path match pattern.
    PathPattern,
    /// Constructor match pattern.
    ConstructorPattern,
    /// Literal match pattern.
    LiteralPattern,
    /// Parser recovery placeholder.
    Error,
}

/// Description of a syntax node before tree construction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeSpec {
    /// Node category.
    pub kind: SyntaxKind,
    /// Exact node range.
    pub span: Span,
}

impl NodeSpec {
    /// Creates a node description.
    #[must_use]
    pub const fn new(kind: SyntaxKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// One tree element.
#[derive(Clone, Debug)]
pub enum SyntaxElement {
    /// Nested grammar node.
    Node(SyntaxNode),
    /// Original lexer token.
    Token(Token),
}

/// Immutable lossless syntax node.
#[derive(Clone, Debug)]
pub struct SyntaxNode {
    kind: SyntaxKind,
    span: Span,
    children: Vec<SyntaxElement>,
}

impl SyntaxNode {
    /// Returns the node category.
    #[must_use]
    pub const fn kind(&self) -> SyntaxKind {
        self.kind
    }

    /// Returns the exact node range.
    #[must_use]
    pub const fn span(&self) -> Span {
        self.span
    }

    /// Returns child nodes and tokens in source order.
    #[must_use]
    pub fn children(&self) -> &[SyntaxElement] {
        &self.children
    }

    /// Iterates over direct grammar-node children in source order.
    pub fn child_nodes(&self) -> impl Iterator<Item = &SyntaxNode> {
        self.children.iter().filter_map(|child| match child {
            SyntaxElement::Node(node) => Some(node),
            SyntaxElement::Token(_) => None,
        })
    }

    /// Iterates over direct lexer-token children in source order.
    pub fn child_tokens(&self) -> impl Iterator<Item = &Token> {
        self.children.iter().filter_map(|child| match child {
            SyntaxElement::Node(_) => None,
            SyntaxElement::Token(token) => Some(token),
        })
    }

    /// Counts all lexer tokens below this node, including trivia and EOF.
    #[must_use]
    pub fn token_count(&self) -> usize {
        self.children
            .iter()
            .map(|child| match child {
                SyntaxElement::Node(node) => node.token_count(),
                SyntaxElement::Token(_) => 1,
            })
            .sum()
    }

    fn reconstruct_into(&self, source: &SourceFile, output: &mut String) {
        for child in &self.children {
            match child {
                SyntaxElement::Node(node) => node.reconstruct_into(source, output),
                SyntaxElement::Token(token) => {
                    if let Some(text) = source.slice(token.span) {
                        output.push_str(text);
                    }
                }
            }
        }
    }

    fn pretty_into(&self, source: &SourceFile, depth: usize, output: &mut String) {
        let indent = "  ".repeat(depth);
        let _ = writeln!(
            output,
            "{indent}{:?} {}..{}",
            self.kind, self.span.start, self.span.end
        );
        for child in &self.children {
            match child {
                SyntaxElement::Node(node) => node.pretty_into(source, depth + 1, output),
                SyntaxElement::Token(token) => {
                    let text = source.slice(token.span).unwrap_or_default();
                    let _ = writeln!(
                        output,
                        "{indent}  {:?} {}..{} `{}`",
                        token.kind,
                        token.span.start,
                        token.span.end,
                        text.escape_debug()
                    );
                }
            }
        }
    }

    fn pretty_nodes_into(&self, depth: usize, output: &mut String) {
        let indent = "  ".repeat(depth);
        let _ = writeln!(
            output,
            "{indent}{:?} {}..{}",
            self.kind, self.span.start, self.span.end
        );
        for child in &self.children {
            if let SyntaxElement::Node(node) = child {
                node.pretty_nodes_into(depth + 1, output);
            }
        }
    }
}

/// Whole lossless syntax tree.
#[derive(Clone, Debug)]
pub struct SyntaxTree {
    root: SyntaxNode,
}

impl SyntaxTree {
    /// Builds and validates a lossless tree from lexer tokens and nested grammar spans.
    pub fn build(
        source: SourceId,
        source_len: usize,
        tokens: &[Token],
        mut nodes: Vec<NodeSpec>,
    ) -> Result<Self, SyntaxBuildError> {
        let root_span =
            Span::new(source, 0, source_len).ok_or(SyntaxBuildError::InvalidRootSpan)?;
        validate_tokens(root_span, tokens)?;

        nodes.sort_by(|left, right| {
            left.span
                .start
                .cmp(&right.span.start)
                .then_with(|| right.span.end.cmp(&left.span.end))
                .then_with(|| syntax_kind_order(left.kind).cmp(&syntax_kind_order(right.kind)))
        });
        validate_nodes(root_span, &nodes)?;

        let mut direct_children = vec![Vec::new(); nodes.len()];
        let mut root_children = Vec::new();
        let mut stack: Vec<usize> = Vec::new();

        for index in 0..nodes.len() {
            while let Some(parent) = stack.last().copied() {
                if strictly_contains(nodes[parent].span, nodes[index].span) {
                    break;
                }
                if spans_overlap(nodes[parent].span, nodes[index].span) {
                    return Err(SyntaxBuildError::CrossingNodes {
                        left: nodes[parent].span,
                        right: nodes[index].span,
                    });
                }
                let _ = stack.pop();
            }

            if let Some(parent) = stack.last().copied() {
                direct_children[parent].push(index);
            } else {
                root_children.push(index);
            }
            stack.push(index);
        }

        let root = build_node(
            SyntaxKind::Root,
            root_span,
            &root_children,
            &nodes,
            &direct_children,
            tokens,
        )?;
        Ok(Self { root })
    }

    /// Creates a flat token-preserving tree for internal recovery paths.
    ///
    /// Normal parser output should use [`Self::build`]. This constructor deliberately
    /// skips structural validation so diagnostics can still retain the original input
    /// if a projected grammar range is invalid.
    #[must_use]
    pub fn flat(source: SourceId, source_len: usize, tokens: &[Token]) -> Self {
        let root_span = Span::new(source, 0, source_len).unwrap_or_else(|| Span::empty(source, 0));
        Self {
            root: SyntaxNode {
                kind: SyntaxKind::Root,
                span: root_span,
                children: tokens.iter().copied().map(SyntaxElement::Token).collect(),
            },
        }
    }

    /// Returns the root node.
    #[must_use]
    pub const fn root(&self) -> &SyntaxNode {
        &self.root
    }

    /// Reconstructs the original source text byte-for-byte from token ranges.
    #[must_use]
    pub fn reconstruct(&self, source: &SourceFile) -> String {
        let mut output = String::with_capacity(source.text().len());
        self.root.reconstruct_into(source, &mut output);
        output
    }

    /// Returns a stable human-readable tree dump.
    #[must_use]
    pub fn pretty(&self, source: &SourceFile) -> String {
        let mut output = String::new();
        self.root.pretty_into(source, 0, &mut output);
        output
    }

    /// Returns a stable tree dump containing grammar nodes but omitting lexer tokens.
    #[must_use]
    pub fn pretty_nodes(&self) -> String {
        let mut output = String::new();
        self.root.pretty_nodes_into(0, &mut output);
        output
    }
}

/// Invalid input to the syntax tree builder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyntaxBuildError {
    /// Root span could not be created.
    InvalidRootSpan,
    /// Token is outside the source or belongs to another source file.
    TokenOutsideRoot(Span),
    /// Lexer tokens overlap or are out of source order.
    OverlappingTokens { previous: Span, current: Span },
    /// Grammar node is empty, outside the source, or belongs to another file.
    NodeOutsideRoot(Span),
    /// Two nodes describe exactly the same range.
    DuplicateNodeSpan(Span),
    /// Node spans cross rather than nest.
    CrossingNodes { left: Span, right: Span },
    /// A token crosses a grammar node boundary.
    TokenCrossesNode { token: Span, node: Span },
}

impl fmt::Display for SyntaxBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRootSpan => formatter.write_str("invalid syntax tree root span"),
            Self::TokenOutsideRoot(span) => {
                write!(formatter, "token outside syntax root: {span:?}")
            }
            Self::OverlappingTokens { previous, current } => write!(
                formatter,
                "tokens overlap or are unordered: {previous:?} then {current:?}"
            ),
            Self::NodeOutsideRoot(span) => {
                write!(formatter, "syntax node outside root: {span:?}")
            }
            Self::DuplicateNodeSpan(span) => {
                write!(formatter, "duplicate syntax node range: {span:?}")
            }
            Self::CrossingNodes { left, right } => {
                write!(formatter, "syntax nodes cross: {left:?} and {right:?}")
            }
            Self::TokenCrossesNode { token, node } => {
                write!(formatter, "token {token:?} crosses node {node:?}")
            }
        }
    }
}

impl std::error::Error for SyntaxBuildError {}

fn validate_tokens(root: Span, tokens: &[Token]) -> Result<(), SyntaxBuildError> {
    let mut previous: Option<Span> = None;
    for token in tokens {
        if token.span.source != root.source
            || token.span.start < root.start
            || token.span.end > root.end
        {
            return Err(SyntaxBuildError::TokenOutsideRoot(token.span));
        }
        if let Some(previous_span) = previous
            && previous_span.end > token.span.start
        {
            return Err(SyntaxBuildError::OverlappingTokens {
                previous: previous_span,
                current: token.span,
            });
        }
        previous = Some(token.span);
    }
    Ok(())
}

fn validate_nodes(root: Span, nodes: &[NodeSpec]) -> Result<(), SyntaxBuildError> {
    for (index, node) in nodes.iter().enumerate() {
        if node.span.source != root.source
            || node.span.is_empty()
            || node.span.start < root.start
            || node.span.end > root.end
        {
            return Err(SyntaxBuildError::NodeOutsideRoot(node.span));
        }
        if nodes
            .get(index + 1)
            .is_some_and(|next| next.span == node.span)
        {
            return Err(SyntaxBuildError::DuplicateNodeSpan(node.span));
        }
    }
    Ok(())
}

fn build_node(
    kind: SyntaxKind,
    span: Span,
    child_indices: &[usize],
    nodes: &[NodeSpec],
    direct_children: &[Vec<usize>],
    tokens: &[Token],
) -> Result<SyntaxNode, SyntaxBuildError> {
    let mut children = Vec::new();
    let mut token_index = 0;

    for child_index in child_indices {
        let child_spec = nodes[*child_index];
        while token_index < tokens.len() && tokens[token_index].span.end <= child_spec.span.start {
            children.push(SyntaxElement::Token(tokens[token_index]));
            token_index += 1;
        }

        let child_token_start = token_index;
        while token_index < tokens.len() && tokens[token_index].span.start < child_spec.span.end {
            if tokens[token_index].span.end > child_spec.span.end {
                return Err(SyntaxBuildError::TokenCrossesNode {
                    token: tokens[token_index].span,
                    node: child_spec.span,
                });
            }
            token_index += 1;
        }

        let child = build_node(
            child_spec.kind,
            child_spec.span,
            &direct_children[*child_index],
            nodes,
            direct_children,
            &tokens[child_token_start..token_index],
        )?;
        children.push(SyntaxElement::Node(child));
    }

    children.extend(
        tokens[token_index..]
            .iter()
            .copied()
            .map(SyntaxElement::Token),
    );
    Ok(SyntaxNode {
        kind,
        span,
        children,
    })
}

fn strictly_contains(parent: Span, child: Span) -> bool {
    parent.source == child.source
        && parent.start <= child.start
        && child.end <= parent.end
        && (parent.start < child.start || child.end < parent.end)
}

fn spans_overlap(left: Span, right: Span) -> bool {
    left.source == right.source && left.start < right.end && right.start < left.end
}

const fn syntax_kind_order(kind: SyntaxKind) -> u8 {
    kind as u8
}

#[cfg(test)]
mod tests {
    use jadren_lexer::lex;
    use jadren_source::{SourceManager, Span};

    use super::{NodeSpec, SyntaxBuildError, SyntaxKind, SyntaxTree};

    #[test]
    fn reconstructs_every_source_byte_and_retains_every_token() {
        let text = "// leading\nfn main() { /* body */ print(\"hi\") }\n";
        let mut sources = SourceManager::new();
        let id = sources.add("test.jdn", text).expect("source ID should fit");
        let source = sources.get(id).expect("source should exist");
        let output = lex(source);
        let function_start = text.find("fn").expect("function start");
        let block_start = text.find('{').expect("block start");
        let block_end = text.find('}').expect("block end") + 1;
        let tree = SyntaxTree::build(
            id,
            text.len(),
            &output.tokens,
            vec![
                NodeSpec::new(
                    SyntaxKind::FunctionDeclaration,
                    Span::new(id, function_start, block_end).expect("ordered span"),
                ),
                NodeSpec::new(
                    SyntaxKind::Block,
                    Span::new(id, block_start, block_end).expect("ordered span"),
                ),
            ],
        )
        .expect("valid syntax tree");

        assert_eq!(tree.reconstruct(source), text);
        assert_eq!(tree.root().token_count(), output.tokens.len());
        assert!(tree.pretty(source).contains("FunctionDeclaration"));
        assert!(tree.pretty(source).contains("Block"));
        let function = tree.root().child_nodes().next().expect("function node");
        assert_eq!(function.kind(), SyntaxKind::FunctionDeclaration);
        assert_eq!(function.child_nodes().count(), 1);
        assert!(function.child_tokens().count() >= 4);
    }

    #[test]
    fn rejects_crossing_node_ranges() {
        let text = "abcdef";
        let mut sources = SourceManager::new();
        let id = sources.add("test.jdn", text).expect("source ID should fit");
        let source = sources.get(id).expect("source should exist");
        let output = lex(source);
        let error = SyntaxTree::build(
            id,
            text.len(),
            &output.tokens,
            vec![
                NodeSpec::new(
                    SyntaxKind::FunctionDeclaration,
                    Span::new(id, 0, 4).expect("ordered span"),
                ),
                NodeSpec::new(
                    SyntaxKind::Block,
                    Span::new(id, 2, 6).expect("ordered span"),
                ),
            ],
        )
        .expect_err("crossing ranges must fail");

        assert!(matches!(error, SyntaxBuildError::CrossingNodes { .. }));
    }
}
