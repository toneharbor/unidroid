# Debugging unidroid

Debugging reference for the [unidroid library and command](README.md).

## Library

### Deterministic runs

Build fresh emulators with the same inputs and these settings to compare guest memory with `snapshot().diff(...)` and block addresses and counts with `coverage()`:

```rust
let emu = Emulator::builder()
    .fixed_clock(1_700_000_000)
    .random_seed(42)
    .trace(Trace { coverage: vec!["libfoo.so".into()], ..Trace::default() })
    .build()?;
```

The address layout, mmap/brk/host-malloc allocation, thread switching by executed code bytes and futex virtual time are already deterministic. `fixed_clock(unix_seconds: i64)` gives all guest clock reads one shared elapsed counter, initially zero. `clock_gettime` realtime and `gettimeofday` return the supplied Unix time plus elapsed time; monotonic starts at one second plus elapsed time. Each read then adds one microsecond. Futex virtual time stays separate.

With `fixed_clock`, `stat` and `fstat` report all three timestamps as the supplied Unix seconds with zero nanoseconds, device/uid/gid as zero, and blocks as size rounded up to 512-byte units. The inode is the 64-bit FNV-1a hash of the original guest path, including its spelling; ARM32's additional 32-bit inode field gets the low 32 bits. An open file retains that path for `fstat`. Size, mode, nlink, rdev and blksize keep their usual values.

`random_seed(seed: u64)` supplies a SplitMix64 byte stream to `getrandom`, `/dev/urandom` and `/dev/random`. Successive outputs are consumed as little-endian bytes across all reads, so splitting a read does not change the stream on any platform. The options are independent; omitted options keep their host behavior. The CLI equivalents are `--fixed-clock SECONDS` and `--random-seed N`. Replay always serves recorded clock/random values before consulting either setting.

Other inputs can still make two runs differ:

- Answers and memory changes from your `JniHandler`, policy or serve driver, including debugger responses. Policy `values` positions carry across calls and new handler wrappers; parse or load a fresh policy for a fresh run.
- Rootfs and library file contents, paths and metadata that remains unchanged: permissions (`mode`), hard-link counts (`nlink`) and device identifiers (`rdev`) can differ between hosts.
- Thread interleaving and coverage counts depend on translation-block boundaries. Breakpoints, stepping, watchpoints, code traces and coverage itself can split blocks. Compare with the same debugger and trace settings, emulator/Unicorn version and translation-cache size.
- Prior calls and setup choices, such as process name, properties, aliases and load order. Clock ticks, random bytes, allocations and guest state carry across calls; use fresh emulators with the same setup and call sequence.

### Checkpoints and rewind

After loading libraries, running `JNI_OnLoad` and creating objects, save a reusable prepared state with `emu.checkpoint() -> Result<Checkpoint>`. `Checkpoint` is an opaque in-memory value exported from the crate root; dropping it frees the saved state.

```rust
let input = emu.new_cstring("first")?;
let prepared = emu.checkpoint()?;
emu.call_function(function, &[input])?;
emu.rewind(&prepared)?;
emu.write_bytes(input, b"other\0")?;
emu.call_function(function, &[input])?;
```

Load a library before taking a checkpoint so the CPU architecture is known. `checkpoint` and `rewind` work only between calls. A JNI handler, debugger stop or nested call gets `checkpoint and rewind work only between calls`; a suspended call is still running.

Rewind restores:

- All CPU registers, the current thread identity, saved guest threads and their registers, PCs and waits, thread IDs and virtual scheduling time.
- Memory mappings, permissions and bytes, including allocations, stacks, TLS and library data. Later mappings and libraries disappear; later host allocations and JNI handles are no longer live, and their addresses and handle numbers can be reused.
- Loaded modules and loader state, host function stubs, symbol aliases, library directories, rootfs, properties, process name, host malloc/free state and `dlerror` state.
- Open files and their offsets. Files reopen and seek on each rewind, so reads in one run or copy never move the checkpoint's offset. Keep the rootfs files available and unchanged.
- The fixed clock and its elapsed counter, and the seeded random generator including partly consumed output. Host clock/random values still come from the host when those settings were omitted.
- JNI handles, objects, classes, members, registered natives, references and exceptions. Object graphs are copied deeply, preserving aliases and cycles, including inside object arrays. `new_object`'s `Object::User` payload (`Rc<dyn Any>`) is shared: changes inside that Rust payload remain visible across rewinds and copies.

