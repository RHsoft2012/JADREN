# Getting started

This guide uses the compiler from the public source repository. The repository
is currently the 0.1.3-preview.11 public preview: it provides source, examples,
editor support, and the Unity preview packages under `unity/`. The Windows
preview installer is unsigned and updates the user PATH; installation may
require administrator confirmation. The Downloads page provides a versioned Unity Integration
Bundle; local packages and native plugins must match the same release line.

## Requirements

- Windows or Linux x86-64 for native CLI development;
- Rust 1.97.0, selected by `rust-toolchain.toml`;
- LLVM 22.1.x for native LLVM code generation;
- PowerShell 7 is recommended on Windows; Docker Desktop is optional for a
  repeatable Linux validation environment.

## Build the compiler

From the repository root:

```powershell
cargo build -p jadren-cli
cargo run -p jadren-cli -- version
cargo run -p jadren-cli -- doctor
```

## Linux developer builds

The public repository supports a native Linux x86-64 development build when the
matching LLVM toolchain is installed. From the repository root:

```bash
cargo run -p jadren-cli -- version
cargo run -p jadren-cli -- doctor
cargo run -p jadren-cli -- run examples/hello.jdn
```

The public CI validates the pinned Linux path. This is not evidence for every
Linux distribution or bare-metal configuration.

## Run the first program

Create `hello.jdn`:

```jadren
module examples.hello

fn main() {
    print("Hello, Jadren")
}
```

Check, build, and run it with:

```powershell
cargo run -p jadren-cli -- check hello.jdn
cargo run -p jadren-cli -- build hello.jdn
cargo run -p jadren-cli -- run hello.jdn
```

`build` creates a native x86-64 executable in the compiler's configured output
directory. Use `-o <path>`, `--profile release`, or `--cpu avx2` to select an
output location, optimized build, or explicit AVX2 code generation. The executable
entry is a parameterless
`fn main()` returning either `Unit` or `Int32`; an `Int32` result becomes the
process exit code. The Windows and Linux console runtimes support the built-in
`print(String)` used by this example.

For a guided calculation-to-window path, continue with the
[practical beginner course](BEGINNER_PRACTICAL_COURSE.md).

## Windows desktop preview

`examples/windows-desktop-preview.jdn` is an executed Windows x86-64 preview
of a Jadren program opening a native Win32 window. Direct Jadren UI functions
define the title, dimensions, `0xRRGGBB` colours, labels, status text, buttons,
visible action messages, and corner radii without C or Win32 declarations.

Text inputs can be read back from the Jadren callback without an FFI bridge:
`ui_input_length(event_id)` reports the current UTF-8 byte length and
`ui_input_read(event_id, output)` copies the bytes into a caller-owned
`write Slice<UInt8>` and returns the copied length. A fixed Jadren array such
as `var output: [UInt8; 128]` is borrowed as that slice automatically; an
existing `Buffer<UInt8>` can be used as well. See
`examples/windows-desktop-input.jdn` for a complete program.

Lists are mutable after `ui_run()` as well: `ui_list_item` appends a row,
`ui_list_set_item` replaces one, `ui_list_clear` removes all rows and
`ui_list_count` returns the current bounded count. See
`examples/windows-desktop-list.jdn` for a callback-driven example.

List content can also be read back into a caller-owned UTF-8 buffer with
`ui_list_read_item(event_id, item_index, output)`. The helper returns the exact
copied byte count and returns `0` when the index is invalid or the output slice
is too small.

Tables use the bounded native report view: declare `ui_table_column` headers,
fill rows with `ui_table_cell`, and read or change selection with
`ui_table_selected_row`/`ui_table_set_selected_row`. `ui_table_clear` and the
row-count query are safe from callbacks; the preview keeps up to 8 columns and
64 rows. See `examples/windows-desktop-table.jdn`.

Cell text is available to Jadren callbacks through
`ui_table_read_cell(event_id, row_index, column_index, output)`. It uses the
same bounded UTF-8 read-back contract as list items, so a program can bind a
selected row to its own state without an FFI declaration or hidden allocation.

For the common scalar cases, `ui_state_bind(event_id, slot, mode)` keeps a
native control and a local `ui_state` slot synchronized before the callback
runs. Mode `0` binds checkbox/switch state bidirectionally, mode `1` binds a
select/list/table index bidirectionally, mode `2` publishes text-input UTF-8
length, and mode `3` publishes list/table count. Slots remain bounded to
`0..31`; the binding table is fixed-size and allocation-free. See
`examples/windows-desktop-state-binding.jdn` for a complete program.

The retained UI contract also supports bounded hover help without a C/Win32
bridge: call `ui_app_tooltip(node, text, width, height, text_color,
background_color, corner_radius)` immediately after `ui_app_button` or
`ui_app_checkbox`. The Windows backend uses the native tooltip controller and
the X11 backend paints the hint above the hovered control.

## Local file persistence

Jadren programs can persist raw bytes without an FFI declaration. The bounded
file API is available on Windows and Linux x86-64:

```jadren
fn main() -> Int32 {
    var payload: [UInt8; 4] = [1u8, 2u8, 3u8, 4u8]
    let written: UIntSize = file_write("data.bin", payload)
    var output: [UInt8; 4] = [0u8, 0u8, 0u8, 0u8]
    let loaded: UIntSize = file_read("data.bin", output)
    file_delete("data.bin")
    return (written + loaded) as Int32
}
```

`file_exists`, `file_size`, `file_read`, `file_write`, and `file_delete` use
caller-owned data. A fixed array or `Buffer<UInt8>` is borrowed as a bounded
`Slice<UInt8>` automatically; `file_read` never writes beyond that slice. Paths
are UTF-8 and limited to the runtime's fixed 1024-byte/code-unit path buffer.
Relative paths are resolved against the process working directory. The native
ABI passes `String` and `Slice` as explicit pointer/length pairs, so no hidden
serialization or allocation is introduced.

When an existing file must cross an explicit durability boundary, call
`file_flush(path)` after the write or append succeeds. Windows uses
`FlushFileBuffers` and Linux uses `fsync`; the call does not flush directory
metadata, make a rename transactional, or turn the file into a database.

