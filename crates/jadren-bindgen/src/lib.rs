//! Deterministic C header generation for verified Jadren exports.

use std::collections::BTreeMap;
use std::fmt::Write;

use jadren_parser::{Annotation, AstFile, Expression, Function, Item, LiteralKind, Path, TypeRef};
use jadren_resolve::ResolutionOutput;
use jadren_source::{SourceFile, Span};
use jadren_typeck::TypeCheckOutput;

/// One deterministic header-generation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindgenError {
    /// Stable diagnostic code.
    pub code: &'static str,
    /// Human-readable message.
    pub message: String,
    /// Source range responsible for the failure.
    pub span: Span,
}

/// Result of generating one C header.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CHeader {
    /// Deterministic UTF-8 header text.
    pub text: String,
}

/// Deterministic internal C# interop source generated from the C ABI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CSharpBindings {
    /// UTF-8 C# source text.
    pub text: String,
}

/// Deterministic safe C# facade source generated over the raw bindings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CSharpFacade {
    /// UTF-8 C# source text.
    pub text: String,
}

/// Deterministic C11 static-assert source for one generated ABI header.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CLayoutTests {
    /// UTF-8 C source text.
    pub text: String,
}

/// Generates a guarded C header from one already type-checked source file.
///
/// The generator intentionally accepts only explicit `@repr(C)` records and
/// `@export(..., abi: "C")` functions. It never guesses an ABI for Jadren-only
/// containers or carriers; callers must run the normal compiler checks first.
pub fn generate_c_header(
    source: &SourceFile,
    file: &AstFile,
    _resolution: &ResolutionOutput,
    type_check: &TypeCheckOutput,
) -> Result<CHeader, Vec<BindgenError>> {
    if type_check.has_errors() {
        return Err(vec![BindgenError {
            code: "J0808",
            message: "cannot generate a C header from a type-check failure".to_owned(),
            span: Span::empty(source.id(), 0),
        }]);
    }

    let module = file
        .module
        .as_ref()
        .map(path_text)
        .unwrap_or_else(|| "jadren".to_owned());
    let guard = format!("JADREN_{}_H", c_identifier(&module).to_ascii_uppercase());
    let mut output = String::new();
    writeln!(output, "#ifndef {guard}").expect("String write");
    writeln!(output, "#define {guard}").expect("String write");
    output.push_str("#include <stdint.h>\n#include <stddef.h>\n\n");
    output.push_str(concat!(
        "typedef int32_t JadrenStatus;\n",
        "typedef struct JadrenSlice { void* pointer; size_t length; } JadrenSlice;\n",
        "typedef struct JadrenString { uint8_t* pointer; size_t length; size_t capacity; } JadrenString;\n\n",
        "typedef struct JadrenFloat2 { float lane0; float lane1; } JadrenFloat2;\n",
        "typedef struct JadrenFloat3 { float lane0; float lane1; float lane2; } JadrenFloat3;\n",
        "typedef struct JadrenFloat4 { float lane0; float lane1; float lane2; float lane3; } JadrenFloat4;\n",
        "typedef struct JadrenFloat8 { float lane0; float lane1; float lane2; float lane3; float lane4; float lane5; float lane6; float lane7; } JadrenFloat8;\n\n",
    ));
    output.push_str("#ifdef __cplusplus\nextern \"C\" {\n#endif\n\n");

    let mut errors = Vec::new();
    let repr_records: BTreeMap<_, _> = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(record) | Item::Component(record) if has_c_repr(&record.annotations) => {
                Some((record.name.text.clone(), record))
            }
            _ => None,
        })
        .collect();
    let mut records: Vec<_> = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(record) | Item::Component(record) if has_c_repr(&record.annotations) => {
                Some(record)
            }
            _ => None,
        })
        .collect();
    records.sort_by_key(|record| record.name.text.clone());
    for record in records {
        let name = c_type_name(&module, &record.name.text);
        writeln!(output, "typedef struct {name} {{").expect("String write");
        for field in &record.fields {
            match c_type(source, &module, &field.ty, field.span) {
                Ok(ty) => writeln!(output, "    {ty} {};", field.name.text).expect("String write"),
                Err(error) => errors.push(error),
            }
        }
        writeln!(output, "}} {name};\n").expect("String write");
    }

    let mut exports: Vec<_> = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(function) => export_metadata(source, function),
            _ => None,
        })
        .collect();
    exports.sort_by_key(|export| export.0.clone());
    for (_, function, export) in exports {
        if export.1 != "C" {
            errors.push(BindgenError {
                code: "J0805",
                message: format!("unsupported export ABI `{}`", export.1),
                span: export.2,
            });
            continue;
        }
        let mut parameters = Vec::with_capacity(function.parameters.len());
        for parameter in &function.parameters {
            let mut visiting = BTreeMap::new();
            if let Some(error) = reject_float8_c_abi_type(
                &parameter.ty,
                &repr_records,
                parameter.span,
                &mut visiting,
            ) {
                errors.push(error);
                continue;
            }
            match c_type(source, &module, &parameter.ty, parameter.span) {
                Ok(ty) => parameters.push(format!("{ty} {}", parameter.name.text)),
                Err(error) => errors.push(error),
            }
        }
        let result = function.return_type.as_ref().map_or_else(
            || Ok("void".to_owned()),
            |ty| {
                let mut visiting = BTreeMap::new();
                reject_float8_c_abi_type(ty, &repr_records, function.span, &mut visiting)
                    .map_or_else(|| c_type(source, &module, ty, function.span), Err)
            },
        );
        match result {
            Ok(result) => {
                writeln!(output, "{result} {}({});", export.0, parameters.join(", "))
                    .expect("String write");
            }
            Err(error) => errors.push(error),
        }
    }

    output.push_str("\n#ifdef __cplusplus\n}\n#endif\n\n#endif\n");
    if errors.is_empty() {
        Ok(CHeader { text: output })
    } else {
        Err(errors)
    }
}

