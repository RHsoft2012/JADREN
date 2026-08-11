# Jadren 28-lesson learning path

This is the public map of the Jadren learning path. It turns the internal
learning index into a reviewable English course: every lesson has a clear goal,
the next public reference, and a runnable example when the preview currently
has one. The examples are deliberately small and can be checked with the
Jadren CLI from the repository root.

## 01. Install the toolchain

Install the pinned CLI and verify the compiler before changing a project.
Read [Getting started](GETTING_STARTED.md).

## 02. Write the first program

Create a module, return an explicit process result, and run the first check.
Example: [lesson-calculations.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/lesson-calculations.jdn).

## 03. Follow the practical beginner course

Move from a calculation to a native window, a visible callback, and validated
text input. Start with the [practical beginner course](BEGINNER_PRACTICAL_COURSE.md).
Examples: [lesson-calculations.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/lesson-calculations.jdn), [lesson-first-window.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/lesson-first-window.jdn), [lesson-calculator-window.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/lesson-calculator-window.jdn), and [lesson-calculator-input.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/lesson-calculator-input.jdn).

## 04. Learn variables and types

Use explicit integer, floating-point, boolean, string, buffer, and slice types.
Example: [vector-slice.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/vector-slice.jdn).

## 05. Control the flow

Use conditions, loops, break, and continue without hiding the resulting state.
Example: [if-local-branch.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/if-local-branch.jdn).

## 06. Define functions and results

Give every function a visible parameter and result contract, then compose small
functions into a program. Example: [lesson-calculations.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/lesson-calculations.jdn).

## 07. Use buffers and slices

Pass caller-owned memory with explicit read and write capabilities.
Example: [stdlib-buffer-list.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/stdlib-buffer-list.jdn).

## 08. Understand ownership and moves

Keep ownership, borrowing, and cleanup visible at each boundary.
Example: [stdlib-buffer-owned-string.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/stdlib-buffer-owned-string.jdn).

## 09. Connect FFI and Unity

Export a narrow C ABI and let the host retain ownership of external objects.
Examples: [ffi-export.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/ffi-export.jdn) and [unity-command-stream.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/unity-command-stream.jdn).

## 10. Add SIMD and safe parallel work

Keep one algorithm and select a validated native capability explicitly.
Examples: [vector-slice.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/vector-slice.jdn) and [native-add.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/native-add.jdn).

## 11. Build a project from an empty directory

Split a program into modules, a manifest, and a reproducible project layout.
Example: [multi-file-app](https://github.com/RHsoft2012/JADREN/tree/agent/beginner-course-publish/examples/multi-file-app).

## 12. Run the Unity workflow

Install the matching packages, connect the native boundary, and verify a small
scene before expanding it. Example: [unity-command-stream.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/unity-command-stream.jdn).

## 13. Measure performance and portability

Compare the scalar baseline with a capability that is actually available.
Examples: [native-add.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/native-add.jdn) and [animation-batch.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/animation-batch.jdn).

## 14. Diagnose failures

Use compiler diagnostics, compile-fail fixtures, and the debugger smoke path to
separate a language error from a platform capability boundary.
Example: [debugger-smoke.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/debugger-smoke.jdn).

## 15. Use the language reference

Keep the syntax and ownership rules close while writing a larger example. Read
the [language overview](LANGUAGE_OVERVIEW.md).

## 16. Use the core API contracts

Learn the standard library contracts instead of adding hidden allocation or
implicit global state. Example: [import_math.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/stdlib/core/examples/import_math.jdn).

## 17. Search the API index

Use the public reference pages and the typed examples to find the exact function
shape before composing a call. Example: [app-table-sort.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/app-table-sort.jdn).

## 18. Follow the compiler pipeline

Trace source through the frontend and intermediate representations before
debugging a backend result. Example: [stage2_direct_call_zero_argument_executable.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/selfhost/stage2_direct_call_zero_argument_executable.jdn).

## 19. Prepare a first release

Learn how a versioned preview, manifest, and export are checked together.
Example: [release-export.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/release-export.jdn).

## 20. Move from C# in Unity

Keep the C# host thin and pass only explicit, validated ABI data.
Example: [ffi-export.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/ffi-export.jdn).

## 21. Use the complete practical guide

Combine language, native UI, data, file, and network boundaries into one
application-shaped exercise. Example: [time-tracker-pro-project](https://github.com/RHsoft2012/JADREN/tree/agent/beginner-course-publish/examples/time-tracker-pro-project).

## 22. Use the cookbook and FAQ

Start from a small recipe, then inspect its safety and capability boundary.
Example: [csv-export.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/csv-export.jdn).

## 23. Build a native Windows UI

Create a retained window, controls, colors, callbacks, and explicit teardown.
Example: [windows-desktop-app-binding.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/windows-desktop-app-binding.jdn).

## 24. Persist application data

Separate application state, dynamic lists, typed tables, transactions, and
atomic checkpoints. Examples: [app-state-persistence.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/app-state-persistence.jdn) and [app-table-query.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/app-table-query.jdn).

## 25. Build an HTTP client and server

Parse bounded messages, route explicit requests, and keep authentication and
TLS as visible contracts. Examples: [http-client.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/http-client.jdn) and [http-server.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/http-server.jdn).

## 26. Install the Unity plugin

Use the matching preview package and follow the documented host and ownership
rules. Start with [Unity integration](UNITY_INTEGRATION.md).

## 27. Run a Unity batch kernel

Use one caller-owned data boundary for a deterministic batch update.
Example: [animation-batch.jdn](https://github.com/RHsoft2012/JADREN/blob/agent/beginner-course-publish/examples/animation-batch.jdn). Unity execution remains capability-gated by the host package and device.

## 28. Plan GPU dispatch with a CPU fallback

Select GPU work only when the documented capability gate passes and keep the
CPU fallback explicit. Read [Compiler and platforms](COMPILER_AND_PLATFORMS.md).
This preview does not claim a universal GPU device or automatic dispatch.

## How to use this course

For each lesson, open the linked example, run `check`, read the expected
boundary, and only then move to the next item. The [Jadren Lessons](https://jadren.rhsoft.eu/lessons.html) page provides the visual route; this page is the complete 28-item map.