For a multi-process application, reserve a separate lock path and use the
non-blocking token API:

```jadren
let lock: UIntSize = file_lock("target/app-data.lock")
if lock == 0usize { return 1 }
// read/write the protected checkpoint here
if !file_unlock(lock) { return 2 }
```

Windows uses an exclusive `CreateFileW` handle and Linux uses an advisory
`fcntl(F_SETLK)` record lock. The caller owns the token and must release it on
every path; the lock API does not queue, retry, or create a hidden scheduler.
See `scripts/check-file-lock.ps1` for holder/contention/reacquire coverage.

For a crash-resistant replacement pattern, write the new bytes to a temporary
path and then call `file_replace_atomic(source_path, target_path)`. Windows uses
`MoveFileExW` with replace/write-through flags; Linux uses the same-filesystem
atomic `rename` contract. The temporary path is caller-selected and must be
kept in the same directory/filesystem as the target.

For small text exports, `file_write_text(path, content)` writes a UTF-8
`String` directly and returns the number of bytes written. It is useful for
CSV, line-oriented logs, and simple JSON examples; it does not format values or
allocate a temporary string for the caller.

Use `file_append_text(path, content)` for bounded streaming: write the header
once, append rows or chunks, then use `file_size` or `file_replace_atomic` as
needed. Each call is independent and writes only the supplied UTF-8 block.

Directory setup is explicit as well. `directory_exists(path)` distinguishes a
directory from a file, `directory_create(path)` creates one and is idempotent
when the directory already exists, and `directory_delete(path)` removes only an
empty directory. The same three builtins use Win32 on Windows and POSIX
`mkdir`/`rmdir` on Linux; `examples/directory-runtime.jdn` exercises the full
create/check/write/delete/cleanup flow.

`directory_list(path, output)` provides a bounded one-level file-picker source.
It writes UTF-8 file and subdirectory basenames, sorted by byte order and
separated by LF without a trailing newline. The caller owns the writable byte
buffer; if the complete result does not fit, the runtime returns `0` without
modifying it. See `examples/directory-list-runtime.jdn` for capacity, sorting,
cleanup, and non-ASCII filename coverage.

When a picker also needs the entry kind, use
`directory_list_ex(path, names, kinds)`. It returns the entry count and writes
parallel kind bytes in the same sorted order: `1` is a regular file, `2` is a
directory, and `3` is another filesystem entry. Both caller-owned buffers are
validated before either is written. See
`examples/directory-list-ex-runtime.jdn` for capacity, ordering, and cleanup.

For text configuration and import files, `file_read_text(path, output)` is the
bounded counterpart to `file_write_text`. It reads the complete file only when
the caller-owned byte buffer has room for one trailing NUL, returns the UTF-8
byte length without that terminator, and leaves the buffer untouched when the
path or capacity is invalid. The example
`examples/file-read-text-runtime.jdn` covers the capacity guard, Unicode bytes,
terminator, and cleanup.

For large files, use `file_read_at(path, offset, output)` to read a bounded chunk
without loading the whole file. The runtime seeks to the byte offset and returns
the number of bytes read; an offset beyond EOF returns `0` without changing the
caller-owned buffer. See `examples/file-read-at-runtime.jdn` for chunking and the
EOF guard.

To update one region without rewriting a whole file, use
`file_write_at(path, offset, input)`. It creates the file when needed, preserves
existing bytes outside the written range, and returns the number of bytes written.
Writing beyond EOF uses the native filesystem's gap semantics. The example
`examples/file-write-at-runtime.jdn` covers an in-place chunk update and a write
after EOF.

Use `file_copy(source_path, target_path)` for a bounded native backup copy. The
target must not already exist, so a failed copy cannot silently replace user
data; incomplete targets are removed. Windows uses `CopyFileW`, while Linux
copies through a fixed caller-independent 8 KiB transfer buffer.

Dynamic rows can use `format_int`, `format_uint`, and `format_bool` with a
caller-owned byte array, then `file_append(path, buffer, length)` to append
only the formatted prefix. The formatters are deterministic decimal/boolean
conversions and return `0` when the output slice is too small.

`format_float(value, buffer)` is the bounded floating-point companion. It
writes six fixed decimal places without locale or allocation; IEEE `nan`,
`inf`, and `-inf` are emitted as ASCII literals. Use a 32-byte buffer for
ordinary values and treat a `0` return as a rejected range or insufficient
capacity.

For one field at a time, `csv_escape(value, buffer)` adds CSV quoting only
when needed and doubles embedded quotes. `json_escape(value, buffer)` emits a
complete quoted JSON string and escapes quotes, backslashes, and control bytes.
Both preserve UTF-8 bytes, use caller-owned output, and return `0` when the
buffer cannot hold the complete field; `examples/escaping-export.jdn` shows
their bounded file-export flow.

For a small JSON object, `json_object_field_string(key, value, buffer)` writes
one complete escaped member such as `"name":"Aho"`. The caller owns commas,
ordering, braces, and the file sink, so the helper remains bounded and does
not allocate; `examples/json-object-export.jdn` shows the complete flow.

Numeric and boolean members use the same shape:
`json_object_field_int`, `json_object_field_uint`, `json_object_field_float`,
and `json_object_field_bool`. They write deterministic JSON primitives into a
caller-owned buffer; non-finite floats are rejected because JSON has no valid
`nan`/`inf` literals. See `examples/json-numeric-export.jdn` for a complete
object export with capacity rejection and cleanup.

Primitive arrays use `json_array_int`, `json_array_uint`, `json_array_float`,
and `json_array_bool`. They accept a caller-owned array or `read Slice<T>` and
write a complete bounded JSON array, including brackets and separators. Float
arrays reject non-finite elements. The caller still owns any surrounding object
or nested-array syntax; see `examples/json-array-export.jdn`.