Breakpoints, watchpoints, trace settings, coverage counts, the event handler, translation-cache size and the JNI crash-history ring stay with the target emulator. Reset coverage explicitly with `reset_coverage()` when comparing runs. Trace selections are reapplied when the module list changes. Changed mappings and pages lose their translated code; unchanged pages in matching mappings keep it warm. Checkpoints do not use Unicorn's built-in memory snapshot mode, so guest `mprotect`, `munmap` and thread switching keep working.

Create an independent instance from the same checkpoint:

```rust
let mut copy = Emulator::builder().build()?;
copy.rewind(&prepared)?;
```

A fresh emulator creates its CPU for the checkpoint's architecture. An existing CPU must have the same architecture; a mismatch error names both. The copy keeps its own debugger, trace, event and cache settings. Shared read-only library mappings retain the same `Arc<Image>`; writable memory and other private mappings are copied. Each checkpoint and copy therefore costs about the writable bytes, plus private read-only pages, JNI objects, thread contexts and host bookkeeping. Copies remain usable after the original emulator and checkpoint are dropped.

Rust handlers and policies live outside the emulator. Save `policy.clone()` beside a checkpoint and restore that clone alongside `rewind`; cloning a `Policy` includes its current `values` sequence positions. Save any other handler state yourself.

In serve, `{"cmd":"checkpoint"}` returns a session-local numeric ID, starting at 1 and increasing without reuse. `{"cmd":"rewind","id":1}` restores it and returns `null`; `{"cmd":"drop_checkpoint","id":1}` frees it and returns `null`. Unknown or dropped IDs error with `no checkpoint N`. Serve also captures and restores its policy's sequence positions. These commands are recorded and replayed like ordinary commands. The recording journal and replay cursor always continue forward; rewinding guest state never rewinds either stream.

### Crash reports and locations

Guest failures carry a `Crash`. Its `Display` is the same error message; `report()` adds the thread, backtrace, registers, stack and recent JNI calls:

```rust
if let Err(error) = emu.call_function(f, &[]) {
    if let Some(crash) = error.downcast_ref::<unidroid::Crash>() {
        eprintln!("{}", crash.report());
    }
}
```

`Crash` has `message`, `thread`, `registers` (name/value pairs), `backtrace`, `stack` and `jni`. The stack holds up to 32 `(address, value)` pairs of pointer-size words starting at the crash's sp, stopping at the first unreadable word. `report()` prints them after the registers, four words per address-labelled line. Each `Frame` has `address`, `location` and an optional `symbol`. Both types serialize with serde. A crash keeps the state at the failure, including failures in guest threads or nested calls. Host function failures start at the guest caller. `emu.registers()?` and `emu.backtrace()?` read the current state. The backtrace follows frame pointers, skips repeated neighbors and stops at 32 frames or a zero return address. Code without frame pointers may give a short backtrace; look for app return addresses in `crash.stack`. The stack window is read only while building a crash.

`jni: Vec<String>` keeps the last 16 JNIEnv calls from any guest thread, oldest first, including the failing call and calls absent from the JNI trace. It is recorded even with tracing off: a fixed-size ring stores plain function indices, caller addresses, thread IDs, member IDs and three raw argument words, with no allocation or formatting while recording. Strings are built only for a crash. Each line names the function and `location(lr)`. Lookup, string and class-handle calls add the JNI trace's details in parentheses; member calls keep ` {class}->{name}{signature}`. Details are read when the crash is built, so changed guest strings show their current contents. Unreadable details are omitted, as are details containing only raw argument words. Calls from another thread append ` in thread N`. `report()` prints these lines after the stack, without a header:

```text
JNIEnv->FindClass(java/lang/Void) from libjnidispatch.so+0x8b64
JNIEnv->NewGlobalRef(java/lang/Void handle=0x19, arg2=0x100e0, arg3=0x107b0) from libjnidispatch.so+0x8b80
JNIEnv->GetStaticFieldID(java/lang/Void->TYPELjava/lang/Class;) from libjnidispatch.so+0x8bb4
JNIEnv->GetStaticObjectField java/lang/Void->TYPELjava/lang/Class; from libjnidispatch.so+0x8bc4
```

`emu.location(address)` stays `libfoo.so+0x1234`. `emu.symbolize(address)` adds a covering function name, like `libc.so+0x1f2a4 (strlen+0x10)`. It uses function symbols with a nonzero size from `.dynsym` and `.symtab`, when present. The symbol table is built on first use and shared between instances.