/// Generates raw C# `DllImport` declarations for the verified C ABI.
///
/// This is intentionally an internal layer: the generated structs expose raw
/// pointers and lengths, while ownership validation belongs to JAD-807.
pub fn generate_csharp_bindings(
    source: &SourceFile,
    file: &AstFile,
    _resolution: &ResolutionOutput,
    type_check: &TypeCheckOutput,
    library: &str,
) -> Result<CSharpBindings, Vec<BindgenError>> {
    if type_check.has_errors() {
        return Err(vec![BindgenError {
            code: "J0808",
            message: "cannot generate C# bindings from a type-check failure".to_owned(),
            span: Span::empty(source.id(), 0),
        }]);
    }
    let module = file
        .module
        .as_ref()
        .map(path_text)
        .unwrap_or_else(|| "jadren".to_owned());
    let namespace = csharp_namespace(&module);
    let mut output = String::new();
    output.push_str("// <auto-generated />\n");
    output.push_str("using System;\nusing System.Runtime.InteropServices;\nusing JadrenStatus = System.Int32;\n\n");
    writeln!(output, "namespace {namespace};\n").expect("String write");
    output.push_str(concat!(
        "[StructLayout(LayoutKind.Sequential)]\n",
        "internal struct JadrenSlice { internal IntPtr pointer; internal UIntPtr length; }\n",
        "[StructLayout(LayoutKind.Sequential)]\n",
        "internal struct JadrenString { internal IntPtr pointer; internal UIntPtr length; internal UIntPtr capacity; }\n\n",
        "[StructLayout(LayoutKind.Sequential, Pack = 4)] internal struct JadrenFloat2 { internal float lane0; internal float lane1; }\n",
        "[StructLayout(LayoutKind.Sequential, Pack = 4)] internal struct JadrenFloat3 { internal float lane0; internal float lane1; internal float lane2; }\n",
        "[StructLayout(LayoutKind.Sequential, Pack = 4)] internal struct JadrenFloat4 { internal float lane0; internal float lane1; internal float lane2; internal float lane3; }\n",
        "[StructLayout(LayoutKind.Sequential, Pack = 4)] internal struct JadrenFloat8 { internal float lane0; internal float lane1; internal float lane2; internal float lane3; internal float lane4; internal float lane5; internal float lane6; internal float lane7; }\n\n",
    ));

    let mut errors = Vec::new();
    let repr_records: BTreeMap<_, _> = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(record) | Item::Component(record) if has_c_repr(&record.annotations) => {
                Some((record.name.text.clone(), record))
            }
            _ => None,
        })
        .collect();
    let mut records: Vec<_> = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(record) | Item::Component(record) if has_c_repr(&record.annotations) => {
                Some(record)
            }
            _ => None,
        })
        .collect();
    records.sort_by_key(|record| record.name.text.clone());
    for record in records {
        let name = csharp_type_name(&module, &record.name.text);
        output.push_str("[StructLayout(LayoutKind.Sequential)]\n");
        writeln!(output, "internal struct {name} {{").expect("String write");
        for field in &record.fields {
            match csharp_type(source, &module, &field.ty, field.span) {
                Ok(ty) => writeln!(output, "    internal {ty} {};", field.name.text)
                    .expect("String write"),
                Err(error) => errors.push(error),
            }
        }
        output.push_str("}\n\n");
    }

    let mut exports: Vec<_> = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(function) => export_metadata(source, function),
            _ => None,
        })
        .collect();
    exports.sort_by_key(|export| export.0.clone());
    for (_, function, export) in exports {
        if export.1 != "C" {
            errors.push(BindgenError {
                code: "J0805",
                message: format!("unsupported export ABI `{}`", export.1),
                span: export.2,
            });
            continue;
        }
        let mut parameters = Vec::with_capacity(function.parameters.len());
        for parameter in &function.parameters {
            let mut visiting = BTreeMap::new();
            if let Some(error) = reject_float8_c_abi_type(
                &parameter.ty,
                &repr_records,
                parameter.span,
                &mut visiting,
            ) {
                errors.push(error);
                continue;
            }
            match csharp_native_parameters(
                source,
                &module,
                &parameter.ty,
                parameter.span,
                &parameter.name.text,
            ) {
                Ok(native_parameters) => parameters.extend(native_parameters),
                Err(error) => errors.push(error),
            }
        }
        let result = function.return_type.as_ref().map_or_else(
            || Ok("void".to_owned()),
            |ty| {
                let mut visiting = BTreeMap::new();
                reject_float8_c_abi_type(ty, &repr_records, function.span, &mut visiting)
                    .map_or_else(|| csharp_type(source, &module, ty, function.span), Err)
            },
        );
        if let Ok(result) = result {
            output.push_str("internal static partial class NativeMethods {\n");
            writeln!(
                output,
                "    [DllImport(\"{}\", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]",
                csharp_string(library)
            )
            .expect("String write");
            writeln!(
                output,
                "    internal static extern {result} {}({});",
                export.0,
                parameters.join(", ")
            )
            .expect("String write");
            output.push_str("}\n\n");
        } else if let Err(error) = result {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        Ok(CSharpBindings { text: output })
    } else {
        Err(errors)
    }
}