Flat JSON objects can be loaded back from a file using
`json_object_read_string`, `json_object_read_int`, `json_object_read_uint`,
`json_object_read_float`, and `json_object_read_bool`. They scan caller-owned JSON bytes without allocation;
the reader intentionally rejects nested objects/arrays and treats missing or
invalid scalar members as `0`/`0.0`/`false`; non-finite float values are rejected. String values are copied into a bounded
output slice. `examples/json-object-persistence.jdn` demonstrates the
write-file/read-file/restore-state path.

For application code that should not hand-build JSON for every setting, the
bounded process-local store provides `app_state_clear`, typed
`app_state_set_*`/`app_state_get_*`, `app_state_read_text`, `app_state_save`,
and `app_state_load`. It keeps at most 32 keys, each key is at most 64 safe
bytes, and text values are at most 256 UTF-8 bytes. `save` writes a flat JSON
object and `load` accepts primitive string, integer, unsigned integer, and
boolean members; malformed or nested JSON is rejected without replacing the
existing store. The store is intentionally one-process and allocation-free,
not a database, schema system, or transaction log. For crash-resistant
replacement, write a temporary JSON path and finish with
`file_replace_atomic`; see `examples/app-state-persistence.jdn`.
For dynamic settings, `app_state_count` reports the number of keys,
`app_state_exists` distinguishes a missing key from a stored zero, false, or
empty string; `app_state_read_key` enumerates keys in insertion order, and
`app_state_remove` deletes one key without resetting the rest.
`app_state_save_atomic` combines temporary JSON output and atomic replacement
for crash-resistant settings updates.

The composed desktop example `examples/time-tracker.jdn` shows the intended
next level: it restores three bounded task rows, binds the table selection,
reads a text input into a caller-owned buffer, marks a row complete, persists
the flags as JSON, and exports a CSV report. It is a reference application
shape for growing Jadren APIs; it does not hide an unbounded database or a
foreign UI bridge.

For application data that must grow beyond one scalar key, `app_list_*` offers
four bounded process-local text lists. Use `app_list_push_text` and
`app_list_count` to add and inspect items, `app_list_set_text` to update an
existing item, `app_list_remove` to close a gap, and `app_list_read_text` to
copy UTF-8 bytes into a caller-owned buffer. Each list has 64 items and each
value has a 256-byte limit; `examples/app-list-runtime.jdn` is a runnable
non-UI example. `app_list_save` writes one list as a JSON string array and
`app_list_load` restores it transactionally; see
`examples/app-list-persistence.jdn`. This is an explicit bounded storage
primitive while a generic heap collection and typed records remain future
language work.
`app_list_save_atomic` adds crash-resistant replacement by writing a temporary
JSON path before atomically replacing the target.
For deterministic list queries, `app_list_sort_text(list_id, descending)`
stably sorts UTF-8 values in place and `app_list_find_text(list_id, query,
start_index)` returns the first exact match from a bounded starting index, or
`-1`. See `examples/app-list-query.jdn`.
`-1`. `app_list_filter_text` performs an exact projection into a different
bounded list; `app_list_filter_text_ex` adds contains, prefix, suffix and ASCII
case-insensitive modes (`0..7`) while preserving source order. See
`examples/app-list-filter.jdn`.
`app_list_filter_text_ex_bytes` accepts a caller-owned UTF-8 `read Slice<UInt8>`
and an explicit valid length, so a text input can drive the same projection
without constructing a temporary `String`. The source list stays unchanged;
the destination is cleared only after the slice-length contract is valid. See
`examples/app-list-filter-bytes.jdn`.
For application-defined selection use
`app_list_filter_callback(source_list_id, destination_list_id, predicate)`.
The predicate has type `fn(Int32, Int32) -> Bool` and receives the source list
id plus item index. The runtime checks a source fingerprint around the bounded
scan and publishes the destination only after success; keep the callback
read-only. See `examples/app-list-filter-callback.jdn`.
For application-defined ordering use
`app_list_sort_callback(list_id, comparator)`. The comparator has type
`fn(Int32, Int32, Int32) -> Int32` and receives the list id plus two item
indices; negative, zero, and positive results define left-first, stable-equal,
and right-first order. The runtime sorts a temporary bounded permutation,
checks the source fingerprint around every callback, and publishes the new
order only after the read-only scan succeeds. See
`examples/app-list-sort-callback.jdn`.
For paged views, `app_list_page(source_list_id, destination_list_id,
start_index, page_size)` copies at most 64 items in source order into a
separate bounded list. The final page may be shorter than requested and
`start_index == app_list_count(source_list_id)` is a valid empty page; invalid
ranges leave the destination unchanged. This is an explicit caller-driven
projection, not a hidden collection or planner.
`app_list_push_text_bytes` appends the valid UTF-8 prefix of a caller-owned
`read Slice<UInt8>`, while `app_list_set_text_bytes` replaces one existing item.
Both require an explicit length no greater than the slice capacity and return
`false` without changing the list when that contract is invalid. See
`examples/app-list-input-bytes.jdn` for input, rejection, update and read-back
in one bounded flow.
`app_list_export_csv(list_id, output)` writes each item as one CSV row into a
caller-owned `write Slice<UInt8>`. Standard quoting is applied for commas,
quotes and line breaks; the runtime calculates the complete size first and
writes nothing when the output capacity is too small. See
`examples/app-list-export-csv.jdn`.