### Breakpoints, watchpoints and stops

`emu.add_breakpoint(address)?` stops before the instruction at that address. `emu.remove_breakpoint(address)?` removes it and errors if none is there. Both clear the Thumb bit; adding the same address twice keeps one breakpoint. Breakpoints also work on code that already ran and in any guest thread. With no breakpoints, watchpoints, step or finish active, no debugger hook is installed.

A Thumb `IT` instruction and the instructions it makes conditional run as one step. A breakpoint hit inside an `IT` block is reported at the first instruction after the block, or at the next Unicorn stop if the block branches away; a conditional instruction that is skipped does not hit a breakpoint. Watchpoint hits follow the same rule: `Stop.address` is where execution really stopped, while `WatchHit.pc` stays the instruction that accessed memory.

`emu.add_watchpoint(address, size, Watch::Write, WatchAction::Stop)?` watches writes to `[address, address + size)`; use `Watch::Read` for reads or `Watch::Access` for either. Size zero errors with `empty watchpoint`. Adding the same start replaces its size, kind and action. `emu.remove_watchpoint(address)?` removes that start or errors with `no watchpoint at <symbolize(address)>`. Only guest instructions trigger watches: `write_bytes`, serve's `write`, and host JNI functions such as `SetByteArrayRegion` do not. A `WatchAction::Stop` hit stops after the accessing instruction completes and before the next runs, subject to the Thumb IT rule above. If one instruction makes several watched accesses, only the first is reported. Watches work in guest threads and nested calls. Their memory and instruction hooks exist only while a watchpoint exists; removing the last also flushes translated code.

`WatchAction::Log` emits `Event::Watch` at the same hit boundary and resumes as `Resume::Continue` would, without calling `stopped`. A breakpoint on the next instruction still stops. The event has `at: String` (`symbolize(pc)`), `pc: u64` (accessing instruction, Thumb bit cleared), `address: u64`, `size: usize`, `write: bool`, `value: u64`, `thread: u32`, and `callers: Vec<String>` (symbolized backtrace frames after the first, read at the hit). Text uses `read` or `write`; it omits ` from ...` when callers are empty:

```text
watch write 0x40001000:4=0x2a at libfoo.so+0x1234 (f+0x10) thread 1234 from libfoo.so+0x2000 (g+0x8) <- libfoo.so+0x3000 (h+0x4)
```

JSON uses the `watch` event tag, hex strings for `pc`, `address` and `value`, numbers for `size` and `thread`, and a boolean for `write`:

```json
{"event":"watch","at":"libfoo.so+0x1234 (f+0x10)","pc":"0x10001234","address":"0x40001000","size":4,"write":true,"value":"0x2a","thread":1234,"callers":["libfoo.so+0x2000 (g+0x8)","libfoo.so+0x3000 (h+0x4)"]}
```

Stops go to the running call's `JniHandler::stopped(&mut self, emu: &mut Emulator, stop: &Stop) -> Resume`. The default returns `Resume::Continue`, including for `()` and `Probe`. `Stop` has `reason: StopReason` (`Breakpoint`, `Watchpoint`, `Step` or `Finish`), `address: u64` (Thumb bit cleared), `thread: u32`, and `watch: Option<WatchHit>` (`Some` only for watchpoint stops). `WatchHit` holds `pc: u64` (accessing instruction, Thumb bit cleared), `address: u64` (access start, possibly before the watched range), `size: usize`, `write: bool`, and `value: u64` (the written or read bits at access time). At a watch stop, writes have landed and loads have filled their destination registers; `Continue` resumes at `Stop.address` without skipping a breakpoint there. The handler can inspect or change registers and memory, or make a nested call that has stops of its own. Return one of:

| `Resume` | what happens |
|---|---|
| `Continue` | run on, skipping the current breakpoint hit once |
| `Step` | execute one guest instruction, then stop with `Step`; `bl` enters the callee, and entering a host function (JNI, dl or system property) runs that whole host function as one step |
| `Finish` | save lr and sp; stop with `Finish` in the same thread when `pc == (saved_lr & !1)` and `sp >= saved_sp`, including at the end of a top-level call |
| `Return(bits)` | return immediately to lr; set arm64 `x0`, or arm32 `r0` to the low 32 bits and `r1` to the high 32 bits |
| `Abort` | fail the running call with a `Crash` whose message is `stopped by the debugger at <symbolize(address)>` |