/// Generates a safe, ownership-explicit facade over [`generate_csharp_bindings`].
///
/// The facade exposes borrowed slices through a validating view and owns
/// runtime strings through an explicit `IDisposable` handle. Exported
/// functions with ambiguous by-value String/pointer ownership are rejected
/// instead of receiving a misleading safe wrapper.
pub fn generate_csharp_facade(
    source: &SourceFile,
    file: &AstFile,
    _resolution: &ResolutionOutput,
    type_check: &TypeCheckOutput,
    _library: &str,
) -> Result<CSharpFacade, Vec<BindgenError>> {
    if type_check.has_errors() {
        return Err(vec![BindgenError {
            code: "J0808",
            message: "cannot generate a C# facade from a type-check failure".to_owned(),
            span: Span::empty(source.id(), 0),
        }]);
    }
    let module = file
        .module
        .as_ref()
        .map(path_text)
        .unwrap_or_else(|| "jadren".to_owned());
    let namespace = csharp_namespace(&module);
    let mut output = String::new();
    output.push_str("// <auto-generated safe facade />\n");
    output.push_str(
        "using System;\nusing System.Runtime.InteropServices;\nusing System.Runtime.CompilerServices;\n\n",
    );
    writeln!(output, "namespace {namespace};\n").expect("String write");
    output.push_str(concat!(
        "public readonly struct JadrenSliceView { private readonly IntPtr _pointer; private readonly UIntPtr _length; public UIntPtr Length => _length; public bool IsEmpty => _length == UIntPtr.Zero; public JadrenSliceView(IntPtr pointer, UIntPtr length) { if (length != UIntPtr.Zero && pointer == IntPtr.Zero) throw new ArgumentException(\"non-empty slice requires a pointer\", nameof(pointer)); _pointer = pointer; _length = length; } internal IntPtr Pointer => _pointer; internal UIntPtr NativeLength => _length; }\n",
        "public sealed class JadrenStringHandle : IDisposable { private JadrenString _value; private bool _disposed; public int LastStatus { get; private set; } public UIntPtr Length { get { ThrowIfDisposed(); return _value.length; } } private JadrenStringHandle(JadrenString value) { _value = value; } public static JadrenStringHandle FromUtf8(byte[] utf8) { if (utf8 is null) throw new ArgumentNullException(nameof(utf8)); var length = new UIntPtr((ulong)utf8.LongLength); if (utf8.Length == 0) return FromResult(RuntimeMethods.FromUtf8(IntPtr.Zero, length)); var pinned = GCHandle.Alloc(utf8, GCHandleType.Pinned); try { return FromResult(RuntimeMethods.FromUtf8(pinned.AddrOfPinnedObject(), length)); } finally { pinned.Free(); } } private static JadrenStringHandle FromResult(JadrenStringResult result) { if (result.status != 0) throw new InvalidOperationException($\"Jadren string creation failed: {result.status}\"); return new JadrenStringHandle(result.value); } public void Dispose() { if (_disposed) return; LastStatus = RuntimeMethods.Destroy(ref _value); _value = default; _disposed = true; GC.SuppressFinalize(this); } private void ThrowIfDisposed() { if (_disposed) throw new ObjectDisposedException(nameof(JadrenStringHandle)); } }\n",
        "[StructLayout(LayoutKind.Sequential)] internal struct JadrenStringResult { internal JadrenString value; internal int status; }\n",
        "internal static class RuntimeMethods { [DllImport(\"__RUNTIME_LIBRARY__\", EntryPoint = \"jadren_rt_string_from_utf8\", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)] internal static extern JadrenStringResult FromUtf8(IntPtr bytes, UIntPtr length); [DllImport(\"__RUNTIME_LIBRARY__\", EntryPoint = \"jadren_rt_string_destroy\", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)] internal static extern int Destroy(ref JadrenString value); }\n",
        "public static class SafeMethods {\n",
    ));
    output = output.replace("__RUNTIME_LIBRARY__", "jadren_runtime");

    let mut errors = Vec::new();
    let repr_records: BTreeMap<_, _> = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(record) | Item::Component(record) if has_c_repr(&record.annotations) => {
                Some((record.name.text.clone(), record))
            }
            _ => None,
        })
        .collect();
    let mut exports: Vec<_> = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(function) => export_metadata(source, function),
            _ => None,
        })
        .collect();
    exports.sort_by_key(|export| export.0.clone());
    for (_, function, export) in exports {
        if export.1 != "C" {
            errors.push(BindgenError {
                code: "J0805",
                message: format!("unsupported export ABI `{}`", export.1),
                span: export.2,
            });
            continue;
        }
        let mut parameters = Vec::with_capacity(function.parameters.len());
        let mut arguments = Vec::with_capacity(function.parameters.len());
        let mut supported = true;
        for parameter in &function.parameters {
            let mut visiting = BTreeMap::new();
            if let Some(error) = reject_float8_c_abi_type(
                &parameter.ty,
                &repr_records,
                parameter.span,
                &mut visiting,
            ) {
                supported = false;
                errors.push(error);
                continue;
            }
            match safe_csharp_type(source, &module, &parameter.ty, parameter.span) {
                Ok(ty) => {
                    parameters.push(format!("{ty} {}", parameter.name.text));
                    arguments.extend(safe_csharp_arguments(&parameter.ty, &parameter.name.text));
                }
                Err(error) => {
                    supported = false;
                    errors.push(error);
                }
            }
        }
        let result = function.return_type.as_ref().map_or_else(
            || Ok("void".to_owned()),
            |ty| {
                let mut visiting = BTreeMap::new();
                reject_float8_c_abi_type(ty, &repr_records, function.span, &mut visiting)
                    .map_or_else(|| safe_csharp_type(source, &module, ty, function.span), Err)
            },
        );
        let Ok(result) = result else {
            if let Err(error) = result {
                errors.push(error);
            }
            continue;
        };
        if !supported {
            continue;
        }
        writeln!(
            output,
            "    public static {result} {}({}) => NativeMethods.{}({});",
            csharp_identifier(&export.0),
            parameters.join(", "),
            export.0,
            arguments.join(", ")
        )
        .expect("String write");
    }
    output.push_str("}\n");
    if errors.is_empty() {
        Ok(CSharpFacade { text: output })
    } else {
        Err(errors)
    }
}