For row/column application data, `app_table_*` provides four bounded tables
with 64 rows and 8 text columns. Call `app_table_append_row`,
`app_table_set_cell`, `app_table_set_cell_bytes`, `app_table_read_cell`, `app_table_remove_row`, and
`app_table_row_count`; the model is independent from the Windows `ui_table`
widget. `examples/app-table-runtime.jdn` is a runnable example. The bounded
model supports stable byte-wise `app_table_sort_text` and exact-match
`app_table_find_text` from a chosen start row; a general filter engine and
automatic UI projection remain later stages.
`app_table_filter_text` provides the first bounded exact-match projection into
a separate destination table; the source remains unchanged. See
`examples/app-table-filter.jdn`.
`app_table_filter_text_ex` extends this with explicit exact, contains, prefix,
suffix, and ASCII case-insensitive modes (`0..7`); non-ASCII bytes remain
byte-exact. `app_table_filter_text_ex_bytes` accepts a caller-owned UTF-8
slice plus an explicit valid length, so a UI input can drive the projection
without first constructing a temporary `String`. See the CRUD flow in
`examples/time-tracker-pro.jdn`.
For repeated exact lookups, `app_table_index_build(table_id, column_index)`
creates a deterministic bounded text index and `app_table_index_find_text`
returns the first source row for a key. `app_table_index_is_valid` reports
whether the table contents still match the index and `app_table_index_clear`
removes it; mutations invalidate the index automatically. See
`examples/app-table-index.jdn`.
For a common two-field lookup, `app_table_index_build_pair(table_id,
first_column_index, second_column_index)` builds a deterministic lexicographic
index over two distinct text columns and `app_table_index_find_pair_text`
returns the first row matching both exact keys. The pair index is bounded to
64 rows and uses the same stale fingerprint rule; numeric ranges and callback
predicates remain separate layers. For an inclusive signed-`Int` range,
`app_table_index_collect_int_range(table_id, column_index, lower, upper, output)`
writes matching source row ids into caller-owned `write Slice<Int32>` in sorted
index order and returns the match count. A stale index, invalid range, or short
output returns zero without partial output writes. See
`examples/app-table-index-range-int.jdn` and
`examples/app-table-pair-index.jdn`. The same contract is available for
`UInt64` and finite `Float64` columns through
`app_table_index_collect_uint_range` and `app_table_index_collect_float_range`;
reversed ranges and `NaN` bounds return zero. See
`examples/app-table-index-range-numeric.jdn`.
For application-defined predicates use
`app_table_filter_callback(source_table_id, destination_table_id, predicate)`.
The predicate has type `fn(Int32, Int32) -> Bool` and receives the source table
id plus row index. Matching rows are projected in source order; the runtime
checks the source fingerprint during the bounded scan and commits the
destination only after success. Keep the callback read-only. See
`examples/app-table-filter-callback.jdn`.
For application-defined ordering use
`app_table_sort_callback(table_id, comparator)`. The comparator has type
`fn(Int32, Int32, Int32) -> Int32` and receives the table id plus two row ids;
negative, zero, and positive results mean left-first, stable-equal, and
right-first ordering. The runtime sorts a temporary bounded permutation,
checks the table fingerprint around every callback, and publishes the new
order only after the complete read-only scan succeeds. See
`examples/app-table-sort-callback.jdn`.
For paged table views, `app_table_page(source_table_id, destination_table_id,
start_row, page_size)` preserves the source schema and copies at most 64 rows
in source order. A short final page and `start_row == app_table_row_count(...)`
are valid; invalid ranges or an invalid source leave the destination unchanged.
`app_table_save` writes the complete table as a nested JSON array and
`app_table_load` restores it transactionally; see
`examples/app-table-persistence.jdn`. Query behavior is covered by
`examples/app-table-query.jdn`.
`app_table_save_atomic` provides the same crash-resistant temporary/target
sequence for table JSON.
`app_table_export_csv` writes the active header and rows into a caller-owned
bounded output slice with standard quoting and no partial output on a small
buffer. `app_table_import_csv(table_id, input, input_length)` reads an explicit
valid CSV prefix, first checks `input_length <= capacity(input)`, validates its
header, row limits and existing typed schema, and replaces the table only after
the complete parse succeeds. This allows a larger `file_read_text` buffer with
spare capacity and a trailing terminator without parsing unused bytes. See
`examples/app-table-csv-roundtrip.jdn` for the rollback path.
`app_table_set_cell_bytes` accepts a caller-owned UTF-8 slice, which lets a
form read an input buffer and write it into a table without a temporary
`String` allocation. See the CRUD pattern in the TimeTracker lesson.
`app_table_set_cell_bytes_ex` adds an explicit valid byte length, which is the
safe form for a larger input buffer whose unused tail must not become part of
the cell.
For text input state, `ui_state_bind_text` provides a bounded bidirectional
slot binding, while `ui_state_text_read` and `ui_state_text_set` expose
caller-owned UTF-8 read-back and programmatic updates.
Native list/table controls can bind to bounded `app_list`/`app_table` models
with `ui_list_bind_app` and `ui_table_bind_app`. `ui_refresh_bindings()`
refreshes every bound list, table and text input in one pass. The Windows
preview also runs that pass automatically after `jadren_ui_on_click`, while
the per-control `ui_list_refresh_app` and `ui_table_refresh_app` calls remain
available when a program wants a narrower deterministic update. See
`examples/windows-desktop-auto-binding.jdn`.
Text forms can bind to a persisted `app_state` key with
`ui_input_bind_app_state`; native edits are stored automatically and
`ui_input_refresh_app_state` reloads the key after `app_state_load`.
Use `app_state_tx_begin`, `app_state_tx_commit`, and `app_state_tx_rollback` to
group bounded state changes behind a confirm/cancel flow. The snapshot is
process-local and in-memory; durable persistence still uses the save APIs.
Tables can declare process-local column kinds with
`app_table_set_column_type(table, column, kind)`: `0=text`, `1=int`, `2=uint`,
`3=bool`, `4=float`. `app_table_validate` checks non-empty cells before saving or binding;
the existing JSON row format stays compatible and schema must be configured
again before loading. Filtered destination tables inherit the source schema,
and loading rejects rows that do not match the configured schema.
`app_table_save_schema` and `app_table_load_schema` persist the eight column
kinds in a separate bounded JSON array; the atomic save variant is available
for crash-safe replacement.
Typed scalar reads are available through `app_table_read_int`,
`app_table_read_uint`, `app_table_read_float`, and `app_table_read_bool`; they require the matching
column kind and return bounded defaults for invalid or empty cells.
Typed scalar writes are available through `app_table_set_int`,
`app_table_set_uint`, `app_table_set_float`, and `app_table_set_bool`; the runtime formats the value
and validates it against the selected column kind.
`app_table_tx_begin`, `app_table_tx_commit`, and `app_table_tx_rollback` provide
a bounded process-local snapshot for grouped table edits; rollback restores the
rows and schema together.
Named typed-record metadata uses `app_table_set_column_name`,
`app_table_read_column_name`, and `app_table_find_column` with unique ASCII
identifiers. `app_table_save_schema_full` (or its atomic variant) writes all
eight `{name, kind}` entries and `app_table_load_schema_full` restores them
transactionally; the legacy kind-only schema JSON remains unchanged.
Named-field helpers `app_table_set_named_cell`/`app_table_read_named_cell` and
the typed named int/uint/float/bool setters and readers address rows by stable field
name, so application code need not manually call `app_table_find_column`.
Full-schema documents carry a non-negative application-managed `version`.
`app_table_schema_version`/`app_table_set_schema_version` read and update it;
`app_table_load_schema_full` treats legacy unversioned documents as version `0`,
and `app_table_load_schema_full_if_version` refuses a mismatched version before
changing the live table. Use it inside `app_table_tx_*` when implementing an
explicit application migration.
When one operation changes related tables, use
`app_table_tx_begin_all`/`app_table_tx_commit_all` or
`app_table_tx_rollback_all` for a bounded in-memory snapshot across all four
tables. The all-table and single-table transaction modes are mutually exclusive.
For a form that changes state, lists, and tables together, use
`app_data_tx_begin`, then commit or rollback the complete bounded model with
`app_data_tx_commit` or `app_data_tx_rollback`.
To persist that complete model in one file, use `app_data_save` or
`app_data_save_atomic`; `app_data_load` validates the bounded length-framed
checkpoint before replacing any live store. The checkpoint is capped at 4 MiB,
while the existing separate state/list/table JSON APIs remain compatible.
For a caller-owned network or IPC buffer, `app_data_write_exact(output,
length)` exports the same complete model without a temporary file, and
`app_data_load_exact(input, input_length)` validates the whole prefix before
replacing state, lists and tables. A short or malformed buffer leaves both the
outputs and the live model unchanged.
Use `app_data_write_exact_if_revision(output, length, expected_revision)` when
an HTTP or IPC controller must export only the equality-only model revision it
read; stale calls return `false` before changing the caller-owned buffer or
length. Transport, locking, and retry remain explicit caller responsibilities.
Use `app_data_load_exact_if_revision(input, input_length, expected_revision)`
when a delayed response must not overwrite local changes made after the
request started; the stale token is rejected before the transactional restore.
For a single explicit polling boundary across the complete model, use
`app_data_revision() -> UInt64`. Compare the opaque token with a previously
saved value and refresh derived UI only when equality changes; it covers
`app_state`, all bounded lists and all bounded tables. It is not ordered,
monotonic, or cryptographic, and it does not start a hidden reactive loop.
Use `app_data_tx_begin_if_revision(expected_revision)` before a grouped edit
when a stale caller must not overwrite a newer model. It opens the complete
state/list/table transaction only when the equality-only token still matches;
otherwise it returns `false` without opening a transaction. This is a
process-local caller-driven guard, not a cross-process lock or durable commit.
Before exporting, sending, or committing the model, call
`app_data_validate() -> Bool` as one no-mutation preflight across state, all
bounded lists, and all table schemas/cells. It does not replace atomic file
commit, locking, or database constraints.
When a controller must persist exactly the revision it read, use
`app_data_save_atomic_if_revision(temporary_path, target_path, expected_revision)`.
The atomic promotion is rejected when the model-wide equality-only revision is
stale; this remains a process-local, caller-driven guard.
For append-only crash recovery, `app_data_journal_append(journal_path,
scratch_path)` writes a length- and FNV-1a-checksum-framed checkpoint and
`app_data_journal_recover(journal_path, scratch_path)` restores the last
complete, checksum-valid frame while ignoring incomplete or corrupted tails.
Legacy `0.1` length-only frames remain readable. The bounded journal is still
not an fsync, cross-process locking, or database-transaction layer.
`app_data_journal_recover_compact(journal_path, scratch_path, temporary_path)`
recovers the last valid frame, writes one canonical `0.2` frame to the
caller-owned temporary path, and atomically promotes it over the journal. The
temporary path must be distinct from both the journal and scratch path; the
runtime clears it before writing. This remains a bounded
process-local compaction helper, not an fsync or database commit.
`app_data_journal_recover_compact_durable(journal_path, scratch_path,
temporary_path, lock_path)` holds a separate cross-process lock during
recovery and replacement, flushes the temporary frame before the atomic swap,
and flushes the promoted journal before releasing the lock. Keep the lock path
separate from all three data paths; a post-replacement flush failure can leave
the new journal on disk, so reload it before retrying.
To make rotation caller-driven, `app_data_journal_compact_if_over_durable(
journal_path, scratch_path, temporary_path, lock_path, max_bytes)` compacts
only when the current journal exceeds the explicit byte threshold; at or below
the threshold it is a no-op. Choose a threshold that can hold one canonical
frame. No background worker or implicit retry is introduced.
For retention by checkpoint count, use
`app_data_journal_compact_if_frames_over_durable(journal_path, scratch_path,
temporary_path, lock_path, max_frames)`. It counts only complete checksum-valid
frames, is a no-op at or below the limit, and otherwise holds the lock through
recovery, flush and atomic replacement before leaving the newest valid checkpoint.
This is explicit caller policy, not a background worker.
When recovery needs more than the newest checkpoint, use
`app_data_journal_retain_last_durable(journal_path, scratch_path,
temporary_path, lock_path, max_frames)`. It preserves the last complete valid
frames in order under the lock and atomically replaces the journal; at or below
the limit it is a no-op. This is bounded caller-owned WAL retention, not a
hidden scheduler or database engine.
To inspect a retained history entry, call
`app_data_journal_recover_frame_durable(journal_path, scratch_path, lock_path,
frame_index)`. The index is zero-based among complete valid frames; checksum
failures and torn tails are ignored, and an out-of-range index returns `false`
without replacing the live model.
Use `app_data_journal_count_frames_durable(journal_path, lock_path)` immediately
before selecting an index when the caller needs the current replay range. It
holds the same separate lock and counts only complete valid v1/v2 frames, so a
torn tail is ignored. The `UIntSize` result is `0usize` for an empty,
unreadable, or missing journal; callers that need to distinguish those cases
must add their own diagnostic path.
For a consistent maintenance snapshot, use
`app_data_journal_stats_durable(journal_path, lock_path, output)`. With a
two-element caller-owned `UIntSize` slice it writes the complete-frame count to
`output[0]` and the journal byte size to `output[1]` under one lock. A short
output or missing journal returns `false` without changing either slot.
To combine both limits in one scheduler-owned maintenance boundary, call
`app_data_journal_compact_if_needed_durable(journal_path, scratch_path,
temporary_path, lock_path, max_bytes, max_frames)`. It holds the lock while
checking both thresholds and while compacting to the newest valid snapshot only
when at least one limit is exceeded. At or below both limits it is a successful
no-op; both limits must be nonzero. No worker or implicit retry is introduced.
For a non-mutating decision before choosing the write path, use
`app_data_journal_maintenance_plan_durable(journal_path, lock_path, max_bytes,
max_frames, output)`. Its three `UIntSize` slots contain action `0` (none), `1`
(canonical compaction), or `2` (retain frames), followed by the complete-frame
count and byte size from one lock-held snapshot. The plan is advisory after the
lock is released, so the selected mutating call must revalidate its own state.
When a short cross-process lock collision is expected, use
`app_data_journal_maintenance_retry_durable(journal_path, scratch_path,
temporary_path, lock_path, max_bytes, max_frames, max_attempts,
retry_delay_ms)`. It repeats the complete maintenance boundary a finite number
of caller-selected times and optionally sleeps between failed attempts.
`max_attempts` must be nonzero; no worker or unbounded scheduler is created.
Before choosing a caller-owned buffer capacity, use
`app_data_journal_frame_length_durable(journal_path, lock_path, frame_index)`.
It holds the separate lock, scans complete checksum-valid v1/v2 frames and
returns the exact payload length without allocating or copying a hidden
buffer. `0usize` means an invalid/torn frame, missing journal or out-of-range
index; valid zero-length frames are not part of the journal format. The length
preflight and the subsequent exact read are separate lock-held calls, so a
caller that allows concurrent retention must provide its own stable-snapshot
policy.
For query or export metadata without copying a payload, use
`app_data_journal_frame_span_durable(journal_path, lock_path, frame_index,
output)`. It writes frame start offset, total framed bytes and payload bytes to
three caller-owned `UIntSize` slots from one lock-held checksum-valid snapshot.
Short output or an invalid frame leaves the output unchanged.
For repeated query/export access, build a persistent index with
`app_data_journal_build_index_durable(journal_path, index_path, temporary_path,
lock_path)`. The flushed fixed-width index records the journal byte size and
checksum plus each frame span. Use
`app_data_journal_index_lookup_durable(journal_path, index_path, lock_path,
frame_index, output)` to reject stale or malformed index data before writing
the three span slots. Keep the temporary index path separate from the journal,
index and lock paths.
For a compact diagnostic or export hand-off, use
`app_data_journal_index_export_csv_durable(journal_path, index_path, lock_path,
output)`. It validates the same snapshot and writes
`frame_index,offset,total_bytes,payload_bytes` plus one numeric row per indexed
frame. The return value is the exact byte length; stale/malformed indexes and
short caller-owned buffers return `0` without partial output.
For exports larger than one caller-owned buffer, use
`app_data_journal_index_export_csv_file_durable(journal_path, index_path,
lock_path, output_path, temporary_path)`. It validates the complete snapshot,
writes rows to the distinct temporary file, flushes it and atomically promotes
the final CSV. A stale/malformed index or write failure removes only the
temporary file and leaves an existing output file unchanged.
For paged table or export access, use
`app_data_journal_index_range_durable(journal_path, index_path, lock_path,
start_frame, max_frames, output)`. It validates the same snapshot and writes
three `UIntSize` values per returned frame (offset, total bytes, payload bytes).
The return value is the page count; short output or an out-of-range start leaves
the caller buffer unchanged.
`app_data_journal_index_read_page_durable(journal_path, index_path, lock_path,
start_frame, max_frames, output, metadata)` reads the same page's complete
payloads contiguously under one snapshot lock. Metadata uses four `UIntSize`
slots per frame (journal offset, total bytes, payload bytes, output offset), so
the caller can slice the payload page without another lock/read round-trip.
For the common case of reading the newest checkpoint, use
`app_data_journal_read_latest_frame_exact_durable(journal_path, lock_path,
output, length)`. It discovers the last complete valid frame and copies it
under the same lock, so retention cannot change the selected index between
discovery and read; short buffers and empty journals return `false` without a
partial write.
If you need the bytes of one retained checkpoint for a diff, export or IPC
message without changing the live model, use
`app_data_journal_read_frame_exact_durable(journal_path, lock_path, frame_index,
output, length)`. It holds the same separate lock, validates one complete
checksum-valid v1/v2 frame and checks the caller-owned output capacity before
copying. On success `length[0]` is the exact payload length; a short buffer,
torn/invalid frame, missing journal or out-of-range index returns `false`
without partial output or model mutation.
For a native HTTP client request, `http_request_write(method, target, host,
body, output)` builds a validated HTTP/1.1 message in caller-owned memory;
send it explicitly through the TCP API.
Use `http_request_write_header_block(...)` when a request needs multiple
validated custom headers in one bounded message.
For one client cookie pair, use
`jadren.network.http.write_request_cookie(...)`; it validates the cookie name
and value and emits a bounded `Cookie: name=value` header without socket I/O.
For a stateless cookie guard, import
`jadren.network.auth.cookie_value_matches`; it compares one exact `name=value`
pair in caller-owned memory without creating or storing a session.
To issue a dynamic cookie response, use
`jadren.network.http.write_response_cookie_ex(...)`; it validates the cookie
name/value and writes one bounded `Set-Cookie` header with explicit keep-alive
mode. Expiry, rotation, CSRF, TLS and session storage remain application policy.
For a bounded local request policy, import
`jadren.network.policy.rate_limit_allow` and keep a two-element caller-owned
`[UInt64; 2]` state array: element zero is the window start in milliseconds and
element one is the accepted count. `rate_limit_retry_after_ms` reports the
remaining window without sleeping or creating a worker. The policy is local and
deterministic; distributed coordination, client-state eviction, persistence and
HTTP `429` serialization remain application/server responsibilities. See
`examples/network-policy-project` for a complete package fixture.
To serialize a rejected request without hand-writing HTTP framing, use
`http_response_write_header(429, "text/plain", "Retry-After", seconds, body,
output)`. The writer validates one custom header, performs a complete capacity
check before writing, and returns `0` without partial output on failure.
`http_response_write_header_ex` adds the explicit keep-alive boolean. See
`examples/http-response-header.jdn` for a bounded `429` response that is then
read back through the response parser.
For several custom headers, use `http_response_write_header_block` with a
CRLF-separated block such as `Retry-After: 1\r\nX-Trace: limited`. It validates
each field before writing, rejects framing headers and never leaves a partial
response; the `_ex` variant adds explicit keep-alive mode.
The `time_now_unix_seconds() -> Int64` helper provides the current UTC Unix
timestamp directly from the native runtime with one-second precision. It is a
useful source value for TimeTracker rows and checkpoints; calendar formatting
and time zones remain a higher-level library concern.
For elapsed durations, use the separate `time_now_monotonic_ms() -> UInt64`
helper. It uses a monotonic host clock and is not affected by wall-clock
adjustments; it is not a calendar timestamp.
To split a Unix timestamp into displayable fields, call
`time_utc_parts(timestamp, output)` with a caller-owned `write Slice<Int32>` of
at least six elements. It writes `[year, month, day, hour, minute, second]` in
UTC and never consults the host's local timezone.
For an explicit fixed offset, use
`time_utc_offset_parts(timestamp, offset_minutes, output)`. The offset is in
minutes (for example `60` or `-60`); this API deliberately does not load an
IANA timezone database or apply implicit daylight-saving rules.
For caller-driven reminders, use the bounded application timer queue:
`app_scheduler_set(task_id, due_unix_seconds, repeat_seconds)` followed by
`app_scheduler_poll(now_unix_seconds, output)`. The queue is fixed at 64 entries,
does not create threads or block, and keeps its state unchanged when the output
slice is too small.
The first allocation-free String helpers are `string_length(String)` and
`string_equals(String, String)`. They operate on exact UTF-8 bytes; the length
is a byte count, not a Unicode scalar count, and neither helper changes
ownership.
`string_builder_append(String, write Slice<UInt8>, UIntSize)` and
`string_builder_append_bytes(read Slice<UInt8>, write Slice<UInt8>, UIntSize)`
append into a caller-owned bounded byte buffer and return the next offset. They
perform no allocation and leave the buffer unchanged when capacity is too
small. For multi-step text composition, use the move-only `OwnedString` type:
`string_owned_from(String)` and `string_owned_create(UIntSize)` return
`Result<OwnedString, Int32>`, while `string_owned_append`,
`string_owned_length`, `string_owned_copy`, and `string_owned_clear` provide
explicit mutation and bounded export operations. The compiler emits automatic
cleanup through `jadren_rt_owned_string_destroy`; allocation and invalid UTF-8
fail with stable status codes instead of partial output.