`Finish` relies on lr, so use it at a function's first instruction, before the function changes lr. Its stack check prevents stopping at a deeper recursive return. `Return(bits)` is also meant for the first instruction: it replaces the function's result without running the body. This is a function hook.

For a plain C call, use `emu.call_function_with(&mut handler, function, args)?` to receive stops and JNI questions in that handler. It takes the same arguments and returns the same result as `call_function`.

### Memory and registers

`emu.resolve(text)?` accepts `libfoo.so+0x1234`, an absolute hex address such as `0x50001000`, or an exported symbol name searched in module load order. Errors name the unknown module, bad number or unknown symbol. `emu.modules()` returns `Vec<ModuleInfo>` in load order: each entry has `name: String`, the host `path: PathBuf` recorded at load, `base: u64` and `size: u64`. Its index is the `Module` number.

`emu.maps()?` returns `Vec<Mapping>` sorted by start, one entry per current Unicorn region. Each entry has `start` and exclusive `end` addresses, `perms` such as `r-x` or `rw-`, and `name`. Names are checked in order: a loaded module's name, `[trampoline]`, `[host malloc]` (allocations from `emu.malloc` and `new_cstring`), `[mmap]`, `[heap]` (brk), `[stack]`, `[vectors]` (ARM32), or an empty string for an unnamed range. Use it to identify an address before reading memory.

`emu.search(pattern: &[u8])?` returns matching addresses in ascending order across all mapped memory, up to `SEARCH_LIMIT` (1000). Matches can overlap or span adjacent mappings. An empty pattern errors with `empty search pattern`. Each call reads the current memory mapping by mapping and keeps no copies afterward. A full list of 1000 means there may be more: narrow the pattern. Serve's `search` adds the same locations as snapshot diff, including library offsets and names such as `[host malloc]`.

`emu.crypto_constants()?` returns `CryptoConstant` entries for known hash, cipher, checksum and encoding constants found in mapped memory. Each entry has the name, address and a location such as `libfoo.so+0x1234` or `0x40001000 [host malloc]`; results are in address order. It reads memory only for the call and keeps no scan state. It finds tables stored as data, including tables built during initialization, but not values assembled by instructions such as `movz`/`movk` or `movw`/`movt`. A custom Base64 alphabet is not recognized.

`emu.snapshot()?` captures the current thread ID, general registers, modules and every mapped byte, including code and memory without read permission. `Snapshot::save(path)` writes a JSON header followed by raw memory; `Snapshot::load(path)` reads it back. `before.diff(&after)` returns changed registers, newly mapped and unmapped ranges, and changed bytes with locations. It compares by address even when mappings split, ignores permission changes, and merges byte changes separated by fewer than eight unchanged bytes. `diff.report()` formats all four sections as text:

```rust
let before = emu.snapshot()?;
before.save("before.snap")?;
emu.call_function(f, &[1, 2])?;
emu.snapshot()?.save("after.snap")?;
let after = unidroid::Snapshot::load("after.snap")?;
println!("{}", before.diff(&after).report());
```

Maps and snapshots install no hooks and store no copies until called. A snapshot holds all guest memory, including keys and inputs; keep snapshot files out of git. This repository ignores `*.snap`.

`emu.register(name)?` reads a register and `emu.set_register(name, value)?` writes it. Names are `x0`–`x28`, `fp`, `lr`, `sp`, `pc`, `nzcv` on arm64; `r0`–`r12`, `sp`, `lr`, `pc`, `cpsr` on arm32; and `d0`–`d31` on both. The `d` registers hold 64-bit floating-point bits and are available through named reads and writes, including serve's `set_register`. Dumps from `registers()`, crash reports, break questions and serve's `registers` list only the general registers, sp, lr, pc and flags, in the order above. An unknown name lists the valid names.

### Traces and events

`Trace` has three booleans (`jni`, `syscalls` and `imports`, default `false`) and three specification lists (`code: Vec<String>`, `calls: Vec<String>` and `coverage: Vec<String>`, default empty). Set them with `.trace(Trace { .. })` on the builder, or `emu.set_trace(Trace { .. })?` at runtime. `emu.trace()` reads the settings. Each change replaces all settings. With traces off, no trace hook is installed.