/// Generates C11 compile-time layout assertions for a generated header.
pub fn generate_c_layout_tests(
    source: &SourceFile,
    file: &AstFile,
    _resolution: &ResolutionOutput,
    type_check: &TypeCheckOutput,
    header_include: &str,
    pointer_bits: u16,
) -> Result<CLayoutTests, Vec<BindgenError>> {
    if type_check.has_errors() {
        return Err(vec![BindgenError {
            code: "J0808",
            message: "cannot generate ABI layout tests from a type-check failure".to_owned(),
            span: Span::empty(source.id(), 0),
        }]);
    }
    let pointer_bytes = match pointer_bits {
        32 | 64 => u64::from(pointer_bits / 8),
        _ => {
            return Err(vec![BindgenError {
                code: "J0811",
                message: format!("unsupported ABI pointer width {pointer_bits}"),
                span: Span::empty(source.id(), 0),
            }]);
        }
    };
    if header_include
        .chars()
        .any(|character| character == '"' || character == '\n' || character == '\r')
    {
        return Err(vec![BindgenError {
            code: "J0811",
            message: "header include contains an invalid C string".to_owned(),
            span: Span::empty(source.id(), 0),
        }]);
    }
    let module = file
        .module
        .as_ref()
        .map(path_text)
        .unwrap_or_else(|| "jadren".to_owned());
    let records: BTreeMap<_, _> = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(record) | Item::Component(record) if has_c_repr(&record.annotations) => {
                Some((record.name.text.clone(), record))
            }
            _ => None,
        })
        .collect();
    let mut output = String::new();
    writeln!(output, "#include \"{header_include}\"").expect("String write");
    output.push_str("#include <stddef.h>\n#include <stdint.h>\n\n");
    writeln!(
        output,
        "_Static_assert(sizeof(JadrenStatus) == 4, \"JadrenStatus size\");"
    )
    .expect("String write");
    writeln!(
        output,
        "_Static_assert(sizeof(JadrenSlice) == {}u, \"JadrenSlice size\");",
        pointer_bytes * 2
    )
    .expect("String write");
    writeln!(
        output,
        "_Static_assert(_Alignof(JadrenSlice) == {}u, \"JadrenSlice alignment\");",
        pointer_bytes
    )
    .expect("String write");
    writeln!(
        output,
        "_Static_assert(sizeof(JadrenString) == {}u, \"JadrenString size\");",
        pointer_bytes * 3
    )
    .expect("String write");
    writeln!(
        output,
        "_Static_assert(_Alignof(JadrenString) == {}u, \"JadrenString alignment\");",
        pointer_bytes
    )
    .expect("String write");
    output.push_str("_Static_assert(sizeof(JadrenFloat8) == 32u, \"JadrenFloat8 size\");\n");
    output.push_str("_Static_assert(_Alignof(JadrenFloat8) == 4u, \"JadrenFloat8 alignment\");\n");

    let mut errors = Vec::new();
    let mut sorted_records: Vec<_> = records.values().copied().collect();
    sorted_records.sort_by_key(|record| record.name.text.clone());
    for record in sorted_records {
        let mut visiting = BTreeMap::new();
        match c_record_layout(
            source,
            &module,
            record,
            &records,
            &mut visiting,
            pointer_bytes,
        ) {
            Ok(layout) => {
                let c_name = c_type_name(&module, &record.name.text);
                writeln!(
                    output,
                    "_Static_assert(sizeof({c_name}) == {}u, \"{c_name} size\");",
                    layout.size
                )
                .expect("String write");
                writeln!(
                    output,
                    "_Static_assert(_Alignof({c_name}) == {}u, \"{c_name} alignment\");",
                    layout.align
                )
                .expect("String write");
                for (field, offset) in layout.fields {
                    writeln!(
                        output,
                        "_Static_assert(offsetof({c_name}, {field}) == {}u, \"{c_name}.{field} offset\");",
                        offset
                    )
                    .expect("String write");
                }
            }
            Err(error) => errors.push(error),
        }
    }
    if errors.is_empty() {
        Ok(CLayoutTests { text: output })
    } else {
        Err(errors)
    }
}

#[derive(Clone, Debug)]
struct ComputedLayout {
    size: u64,
    align: u64,
    fields: Vec<(String, u64)>,
}

fn c_record_layout(
    source: &SourceFile,
    module: &str,
    record: &jadren_parser::RecordDeclaration,
    records: &BTreeMap<String, &jadren_parser::RecordDeclaration>,
    visiting: &mut BTreeMap<String, ()>,
    pointer_bytes: u64,
) -> Result<ComputedLayout, BindgenError> {
    if record.fields.is_empty() {
        return Err(BindgenError {
            code: "J0811",
            message: format!(
                "empty `repr(C)` record `{}` has no portable C layout",
                record.name.text
            ),
            span: record.span,
        });
    }
    if visiting.insert(record.name.text.clone(), ()).is_some() {
        return Err(BindgenError {
            code: "J0811",
            message: format!(
                "recursive `repr(C)` record `{}` has no finite layout",
                record.name.text
            ),
            span: record.span,
        });
    }
    let mut size = 0u64;
    let mut align = 1u64;
    let mut fields = Vec::with_capacity(record.fields.len());
    for field in &record.fields {
        let field_layout = c_type_layout(
            source,
            module,
            &field.ty,
            records,
            visiting,
            pointer_bytes,
            field.span,
        )?;
        size = align_up(size, field_layout.align).ok_or_else(|| BindgenError {
            code: "J0811",
            message: "C layout size overflow".to_owned(),
            span: field.span,
        })?;
        fields.push((field.name.text.clone(), size));
        size = size
            .checked_add(field_layout.size)
            .ok_or_else(|| BindgenError {
                code: "J0811",
                message: "C layout size overflow".to_owned(),
                span: field.span,
            })?;
        align = align.max(field_layout.align);
    }
    visiting.remove(&record.name.text);
    Ok(ComputedLayout {
        size: align_up(size, align).ok_or_else(|| BindgenError {
            code: "J0811",
            message: "C layout size overflow".to_owned(),
            span: record.span,
        })?,
        align,
        fields,
    })
}