### Direct TCP server slice

Jadren also exposes a deliberately small native TCP contract:
`net_tcp_listen`, `net_tcp_accept`, `net_tcp_connect`, `net_tcp_send`,
`net_tcp_receive`, `net_tcp_send_prefix`, and `net_socket_close`. The
`net_tcp_connect_dns` variant delegates hostname lookup to the native IPv4
resolver and remains explicitly blocking. The
`examples/tcp-server.jdn` example listens on `127.0.0.1`, accepts one client,
configures `net_socket_set_timeout` in milliseconds, reads one caller-owned byte
buffer, sends one bounded response, and closes both sockets. Socket operations
are blocking I/O and carry an opaque `UIntSize`
token; `connect` supports `localhost` and IPv4 literals only. For an explicit
TLS client, wrap the connected socket with `net_tls_open_client(socket,
server_name, verify_peer)`, advance `net_tls_step(tls, timeout_ms)` until it
returns `2`, then use `net_tls_send`/`net_tls_receive` and `net_tls_close`.
`net_tls_state` reports `closed`, `handshaking`, `open`, `error`, or
`peer-closed`, while `net_tls_error` exposes the native diagnostic code.
Windows uses Schannel and Linux uses the native OpenSSL backend. Keep
`verify_peer=true` in production; `false` is only for an explicit local
self-signed example. A bounded readiness reactor is available separately via
`net_reactor_open`, `net_reactor_watch`, `net_reactor_poll` and the event
getters; Linux uses `epoll` and Windows uses a bounded `select` adapter for the
classic watch API. One-shot `net_reactor_submit_accept`,
`net_reactor_submit_connect`, `net_reactor_submit_receive` and
`net_reactor_submit_send` return operation tokens; the `_buffer` variants accept
caller-owned `read`/`write Slice<UInt8>` buffers. `net_reactor_event_operation`
identifies the completed operation and `net_reactor_cancel` cancels a pending
one. Windows completes these one-shot operations through native overlapped IOCP
(`AcceptEx`, `ConnectEx`, zero-byte `WSARecv`/`WSASend`); flags `16` and `32`
report timeout and cancellation. Receive/send operations only notify readiness,
so the caller-owned buffer is still passed to `net_tcp_receive`/`net_tcp_send`
after the event. Buffered variants complete the transfer directly and expose
the byte count through `net_reactor_event_bytes`. The reactor never owns watched sockets. Connection pools and a
general HTTP framework remain separate contracts, not hidden behavior of these
calls. The complete accept/connect/receive/send flow is in
`examples/net-reactor-operations.jdn`; `examples/net-reactor-timeout.jdn`
shows the timeout flag and deterministic cleanup.

