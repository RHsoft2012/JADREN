# Practical beginner course: from calculation to a native window

This course is for a first-time Jadren user who wants to build a small native
desktop program without an Electron bridge. It progresses through four
runnable fixtures: a calculation, a window, a button callback, and validated
text input. Jadren 0.1 remains a preview: use the commands below to verify the
examples on your own toolchain instead of treating them as a stable language
guarantee.

## Before you start

Install the CLI and its pinned toolchain as described in
[Getting started](GETTING_STARTED.md). In a terminal at the repository root,
use the same short feedback loop for every lesson:

```powershell
cargo run -p jadren-cli -- check examples/lesson-calculations.jdn
cargo run -p jadren-cli -- run examples/lesson-calculations.jdn
cargo run -p jadren-cli -- build examples/lesson-calculations.jdn -o target/calculations.exe
```

`check` validates syntax and types, `run` executes the example, and `build`
creates a standalone native executable. Every source file linked below lives in
`examples/`; no framework project or JavaScript runtime is required.

## Lesson 1: make one calculation explicit

Start with named parameters and an explicit result type:

```jadren
module lessons.calculations

fn add(a: Int32, b: Int32) -> Int32 {
    return a + b
}

fn main() -> Int32 {
    let total: Int32 = add(12, 8)
    if total != 20 { return 1 }
    print("Calculation complete.")
    return 0
}
```

Run the complete fixture:
[lesson-calculations.jdn](https://github.com/RHsoft2012/JADREN/blob/main/examples/lesson-calculations.jdn).
Change `add` to `multiply`, update the expected result, then run `check` and
`run` again. A non-zero `main` result becomes the native process exit code.

## Lesson 2: create a native window tree

Jadren desktop UI is retained. The program creates a root window, adds child
nodes, closes containers in reverse order, then starts the native backend:

```jadren
let root: Int32 = ui_app_begin("My first window", 640, 420, 0xF6F8FCu32)
let panel: Int32 = ui_app_panel(root, 560, 260, 0xFFFFFFu32, 16, 12, 8, 0, 1)
let title: Int32 = ui_app_label(panel, "Hello from Jadren", 480, 34,
    0x111827u32, 0xFFFFFFu32, 8, 1)
let button: Int32 = ui_app_button(panel, "Click me", 1, 180, 42,
    0xFFFFFFu32, 0x168EF5u32, 12, 0)

if !ui_app_end(panel) { return 2 }
if !ui_app_end(root) { return 3 }
return ui_app_run()
```

Colours are explicit `UInt32` values in `0xRRGGBB` form. Node IDs identify
the parent of the next control and must not be guessed. Build and run the
complete fixture on Windows:
[lesson-first-window.jdn](https://github.com/RHsoft2012/JADREN/blob/main/examples/lesson-first-window.jdn).

## Lesson 3: connect a visible action to Jadren code

The native host delivers a button event ID to an exported callback. Keep the
callback short and put program behaviour in normal functions:

```jadren
@export(name: "jadren_ui_on_click", abi: "C")
pub fn on_click(event_id: Int32) -> Int32 {
    if event_id == 1 {
        ui_set_status("The event reached Jadren.")
    }
    return event_id
}
```

The complete calculator-window example shows the callback, the UI tree and
safe teardown together:
[lesson-calculator-window.jdn](https://github.com/RHsoft2012/JADREN/blob/main/examples/lesson-calculator-window.jdn).
The UI remains visible and directly owned by the native Jadren process; it is
not a hidden background service or browser-hosted application.

## Lesson 4: read and validate text input

Input is caller-owned. Provide a bounded byte buffer, read its valid prefix,
parse it, and update application state only after parsing succeeds:

```jadren
var text: [UInt8; 16] = [0u8, 0u8, 0u8, 0u8,
    0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
    0u8, 0u8, 0u8, 0u8]
var value: [Int64; 1] = [0 as Int64]
let length: UIntSize = ui_input_read(1, text)
if parse_int(text, length, value) {
    app_state_set_int("left", value[0])
    ui_set_status("The left value is valid.")
} else {
    ui_set_status("Enter a signed whole number.")
}
```

Run the complete two-input calculator fixture:
[lesson-calculator-input.jdn](https://github.com/RHsoft2012/JADREN/blob/main/examples/lesson-calculator-input.jdn).
Try an invalid value and confirm that the program reports it without replacing
the last valid state.

## Continue with an application

After the calculator works, extend the same model deliberately:

1. Keep form values in bounded `app_state` slots.
2. Add a bounded dynamic list for recent calculations.
3. Store structured rows in a typed table.
4. Save through an explicit atomic checkpoint.
5. Send or serve HTTP only through caller-owned request and response buffers.

The public [Language overview](LANGUAGE_OVERVIEW.md) explains the ownership
model behind these steps. The public [Compiler and platforms]
(COMPILER_AND_PLATFORMS.md) describes which native paths are currently
validated. For the full visual learning path, start from
[Jadren Lessons](https://jadren.rhsoft.eu/lessons.html).

## Practice checklist

- Change the calculation and its expected result.
- Add a second button with a separate event ID.
- Reject invalid input while retaining the old valid model value.
- Build the program and run the generated executable outside the compiler
  command.

This course intentionally does not add a database, hidden allocation, browser
bridge, implicit network call, TLS policy, or credential storage. Those are
separate, explicit application decisions.

When this four-step course is complete, continue with the [28-lesson learning path](LEARNING_PATH.md) for the full language, desktop, data, HTTP, Unity, and capability-gated GPU route.