fn c_type_layout(
    source: &SourceFile,
    module: &str,
    ty: &TypeRef,
    records: &BTreeMap<String, &jadren_parser::RecordDeclaration>,
    visiting: &mut BTreeMap<String, ()>,
    pointer_bytes: u64,
    span: Span,
) -> Result<ComputedLayout, BindgenError> {
    match ty {
        TypeRef::Path {
            path, arguments, ..
        } => {
            let name = path_text(path);
            let scalar = match name.as_str() {
                "Int8" | "UInt8" => Some((1, 1)),
                "Int16" | "UInt16" => Some((2, 2)),
                "Int32" | "UInt32" | "Float32" | "Status" => Some((4, 4)),
                "Int64" | "UInt64" | "Float64" => Some((8, 8)),
                "IntSize" | "UIntSize" => Some((pointer_bytes, pointer_bytes)),
                "Float2" => Some((8, 4)),
                "Float3" => Some((12, 4)),
                "Float4" => Some((16, 4)),
                "Float8" => Some((32, 4)),
                "Pointer" if arguments.len() == 1 => Some((pointer_bytes, pointer_bytes)),
                "Slice" if arguments.len() == 1 => Some((pointer_bytes * 2, pointer_bytes)),
                "String" => Some((pointer_bytes * 3, pointer_bytes)),
                _ => None,
            };
            if let Some((size, align)) = scalar {
                return Ok(ComputedLayout {
                    size,
                    align,
                    fields: Vec::new(),
                });
            }
            if let Some(record) = path
                .segments
                .last()
                .and_then(|name| records.get(&name.text))
            {
                return c_record_layout(source, module, record, records, visiting, pointer_bytes);
            }
            Err(BindgenError {
                code: "J0811",
                message: format!("type `{name}` has no computed C layout"),
                span,
            })
        }
        TypeRef::Array {
            element, length, ..
        } => {
            let text = source.slice(*length).unwrap_or_default().replace('_', "");
            let count = text.parse::<u64>().map_err(|_| BindgenError {
                code: "J0811",
                message: "array length is not a constant C layout value".to_owned(),
                span,
            })?;
            let element = c_type_layout(
                source,
                module,
                element,
                records,
                visiting,
                pointer_bytes,
                span,
            )?;
            let stride = align_up(element.size, element.align).ok_or_else(|| BindgenError {
                code: "J0811",
                message: "C array stride overflow".to_owned(),
                span,
            })?;
            let size = if count == 0 {
                0
            } else {
                stride
                    .checked_mul(count - 1)
                    .and_then(|value| value.checked_add(element.size))
                    .ok_or_else(|| BindgenError {
                        code: "J0811",
                        message: "C array size overflow".to_owned(),
                        span,
                    })?
            };
            Ok(ComputedLayout {
                size,
                align: element.align,
                fields: Vec::new(),
            })
        }
        TypeRef::Capability { inner, .. } => c_type_layout(
            source,
            module,
            inner,
            records,
            visiting,
            pointer_bytes,
            span,
        ),
        TypeRef::Function { .. } => Ok(ComputedLayout {
            size: pointer_bytes,
            align: pointer_bytes,
            fields: Vec::new(),
        }),
    }
}

fn align_up(value: u64, align: u64) -> Option<u64> {
    if align == 0 {
        return None;
    }
    let remainder = value % align;
    if remainder == 0 {
        Some(value)
    } else {
        value.checked_add(align - remainder)
    }
}

fn export_metadata<'a>(
    source: &SourceFile,
    function: &'a Function,
) -> Option<(String, &'a Function, (String, String, Span))> {
    let annotation = function
        .annotations
        .iter()
        .find(|annotation| path_text(&annotation.name) == "export")?;
    let mut name = None;
    let mut abi = None;
    for argument in &annotation.arguments {
        let Some(key) = argument.name.as_ref().map(|name| name.text.as_str()) else {
            continue;
        };
        let value = match &argument.value {
            Expression::Name(value) => value.text.clone(),
            Expression::Literal {
                kind: LiteralKind::String,
                span,
            } => source
                .slice(*span)
                .unwrap_or_default()
                .trim_matches('"')
                .to_owned(),
            _ => continue,
        };
        match key {
            "name" => name = Some(value),
            "abi" => abi = Some(value),
            _ => {}
        }
    }
    let name = name?;
    let abi = abi?;
    Some((name.clone(), function, (name, abi, annotation.span)))
}

fn c_type(
    source: &SourceFile,
    module: &str,
    ty: &TypeRef,
    span: Span,
) -> Result<String, BindgenError> {
    match ty {
        TypeRef::Path {
            path, arguments, ..
        } => {
            let name = path_text(path);
            match name.as_str() {
                "Bool" | "Char" | "Buffer" | "Option" | "Result" => Err(BindgenError {
                    code: "J0809",
                    message: format!("Jadren type `{name}` has no C 0.1 header mapping"),
                    span,
                }),
                "Int8" => Ok("int8_t".to_owned()),
                "Int16" => Ok("int16_t".to_owned()),
                "Int32" => Ok("int32_t".to_owned()),
                "Int64" => Ok("int64_t".to_owned()),
                "IntSize" => Ok("intptr_t".to_owned()),
                "UInt8" => Ok("uint8_t".to_owned()),
                "UInt16" => Ok("uint16_t".to_owned()),
                "UInt32" => Ok("uint32_t".to_owned()),
                "UInt64" => Ok("uint64_t".to_owned()),
                "UIntSize" => Ok("size_t".to_owned()),
                "Float32" => Ok("float".to_owned()),
                "Float64" => Ok("double".to_owned()),
                "Float2" => Ok("JadrenFloat2".to_owned()),
                "Float3" => Ok("JadrenFloat3".to_owned()),
                "Float4" => Ok("JadrenFloat4".to_owned()),
                "Float8" => Ok("JadrenFloat8".to_owned()),
                "Status" => Ok("JadrenStatus".to_owned()),
                "String" => Ok("JadrenString".to_owned()),
                "Unit" => Ok("void".to_owned()),
                "Pointer" if arguments.len() == 1 => Ok("void*".to_owned()),
                "Slice" if arguments.len() == 1 => Ok("JadrenSlice".to_owned()),
                _ if arguments.is_empty() => Ok(if path.segments.len() == 1 {
                    c_type_name(module, &name)
                } else {
                    c_identifier(&name)
                }),
                _ => Err(BindgenError {
                    code: "J0809",
                    message: format!("generic type `{name}` is not a C 0.1 header mapping"),
                    span,
                }),
            }
        }
        TypeRef::Array {
            element,
            span: type_span,
            ..
        } => {
            let element = c_type(source, module, element, span)?;
            let length = source.slice(*type_span).unwrap_or_default();
            let length = length
                .split(';')
                .nth(1)
                .and_then(|part| part.split(']').next())
                .map(str::trim)
                .unwrap_or("0");
            Ok(format!("{element}[{length}]"))
        }
        TypeRef::Capability { inner, .. } => c_type(source, module, inner, span),
        TypeRef::Function { .. } => Err(BindgenError {
            code: "J0809",
            message: "function pointer types require an explicit C callback ABI declaration"
                .to_owned(),
            span,
        }),
    }
}