A bounded server uses `net_tls_open_server(socket, certificate, private_key)`
after `net_tcp_accept`. Windows expects an explicit PFX/PKCS#12 certificate
path and password; Linux expects PEM certificate and private-key paths. The
server token owns the accepted socket and exposes the same `step`, `state`,
`error`, `send`, `receive`, and `close` operations.

For a server that should keep the HTTP routing/session loop in Jadren, use
`http_session_open_tls(listener, max_connections, max_header_bytes,
max_body_bytes, certificate, private_key)`. It keeps the bounded session
limits, performs the TLS handshake when a connection is accepted, and routes
encrypted request/response bytes through the existing HTTP router. Windows
uses a PFX path plus password; Linux uses PEM certificate and private-key
paths. See `examples/http-session-tls-server-pfx.jdn`,
`examples/http-session-tls-server.jdn`, and
`examples/http-session-tls-client.jdn`.

### Bounded HTTP response writer

`http_response_write(status, content_type, body, output)` builds a complete
HTTP/1.1 response in a caller-owned byte buffer. It writes a reason phrase,
`Content-Type`, `Content-Length`, `Connection: close`, and the body after a
two-pass capacity check; `0` means invalid input or insufficient capacity and
no partial response is emitted. The helper performs serialization only: it
does not parse requests or perform TCP/TLS I/O. See
`examples/http-response-export.jdn` for a deterministic file-backed smoke
example. Request parsing, TLS, async scheduling, and a higher-level router
remain separate contracts.

