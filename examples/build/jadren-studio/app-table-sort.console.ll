target triple = "x86_64-pc-windows-msvc"

%JadrenString = type { ptr, i64 }

@jadren.newline = private unnamed_addr constant [2 x i8] c"\0D\0A", align 1

declare dllimport ptr @GetStdHandle(i32)
declare dllimport i32 @WriteFile(ptr, ptr, i32, ptr, ptr)
declare dllimport i32 @ReadFile(ptr, ptr, i32, ptr, ptr)

define internal i64 @jadren.read_stream(i32 %stream, ptr %data, i64 %length) {
entry:
  %empty = icmp eq i64 %length, 0
  br i1 %empty, label %done_empty, label %start
start:
  %handle = call ptr @GetStdHandle(i32 %stream)
  %total_slot = alloca i64, align 8
  %count_slot = alloca i32, align 4
  store i64 0, ptr %total_slot, align 8
  br label %loop
loop:
  %total = load i64, ptr %total_slot, align 8
  %remaining = sub i64 %length, %total
  %too_large = icmp ugt i64 %remaining, 4294967295
  %chunk64 = select i1 %too_large, i64 4294967295, i64 %remaining
  %chunk = trunc i64 %chunk64 to i32
  %cursor = getelementptr i8, ptr %data, i64 %total
  store i32 0, ptr %count_slot, align 4
  %ok = call i32 @ReadFile(ptr %handle, ptr %cursor, i32 %chunk, ptr %count_slot, ptr null)
  %count32 = load i32, ptr %count_slot, align 4
  %count = zext i32 %count32 to i64
  %next = add i64 %total, %count
  store i64 %next, ptr %total_slot, align 8
  %failed = icmp eq i32 %ok, 0
  %empty_read = icmp eq i32 %count32, 0
  %stop = or i1 %failed, %empty_read
  %complete = icmp uge i64 %next, %length
  br i1 %stop, label %done, label %check_complete
check_complete:
  br i1 %complete, label %done, label %loop
done:
  %result = load i64, ptr %total_slot, align 8
  ret i64 %result
done_empty:
  ret i64 0
}

define internal i64 @jadren.write_stream(i32 %stream, ptr %data, i64 %length) {
entry:
  %empty = icmp eq i64 %length, 0
  br i1 %empty, label %done_empty, label %start
start:
  %handle = call ptr @GetStdHandle(i32 %stream)
  %total_slot = alloca i64, align 8
  %count_slot = alloca i32, align 4
  store i64 0, ptr %total_slot, align 8
  br label %loop
loop:
  %total = load i64, ptr %total_slot, align 8
  %remaining = sub i64 %length, %total
  %too_large = icmp ugt i64 %remaining, 4294967295
  %chunk64 = select i1 %too_large, i64 4294967295, i64 %remaining
  %chunk = trunc i64 %chunk64 to i32
  %cursor = getelementptr i8, ptr %data, i64 %total
  store i32 0, ptr %count_slot, align 4
  %ok = call i32 @WriteFile(ptr %handle, ptr %cursor, i32 %chunk, ptr %count_slot, ptr null)
  %count32 = load i32, ptr %count_slot, align 4
  %count = zext i32 %count32 to i64
  %next = add i64 %total, %count
  store i64 %next, ptr %total_slot, align 8
  %failed = icmp eq i32 %ok, 0
  %empty_write = icmp eq i32 %count32, 0
  %stop = or i1 %failed, %empty_write
  %complete = icmp uge i64 %next, %length
  br i1 %stop, label %done, label %check_complete
check_complete:
  br i1 %complete, label %done, label %loop
done:
  %result = load i64, ptr %total_slot, align 8
  ret i64 %result
done_empty:
  ret i64 0
}

define i64 @stdin_read(ptr %data, i64 %length) {
entry:
  %result = call i64 @jadren.read_stream(i32 -10, ptr %data, i64 %length)
  ret i64 %result
}

define i64 @stdout_write(ptr %data, i64 %length) {
entry:
  %result = call i64 @jadren.write_stream(i32 -11, ptr %data, i64 %length)
  ret i64 %result
}

define i64 @stderr_write(ptr %data, i64 %length) {
entry:
  %result = call i64 @jadren.write_stream(i32 -12, ptr %data, i64 %length)
  ret i64 %result
}

define void @print(%JadrenString %value) {
entry:
  %data = extractvalue %JadrenString %value, 0
  %length64 = extractvalue %JadrenString %value, 1
  %ignored_data = call i64 @stdout_write(ptr %data, i64 %length64)
  %handle = call ptr @GetStdHandle(i32 -11)
  %written = alloca i32, align 4
  %newline = call i32 @WriteFile(ptr %handle, ptr @jadren.newline, i32 2, ptr %written, ptr null)
  ret void
}