fn csharp_type(
    _source: &SourceFile,
    module: &str,
    ty: &TypeRef,
    span: Span,
) -> Result<String, BindgenError> {
    match ty {
        TypeRef::Path {
            path, arguments, ..
        } => {
            let name = path_text(path);
            match name.as_str() {
                "Bool" | "Char" => Err(BindgenError {
                    code: "J0809",
                    message: format!("Jadren type `{name}` has no C# 0.1 ABI mapping"),
                    span,
                }),
                "Int8" => Ok("sbyte".to_owned()),
                "Int16" => Ok("short".to_owned()),
                "Int32" => Ok("int".to_owned()),
                "Int64" => Ok("long".to_owned()),
                "IntSize" => Ok("IntPtr".to_owned()),
                "UInt8" => Ok("byte".to_owned()),
                "UInt16" => Ok("ushort".to_owned()),
                "UInt32" => Ok("uint".to_owned()),
                "UInt64" => Ok("ulong".to_owned()),
                "UIntSize" => Ok("UIntPtr".to_owned()),
                "Float32" => Ok("float".to_owned()),
                "Float64" => Ok("double".to_owned()),
                "Float2" => Ok("JadrenFloat2".to_owned()),
                "Float3" => Ok("JadrenFloat3".to_owned()),
                "Float4" => Ok("JadrenFloat4".to_owned()),
                "Float8" => Ok("JadrenFloat8".to_owned()),
                "Status" => Ok("JadrenStatus".to_owned()),
                "String" => Ok("JadrenString".to_owned()),
                "Unit" => Ok("void".to_owned()),
                "Pointer" if arguments.len() == 1 => Ok("IntPtr".to_owned()),
                "Slice" if arguments.len() == 1 => Ok("JadrenSlice".to_owned()),
                "Buffer" | "Option" | "Result" => Err(BindgenError {
                    code: "J0809",
                    message: format!("Jadren type `{name}` has no C# 0.1 mapping"),
                    span,
                }),
                _ if arguments.is_empty() => Ok(if path.segments.len() == 1 {
                    csharp_type_name(module, &name)
                } else {
                    name.replace('.', "_")
                }),
                _ => Err(BindgenError {
                    code: "J0809",
                    message: format!("generic type `{name}` has no C# 0.1 mapping"),
                    span,
                }),
            }
        }
        TypeRef::Array { element, .. } => Ok(format!(
            "{}[]",
            csharp_type(_source, module, element, span)?
        )),
        TypeRef::Capability { inner, .. } => csharp_type(_source, module, inner, span),
        TypeRef::Function { .. } => Ok("IntPtr".to_owned()),
    }
}

fn safe_csharp_type(
    source: &SourceFile,
    module: &str,
    ty: &TypeRef,
    span: Span,
) -> Result<String, BindgenError> {
    match ty {
        TypeRef::Path {
            path, arguments, ..
        } => {
            let name = path_text(path);
            match name.as_str() {
                "Pointer" => Err(BindgenError {
                    code: "J0810",
                    message: "safe C# facade refuses raw Pointer ownership".to_owned(),
                    span,
                }),
                "String" => Err(BindgenError {
                    code: "J0810",
                    message: "safe C# facade requires an explicit String ownership contract"
                        .to_owned(),
                    span,
                }),
                "Slice" if arguments.len() == 1 => Ok("JadrenSliceView".to_owned()),
                _ => csharp_type(source, module, ty, span),
            }
        }
        TypeRef::Capability { inner, .. } => safe_csharp_type(source, module, inner, span),
        TypeRef::Array { .. } => Err(BindgenError {
            code: "J0810",
            message: "safe C# facade does not expose inline array ABI yet".to_owned(),
            span,
        }),
        TypeRef::Function { .. } => Err(BindgenError {
            code: "J0810",
            message: "safe C# facade requires an explicit callback ownership contract".to_owned(),
            span,
        }),
    }
}

fn csharp_native_parameters(
    source: &SourceFile,
    module: &str,
    ty: &TypeRef,
    span: Span,
    name: &str,
) -> Result<Vec<String>, BindgenError> {
    if is_slice_type(ty) {
        return Ok(vec![
            format!("IntPtr {name}_pointer"),
            format!("UIntPtr {name}_length"),
        ]);
    }
    csharp_type(source, module, ty, span).map(|mapped| vec![format!("{mapped} {name}")])
}

fn safe_csharp_arguments(ty: &TypeRef, name: &str) -> Vec<String> {
    match ty {
        TypeRef::Path { path, .. } if path_text(path) == "Slice" => {
            vec![format!("{name}.Pointer"), format!("{name}.NativeLength")]
        }
        TypeRef::Capability { inner, .. } => safe_csharp_arguments(inner, name),
        TypeRef::Function { .. } => vec![name.to_owned()],
        _ => vec![name.to_owned()],
    }
}