### Bounded HTTP response reader

For a native client, read only the bytes actually returned by the socket with
`http_response_status_prefix(input, input_length)`,
`http_response_header_prefix(input, input_length, name, output)`, and
`http_response_body_prefix(input, input_length, output)`. The reader validates
the `HTTP/1.1` status line and completed headers, performs case-insensitive
header lookup, and requires one unambiguous `Content-Length` before copying the
body. It rejects incomplete prefixes, duplicate framing headers, chunked
transfer, and small output slices without partial output. The non-prefix
variants are convenient when the whole slice is valid. See
`examples/http-response-parser.jdn`; chunked/HTTP2 framing and TLS/session
state remain higher-level contracts.

### Bounded HTTP request parser

The direct HTTP input helpers are `http_request_is_complete(input)`,
`http_request_method(input, output)`, `http_request_target(input, output)`,
`http_request_header(input, name, output)`, and
`http_request_body(input, output)`. They parse caller-owned HTTP/1.1 bytes
without allocation. The parser validates the request line and header framing,
rejects duplicate `Content-Length` and `Transfer-Encoding`, and only copies a
body when a valid `Content-Length` is available. A `0` result means invalid
input or insufficient output capacity. See
`examples/http-request-parser.jdn`; TCP transport, TLS, async scheduling, and
router policy remain separate layers.