Each `code`, `calls` or `coverage` entry is a module file name, such as `libfoo.so`, or a module with a hex offset range, such as `libfoo.so+0x1000-0x2000` (end exclusive). It matches libraries already loaded and libraries loaded later, by file name. Bad specifications fail `build` or `set_trace`. For example, `Trace { code: vec!["libfoo.so+0x1000-0x2000".into()], ..Trace::default() }` traces that part of the library.

Block coverage uses `Trace { coverage: vec!["libfoo.so".into()], ..Trace::default() }` to count executed Unicorn translation blocks whose start lies in the selected ranges, across guest threads and nested calls. `emu.coverage() -> Vec<BlockCount>` returns entries sorted by `address: u64` (Thumb bit cleared), with `count: u64` and `location: String` computed by `symbolize` when read. Counts survive trace changes and turning coverage off; only `emu.reset_coverage()` clears them. Coverage allocates no state or hooks until a range is selected; turning it off removes its hooks and flushes translated code. A translation block is close to a basic block, but breakpoints, stepping and code traces can split it. Compare two inputs' address sets and hit counts with the same debugger and trace settings, resetting between runs.

Instruction events show the bytes in memory, Capstone disassembly, registers read before the instruction, registers written after it, and memory reads and writes in access order. Each memory access gives its address, byte count and value at access time. ARM32 follows the current ARM or Thumb mode, including Thumb conditional blocks. The text looks like this:

```text
[libfoo.so+0x1234] [e00740f9] ldr x0, [sp, #8] ; sp=0x701ffe00 => x0=0x7 | r 0x701ffe08:8=0x7
```

`Event::Instruction` has `at` (from `symbolize`), `address`, `bytes` (hex), `text`, `read`, `written` and `memory`. The register lists are name/value pairs (`Vec<(String, u128)>`, covering vector registers too). In JSON, `address` and register values are hex strings, such as `"address":"0x10001000"` and `[["r0","0x7"]]`, preserving full precision in JavaScript. Pairs stay in instruction order; the event tag is `instruction`. Events wait for the next instruction or a run stop so written values are available. A range exit also captures the result before untraced code changes it. Syscall results are captured after dispatch; a faulting instruction has an empty `written` array and keeps memory accesses reported before the fault. Stops before execution do not duplicate an instruction on resume. Turning code tracing off removes all its hooks and flushes translated code. If an instruction cannot be decoded or a register cannot be read, a `Warning` names its address and tracing continues. Capstone also names SVE/SME and legacy registers that Unicorn does not expose; those reads warn instead of inventing a value.

`memory` is `Vec<MemoryAccess>`, with public fields `write: bool`, `address: u64`, `size: usize` and `value: u64`. Paired and vector transfers appear as separate accesses of up to eight bytes. Text appends ` | ` and comma-separated `r 0xADDRESS:SIZE=0xVALUE` or `w 0xADDRESS:SIZE=0xVALUE`; an empty list adds nothing. JSON always includes `memory`, using `access: "read"` or `"write"`, hex strings for address and value, and a number for size:

```json
{"event":"instruction","at":"libfoo.so+0x1234","address":"0x10001234","bytes":"e00740f9","text":"ldr x0, [sp, #8]","read":[["sp","0x701ffe00"]],"written":[["x0","0x7"]],"memory":[{"access":"read","address":"0x701ffe08","size":8,"value":"0x7"}]}
```

Memory capture is always included in code tracing. Only accesses from a pending traced instruction are recorded; host memory operations and untraced instructions add no accesses.

Function call tracing uses `Trace { calls: vec!["libfoo.so+0x1000-0x2000".into()], ..Trace::default() }`. It reports executed `bl`/`blr` on arm64 and `bl`/`blx` on ARM32, including ARM and Thumb mode changes. The call site must be inside the selected range; the target can be anywhere, including a host trampoline. Plain branches and tail calls (`b`, `br`, `bx`) are excluded. `Event::FunctionCall` is emitted once as the target's first instruction is about to run. It has `to` and `from` from `symbolize` (`from` names the call instruction), `address: u64` for the target with the Thumb bit cleared, and `args: Vec<String>` for the first four argument registers as hex strings. JSON uses the tag `function_call` and a hex string for `address`. Code and call tracing work independently or together; removing call tracing removes its hooks, releases its state and flushes translated code.

`--trace-code SPEC` and `--trace-calls SPEC` are repeatable options on `serve`, `inspect` and `replay`. For example, `unidroid inspect --trace-calls libfoo.so libfoo.so` can print:

```text
function libfoo.so+0x1800 (mix)(0x7, 0x8, 0x40001000, 0xa) from libfoo.so+0x1234 (sign+0x10)
```

Syscall events show the guest number, arguments, signed result, errno name, instruction location and caller. Import events show calls from app libraries into other modules, including dl, property and unresolved-symbol stubs. The previous executed block in the same guest thread must be in an app library; app PLT stubs count, but internal tail calls within a dependency do not. The caller location comes from lr. They show the first four argument registers, with known C strings quoted and capped at 256 bytes. Dependencies next to an app count as app libraries; dependencies found in rootfs do not.

All diagnostics go through `Event`: `Jni`, `JniCall`, `Syscall`, `Call`, `FunctionCall`, `Instruction`, `Watch` or `Warning`. By default they print their text form on stderr. Capture them with the builder's `on_event`:

```rust
let emu = Emulator::builder()
    .trace(Trace { syscalls: true, ..Trace::default() })
    .on_event(|event| eprintln!("{}", serde_json::to_string(event).unwrap()))
    .build()?;
```

Events serialize as objects tagged with `event`: `jni`, `jni_call`, `syscall`, `call`, `function_call`, `instruction`, `watch` or `warning`. Arguments are arrays of text; JNI member arguments keep their typed debug form, such as `Object(Handle(7))`. Syscall numbers and returned results are JSON numbers. If dispatch fails, ends the thread or process, or switches away from the thread, `result` is `null` and `errno` is absent. Its text line ends with `= (no return)`. The event is emitted before any crash propagates. Otherwise `errno` is present only for a Linux error result. JNI locations keep the existing text; syscall and import locations include symbols.

`Probe::default()` is a `JniHandler` that answers calls with null, zero, false or void. `is_assignable` answers false. Pass it to `call_jni_onload` to explore calls without writing a handler. `probe.calls()` returns `&[ProbedCall]` in first-seen order. Each entry has `op`, `is_static`, `class`, `name`, `signature`, `count` and the first `from` location. For `is_assignable`, `class` is the source and `name` the target class. Defaults can make the library return early or crash.

## Serve