fn is_slice_type(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Path {
            path, arguments, ..
        } => path_text(path) == "Slice" && arguments.len() == 1,
        TypeRef::Capability { inner, .. } => is_slice_type(inner),
        TypeRef::Array { .. } => false,
        TypeRef::Function { .. } => false,
    }
}

fn csharp_namespace(module: &str) -> String {
    module
        .split('.')
        .map(csharp_identifier)
        .collect::<Vec<_>>()
        .join(".")
}

fn csharp_type_name(module: &str, name: &str) -> String {
    format!(
        "{}_{}",
        csharp_namespace(module).replace('.', "_"),
        csharp_identifier(name)
    )
}

fn csharp_identifier(text: &str) -> String {
    let mut value: String = text
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    if value
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
    {
        value.insert(0, '_');
    }
    value
}

fn csharp_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn has_c_repr(annotations: &[Annotation]) -> bool {
    annotations.iter().any(|annotation| {
        path_text(&annotation.name) == "repr"
            && annotation.arguments.len() == 1
            && matches!(annotation.arguments[0].value, Expression::Name(ref name) if name.text == "C")
    })
}

fn c_type_name(module: &str, name: &str) -> String {
    c_identifier(&format!("{module}_{name}"))
}

/// Rejects target-specific by-value `Float8` signatures at the C ABI boundary.
///
/// `Float8` is lowered internally as an LLVM vector so that CPU backends can
/// use packed instructions.  A portable C header cannot express the same
/// by-value calling convention on all supported targets, therefore 0.1 only
/// exposes it through pointer/slice memory contracts.  `@repr(C)` records may
/// still contain a `Float8` field for caller-owned tile memory, but such a
/// record may not itself cross an exported C function boundary by value.
fn reject_float8_c_abi_type(
    ty: &TypeRef,
    records: &BTreeMap<String, &jadren_parser::RecordDeclaration>,
    span: Span,
    visiting: &mut BTreeMap<String, ()>,
) -> Option<BindgenError> {
    match ty {
        TypeRef::Path {
            path, arguments, ..
        } => {
            let name = path_text(path);
            match name.as_str() {
                "Float8" => Some(BindgenError {
                    code: "J0812",
                    message: "Float8 has no portable by-value C ABI in 0.1; use a Slice or pointer-backed memory contract".to_owned(),
                    span,
                }),
                // These contracts carry an address and length/address only;
                // the element layout is caller-owned memory, not a by-value
                // platform calling convention.
                "Slice" | "Pointer" => None,
                _ => {
                    for argument in arguments {
                        if let Some(error) = reject_float8_c_abi_type(
                            argument,
                            records,
                            span,
                            visiting,
                        ) {
                            return Some(error);
                        }
                    }
                    if arguments.is_empty()
                        && let Some(record) = path
                            .segments
                            .last()
                            .and_then(|segment| records.get(&segment.text))
                    {
                        if visiting.insert(record.name.text.clone(), ()).is_some() {
                            return None;
                        }
                        for field in &record.fields {
                            if let Some(error) = reject_float8_c_abi_type(
                                &field.ty,
                                records,
                                field.span,
                                visiting,
                            ) {
                                visiting.remove(&record.name.text);
                                return Some(error);
                            }
                        }
                        visiting.remove(&record.name.text);
                    }
                    None
                }
            }
        }
        TypeRef::Array { element, .. } => {
            reject_float8_c_abi_type(element, records, span, visiting)
        }
        TypeRef::Capability { inner, .. } => {
            reject_float8_c_abi_type(inner, records, span, visiting)
        }
        TypeRef::Function {
            parameters,
            return_type,
            ..
        } => {
            for parameter in parameters {
                if let Some(error) = reject_float8_c_abi_type(parameter, records, span, visiting) {
                    return Some(error);
                }
            }
            return_type
                .as_deref()
                .and_then(|result| reject_float8_c_abi_type(result, records, span, visiting))
        }
    }
}