### Direct route matching

`http_route_match(input, expected_method, expected_target)` is the first
server-side composition helper. It compares the parsed HTTP/1.1 request line
exactly, performs no allocation or network I/O, and can be combined with the
TCP listener and `http_response_write`. The loopback composition example is
`examples/http-server.jdn` with `examples/http-client.jdn`; TLS, async, and a
dynamic route table remain separate layers.

For a small caller-driven server, use the bounded route table:
`http_router_clear()`, `http_router_add(method, target, status, content_type,
body)`, `http_router_add_prefix(method, target_prefix, status, content_type,
body)`, `http_router_respond(input, output)`, and `http_router_count()`.
When the receive buffer is larger than the valid request, use
`http_router_respond_prefix(request, request_length, output)` so dispatch reads
only the explicit request prefix.
When a response is generated into a larger caller-owned buffer, use
`http_router_add_exact(method, target, status, content_type, body,
body_length)` so only the validated prefix is published.
The runtime stores at most 16 exact method/target routes with fixed-size
content and body fields. Adding an existing pair replaces it; an invalid or
oversized registration leaves the previous table unchanged. A missing route
produces a bounded 404 response. `http_router_remove(method, target)` removes
one exact route without shifting or reallocating the bounded table and returns
`false` when it is absent. This layer performs no allocation, threading,
keep-alive, TLS, or socket I/O; compose it with the explicit TCP calls and an
application-owned event boundary. See `examples/http-router.jdn`.
Prefix routes match the beginning of the request target after the method has
matched. Exact routes always win; among prefix routes the longest prefix wins,
with stable slot order for equal lengths. Use `http_router_remove_prefix` to
remove a registered prefix without affecting exact routes.

`http_query_param(input, key, output)` extracts one exact query parameter from
a request target or raw query slice. It percent-decodes `%HH` and `+` into a
caller-owned buffer and rejects malformed, duplicate, or oversized values.

For responses whose serialized buffer is larger than the actual payload, use
`net_tcp_send_prefix(socket, input, length)`. It sends only the requested
prefix after checking `length <= input.length`, so a server can pass the exact
length returned by `http_response_write` without transmitting trailing bytes.
The call remains explicit blocking socket I/O and returns the number of bytes
accepted by the native send operation.

```powershell
cargo run -p jadren-cli -- build examples/windows-desktop-preview.jdn `
  -o <output-executable> --profile release
<output-executable>
```

The preview recognizes direct `ui_*` Jadren calls and links its small Windows
runtime automatically. It is a native Windows preview, not yet a
cross-platform widget toolkit; additional controls and layout primitives are
added incrementally to the same direct Jadren API.

The development CLI also exposes formatting and intermediate-representation
inspection commands:

```powershell
cargo run -p jadren-cli -- format hello.jdn --check
cargo run -p jadren-cli -- emit ast hello.jdn
cargo run -p jadren-cli -- emit hir hello.jdn
cargo run -p jadren-cli -- emit mir hello.jdn
cargo run -p jadren-cli -- emit jir hello.jdn
```

## Important limitations

- The language specification is still a draft.
- A signed public installer and release artifacts are not yet available.
- Native `build` and `run` currently target Windows and Linux x86-64.
- Target support differs by platform and workload.
- GPU and mobile targets require their own execution validation.
- Benchmark results are meaningful only for the exact published workload,
  layout, hardware, and build profile.