For the basic protocol, calling and object commands, see [README.md](README.md#serve-drive-it-from-your-program).

### Crash responses

A guest failure adds `crash` to the error response. Other errors still have only `error`. Inside `crash`, frame addresses, register values, and both entries of each `stack` pair are decimal strings; the thread ID is a number. `stack` keeps up to 32 pointer-size words as `[address, value]` pairs starting at the crash's sp, in address order. `registers` is an object keyed by register name, so `crash.registers.pc` reads the program counter. `jni` is the same list of recent JNIEnv strings as the library's `Crash::jni`, present even with tracing off. For example:

```json
{"error":"unhandled JNI call: ...","crash":{"message":"unhandled JNI call: ...","thread":1234,"registers":{"pc":"65820","x0":"7"},"backtrace":[{"address":"268471236","location":"libjnidispatch.so+0x8bc4","symbol":"JNI_OnLoad+0x3c0"}],"stack":[["1879048160","0"],["1879048168","268471236"]],"jni":["JNIEnv->FindClass(java/lang/Void) from libjnidispatch.so+0x8b64","JNIEnv->GetStaticFieldID(java/lang/Void->TYPELjava/lang/Class;) from libjnidispatch.so+0x8bb4","JNIEnv->GetStaticObjectField java/lang/Void->TYPELjava/lang/Class; from libjnidispatch.so+0x8bc4"]}}
```

The example abbreviates the registers, stack and JNI history. If a bad JNI answer replaces the command's error message, `crash` still holds the original guest failure.

### Traces

Change traces during a session with `{"cmd":"trace","jni":true,"syscalls":true,"imports":false,"code":["libfoo.so+0x1000-0x2000"],"calls":["libfoo.so"],"coverage":["libfoo.so"]}`. Omitted booleans become `false`; omitted `code`, `calls` and `coverage` become empty. `{"cmd":"trace"}` turns all traces off. Log format is chosen when starting the CLI; JSON logs go to stderr and protocol replies go to stdout.

### Stops and questions

Use `{"cmd":"watch","at":"0x40001000","size":4,"kind":"write","log":true}` to record hits throughout a call. Log hits go to stderr as text or JSON according to `--log-format`, just like traces, and never ask a `break` question. Omitting `log` or setting it to `false` selects stop mode.

Debugger stops arrive as `{"break": ...}` questions. The fields are `reason` (`breakpoint`, `watchpoint`, `step` or `finish`), `address` (decimal string, Thumb bit cleared), `location` (from `symbolize`), `thread` (number), and `registers` (the same object, in register table order, as `registers` and a crash). Watchpoint stops also have `watch: {pc, location, address, size, write, value}`: `pc`, `address` and `value` are decimal strings, `size` is a number, `write` is a boolean, and `location` is the `symbolize` text of `pc`. Other stops omit `watch`. While stopped, any command can run, including a nested `call_function`, except `checkpoint` and `rewind`, which need the call to have ended (see [Checkpoints and rewind](DEBUGGING.md#checkpoints-and-rewind)). Resume the innermost stop with `continue`, `step`, `finish`, `return` (with a decimal string `value`), or `abort`. These have the same behavior as Rust's `Resume`; they have no separate reply. The original command continues and ends with its own `ok` or `error`.

A short exchange after loading SignUtil and creating the arguments (register objects abbreviated; Java questions during `finish` are answered as in [README.md](README.md#serve-drive-it-from-your-program)):

```text
→ {"cmd":"break","at":"Java_com_anjuke_mobile_sign_SignUtil_getSign0"}
← {"ok":"268442540"}
→ {"cmd":"call_static","class":"com/anjuke/mobile/sign/SignUtil","method":"getSign0(Ljava/lang/String;Ljava/lang/String;Ljava/util/Map;Ljava/lang/String;I)Ljava/lang/String;","args":[7,8,9,10,10]}
← {"break":{"reason":"breakpoint","address":"268442540","location":"libsignutil.so+0x1bac (Java_com_anjuke_mobile_sign_SignUtil_getSign0)","thread":1234,"registers":{"pc":"268442540","lr":"131072"}}}
→ {"cmd":"registers"}
← {"ok":{"pc":"268442540","lr":"131072"}}
→ {"cmd":"finish"}
   ... answer JNI questions ...
← {"break":{"reason":"finish","address":"131072","location":"0x20000","thread":1234,"registers":{"pc":"131072","r0":"16"}}}
→ {"cmd":"continue"}
← {"ok":16}
```

For a routine at `0x20100` that stores its second argument through its first and returns `7`, find the buffer, then watch the next call (registers abbreviated):

```text
→ {"cmd":"search","base64":"YWE="}
← {"ok":[{"address":"1073745920","location":"0x40001000 [host malloc]"}]}
→ {"cmd":"watch","at":"0x40001000","size":4,"kind":"write"}
← {"ok":"1073745920"}
→ {"cmd":"call_function","address":"131328","args":["1073745920","42"]}
← {"break":{"reason":"watchpoint","address":"131332","location":"0x20104","thread":1234,"registers":{"pc":"131332"},"watch":{"pc":"131328","location":"0x20100","address":"1073745920","size":4,"write":true,"value":"42"}}}
→ {"cmd":"unwatch","at":"0x40001000"}
← {"ok":null}
→ {"cmd":"continue"}
← {"ok":"7"}
```

A resume command with no stop open errors with `no breakpoint stop is waiting`. An open stop expects a resume command; an open JNI question expects `answer` or `unhandled`. Sending the other kind gives an error and leaves the question open. With nested questions, answer or resume the innermost one first.

### Commands

| command | fields | result |
|---|---|---|
| `trace` | `jni`, `syscalls`, `imports` (booleans, omitted means false), `code`, `calls`, `coverage` (lists of specifications, omitted means empty) | `null` |
| `coverage` | `reset` (boolean, omitted means false) | list of `{address, location, count}` in address order; address is a decimal string, count is a number; `reset: true` returns the collected counts and then clears them |
| `break` / `unbreak` | `at` (text for `resolve`) | address with Thumb bit cleared / `null` |
| `watch` | `at` (text for `resolve`), `size` (number), `kind` (`read`, `write` or `access`), `log` (boolean, omitted or false means stop) | watched address (decimal string) |
| `unwatch` | `at` (text for `resolve`) | `null` |
| `search` | `base64` (nonempty byte pattern) | up to 1000 `{address, location}` matches in address order; address is a decimal string |
| `crypto` | | list of `{name, address, location}` constants; address is a decimal string |
| `resolve` | `at` (module + hex offset, absolute hex or exported symbol) | address (decimal string) |
| `registers` | | register object, values as decimal strings, in table order |
| `set_register` | `name`, `value` (decimal string) | `null` |
| `backtrace` | | frames, in the same JSON as a crash's `backtrace` |
| `modules` | | list of `{name, path, base, size}`, in load order; base and size are decimal strings |
| `maps` | | list of `{start, end, perms, name}`, sorted by start; start and exclusive end are decimal strings |
| `checkpoint` | | numeric checkpoint ID (1, 2, 3, ... per session); captures guest state and policy sequence positions between calls |
| `rewind` | `id` (number) | `null`; restores the checkpoint and policy positions between calls; unknown IDs error with `no checkpoint N` |
| `drop_checkpoint` | `id` (number) | `null`; frees the checkpoint; unknown IDs error with `no checkpoint N` |
| `snapshot` | `path` (relative to the serve process's directory) | `null`; saves all guest memory, registers and modules, between calls or while stopped |
| `continue` / `step` / `finish` / `abort` | | (resumes the open stop) |
| `return` | `value` (decimal string) | (returns from the stopped function) |

## The `unidroid` command

### Record a session and replay it

Start the service with `serve --record FILE`. stdin, stdout and JNI questions stay the same. Each recording entry is one JSON line, flushed immediately so completed entries survive a crash. A recording contains every input, including keys. Keep recordings out of git.

```bash
target/release/unidroid serve --rootfs rootfs --record session.jsonl
target/release/unidroid replay session.jsonl --trace-syscalls --trace-code libsignutil.so
```

The recording starts with `{"record":1,"version":"<crate version>","options":{...}}`. `options` holds `rootfs`, `process_name`, `prop`, `alias`, `superclass`, `fixed_clock`, `random_seed` and `translation_cache_mb`; the three repeatable settings are arrays of key/value pairs. Defaults are recorded too. Trace and log options are not in the header. Subsequent entries stay in consumption order:

With `--policy`, the header also holds `"policy":{"rules":[...]}`: the parsed rules, with their original `values` lists. The policy path is not stored. Replay uses these embedded rules from the first sequence value, even if the original file changed or was deleted. A recording made without a policy has no `policy` entry. See [README.md](README.md) for the policy format and API.

| entry | what it holds |
|---|---|
| `{"in":"<line>"}` | the input exactly as read, with its line ending removed |
| `{"out":<JSON value>}` | a response or question |
| `{"time":{"clock":"realtime","sec":123,"nsec":456}}` | a clock read; `clock` can also be `monotonic` |
| `{"random":"<base64>"}` | bytes read from `/dev/urandom` or `/dev/random` |

`getrandom` also uses `random` entries and serves them from the recording during replay. Fixed clock and seeded random values are recorded in the same consumption order as host values. Replay rebuilds `fixed_clock` from the header, so fixed file metadata matches too.

`replay FILE` rebuilds the recorded setup, feeds the inputs through the serve loop, and compares every response and question as JSON. Guest `clock_gettime`, `gettimeofday` and random-device reads use the recording instead of the host. Keep the same libraries and rootfs; relative rootfs and `load` paths resolve against the current directory, so run replay from the directory the service ran in.

Replaying a recording that took snapshots writes those snapshot files again, using paths relative to the replay process's directory.

Replay accepts only `--verbose`, `--trace-syscalls`, `--trace-imports`, repeatable `--trace-code` and `--trace-calls`, and `--log-format text|json`. It adds no breakpoints. Diagnostics go to stderr. Success prints `replayed <n> commands, all responses match` on stdout and exits 0; `n` counts input lines, including JNI answers. The first difference exits 1 with one stdout line naming the recording line: differing responses include expected and actual JSON; missing or extra responses, wrong time/random order, exhausted values and unused values also name the mismatch.

The library exposes `serve::Options` (the shared options and `build`), `serve::record(options, builder, file, input, output)` and `serve::replay(file, builder)`. Replay returns the input count; the builder supplies diagnostic settings. The clock/random source and recording entries remain crate-internal.

### `diff`: compare two memory snapshots

Take snapshots before and after a call with serve's `snapshot` command or the library API, then run:

```bash
target/release/unidroid diff before.snap after.snap
```

It prints the differences and exits 0, even when memory changed. For example:

```text
registers:
  x0 0x0 -> 0x7
mapped:
  (none)
unmapped:
  (none)
changes:
  0x40000000 [host malloc], 4 bytes
    0000 - 00 00 00 00
    0000 + 01 02 03 04
```