fn c_identifier(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn path_text(path: &Path) -> String {
    path.segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

#[cfg(test)]
mod tests {
    use jadren_lexer::lex;
    use jadren_parser::parse;
    use jadren_resolve::resolve;
    use jadren_source::SourceManager;
    use jadren_typeck::check_types;

    use super::{
        generate_c_header, generate_c_layout_tests, generate_csharp_bindings,
        generate_csharp_facade,
    };

    #[test]
    fn generates_deterministic_c_header_for_repr_c_export() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "exports.jdn",
                "module game.math; @repr(C) struct Vec3 { x: Float32, y: Float32, z: Float32 } @export(name: \"game_add\", abi: \"C\") fn add(a: Int32, b: Int32) -> Int32 { return a + b }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
        let resolution = resolve(source, &parsed.file);
        assert!(!resolution.has_errors(), "{:?}", resolution.diagnostics);
        let types = check_types(source, &parsed.file, &resolution);
        assert!(!types.has_errors(), "{:?}", types.diagnostics);
        let header = generate_c_header(source, &parsed.file, &resolution, &types).expect("header");
        assert!(header.text.contains("typedef struct game_math_Vec3"));
        assert!(
            header
                .text
                .contains("int32_t game_add(int32_t a, int32_t b);")
        );
        assert_eq!(
            header.text,
            generate_c_header(source, &parsed.file, &resolution, &types)
                .expect("header")
                .text
        );
    }

    #[test]
    fn maps_slice_string_and_status_boundary_types() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "boundary.jdn",
                "module boundary; @export(name: \"jadren_process\", abi: \"C\") fn process(data: read Slice<Float32>, text: String) -> Status { return 0 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
        let resolution = resolve(source, &parsed.file);
        assert!(!resolution.has_errors(), "{:?}", resolution.diagnostics);
        let types = check_types(source, &parsed.file, &resolution);
        assert!(!types.has_errors(), "{:?}", types.diagnostics);
        let header = generate_c_header(source, &parsed.file, &resolution, &types)
            .expect("header")
            .text;
        assert!(header.contains("typedef int32_t JadrenStatus;"));
        assert!(header.contains("JadrenSlice data, JadrenString text"));
        assert!(header.contains("JadrenStatus jadren_process"));
    }

    #[test]
    fn rejects_float8_by_value_c_abi_but_allows_slice_memory_contract() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "float8.jdn",
                "module vectors; @export(name: \"vectors_process\", abi: \"C\") fn process(value: Float8) -> Float8 { return value }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
        let resolution = resolve(source, &parsed.file);
        assert!(!resolution.has_errors(), "{:?}", resolution.diagnostics);
        let types = check_types(source, &parsed.file, &resolution);
        assert!(!types.has_errors(), "{:?}", types.diagnostics);

        let errors = generate_c_header(source, &parsed.file, &resolution, &types)
            .expect_err("by-value Float8 must not cross the C ABI");
        assert!(errors.iter().all(|error| error.code == "J0812"));
        let errors =
            generate_csharp_bindings(source, &parsed.file, &resolution, &types, "jadren_native")
                .expect_err("by-value Float8 must not cross the C# ABI");
        assert!(errors.iter().all(|error| error.code == "J0812"));
        let errors = generate_csharp_facade(source, &parsed.file, &resolution, &types, "")
            .expect_err("safe facade must preserve the by-value Float8 boundary");
        assert!(errors.iter().all(|error| error.code == "J0812"));

        let id = sources
            .add(
                "float8_slice.jdn",
                "module vectors; @export(name: \"vectors_slice\", abi: \"C\") fn process(values: read Slice<Float8>) -> Int32 { return 1 }",
            )
            .expect("slice source");
        let source = sources.get(id).expect("slice source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
        let resolution = resolve(source, &parsed.file);
        assert!(!resolution.has_errors(), "{:?}", resolution.diagnostics);
        let types = check_types(source, &parsed.file, &resolution);
        assert!(!types.has_errors(), "{:?}", types.diagnostics);
        let header = generate_c_header(source, &parsed.file, &resolution, &types)
            .expect("slice-backed Float8 contract");
        assert!(header.text.contains("JadrenSlice values"));
    }

    #[test]
    fn generates_internal_csharp_dllimport_metadata() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "exports.jdn",
                "module game.math; @export(name: \"game_add\", abi: \"C\") fn add(a: Int32, b: Int32) -> Int32 { return a + b }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
        let resolution = resolve(source, &parsed.file);
        let types = check_types(source, &parsed.file, &resolution);
        let bindings =
            generate_csharp_bindings(source, &parsed.file, &resolution, &types, "jadren_native")
                .expect("C# bindings")
                .text;
        assert!(bindings.contains("DllImport(\"jadren_native\""));
        assert!(bindings.contains("internal static extern int game_add(int a, int b);"));
        assert!(bindings.contains("CallingConvention.Cdecl"));
        assert!(bindings.contains("ExactSpelling = true"));
    }

    #[test]
    fn flattens_slice_parameters_for_csharp_pinvoke_abi() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "slice.jdn",
                "module slice; @export(name: \"process\", abi: \"C\") fn process(data: read Slice<Float32>, value: Int32) -> Int32 { return value }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolution = resolve(source, &parsed.file);
        let types = check_types(source, &parsed.file, &resolution);
        assert!(!types.has_errors(), "{:?}", types.diagnostics);
        let bindings =
            generate_csharp_bindings(source, &parsed.file, &resolution, &types, "jadren_native")
                .expect("C# bindings")
                .text;
        assert!(bindings.contains(
            "internal static extern int process(IntPtr data_pointer, UIntPtr data_length, int value);"
        ));
    }

    #[test]
    fn generates_safe_facade_with_slice_validation_and_scalar_wrapper() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "safe.jdn",
                "module safe; @export(name: \"process\", abi: \"C\") fn process(data: read Slice<Float32>, value: Int32) -> Int32 { return value }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolution = resolve(source, &parsed.file);
        let types = check_types(source, &parsed.file, &resolution);
        assert!(!types.has_errors(), "{:?}", types.diagnostics);
        let facade =
            generate_csharp_facade(source, &parsed.file, &resolution, &types, "jadren_native")
                .expect("facade")
                .text;
        assert!(facade.contains("JadrenSliceView"));
        assert!(facade.contains("non-empty slice requires a pointer"));
        assert!(facade.contains("public static int process(JadrenSliceView data, int value)"));
        assert!(facade.contains("NativeMethods.process(data.Pointer, data.NativeLength, value)"));
        assert!(facade.contains("DllImport(\"jadren_runtime\""));
        assert!(facade.contains("IDisposable"));
    }

    #[test]
    fn generates_c_layout_static_asserts() {
        let mut sources = SourceManager::new();
        let id = sources
            .add(
                "layout.jdn",
                "module layout; @repr(C) struct Vec3 { x: Float32, y: Float32, z: Float32 }",
            )
            .expect("source");
        let source = sources.get(id).expect("source");
        let lexed = lex(source);
        let parsed = parse(source, &lexed.tokens);
        let resolution = resolve(source, &parsed.file);
        let types = check_types(source, &parsed.file, &resolution);
        assert!(!types.has_errors(), "{:?}", types.diagnostics);
        let tests =
            generate_c_layout_tests(source, &parsed.file, &resolution, &types, "layout.h", 64)
                .expect("layout tests")
                .text;
        assert!(tests.contains("#include \"layout.h\""));
        assert!(tests.contains("sizeof(layout_Vec3) == 12u"));
        assert!(tests.contains("offsetof(layout_Vec3, z) == 8u"));
        assert!(tests.contains("sizeof(JadrenSlice) == 16u"));
        assert!(tests.contains("sizeof(JadrenFloat8) == 32u"));
        assert!(tests.contains("_Alignof(JadrenFloat8) == 4u"));
    }
}
