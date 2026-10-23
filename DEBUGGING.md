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

The address layout, mmap/brk/host-malloc allocation, thread switching by executed code bytes and futex virtual time are already deterministic. `fixed_clock(unix_seconds: i64)` gives guest clock syscalls and arm64 direct counter reads one shared elapsed counter, initially zero. `clock_gettime` realtime and `gettimeofday` return the supplied Unix time plus elapsed time; monotonic starts at one second plus elapsed time. Each read then adds one microsecond. Futex virtual time stays separate. On arm64, `mrs` reads of `CNTVCT_EL0` and `CNTPCT_EL0` return monotonic time converted to ticks at the CPU's `CNTFRQ_EL0` frequency. Each counter read consumes the same one-microsecond clock tick and is recorded and replayed as a monotonic clock read. Other system registers keep Unicorn's behavior.

With `fixed_clock`, `stat` and `fstat` report all three timestamps as the supplied Unix seconds with zero nanoseconds, device/uid/gid as zero, and blocks as size rounded up to 512-byte units. The inode is the 64-bit FNV-1a hash of the original guest path, including its spelling; ARM32's additional 32-bit inode field gets the low 32 bits. An open file retains that path for `fstat`. Size, mode, nlink, rdev and blksize keep their usual values.

`random_seed(seed: u64)` supplies a SplitMix64 byte stream to `getrandom`, `/dev/urandom` and `/dev/random`. Successive outputs are consumed as little-endian bytes across all reads, so splitting a read does not change the stream on any platform. The options are independent; omitted options keep their host behavior. The CLI equivalents are `--fixed-clock SECONDS` and `--random-seed N`. Replay always serves recorded clock/random values before consulting either setting.

`getrandom` and reads of `/dev/urandom` or `/dev/random` return at most 33554431 bytes per call. Host-file `read` and `pread64` allocate at most the bytes left from the read position and return a short read at end of file.

Other inputs can still make two runs differ:

- ARM32 direct counter reads (`mrrc p15` CNTVCT) and, without `fixed_clock`, arm64 counter reads.
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
- Heap tracking records, including live blocks, freed blocks and release order. They follow the restored memory; `Trace.heap` and its hooks stay with the target emulator.
- Open files and their offsets. Files reopen and seek on each rewind, so reads in one run or copy never move the checkpoint's offset. Keep the rootfs files available and unchanged.
- The fixed clock and its elapsed counter, and the seeded random generator including partly consumed output. Host clock/random values still come from the host when those settings were omitted.
- JNI handles, objects, classes, members, registered natives, references and exceptions. Object graphs are copied deeply with an explicit work list, preserving aliases and cycles, including arbitrarily nested object arrays. `new_object`'s `Object::User` payload (`Rc<dyn Any>`) is shared: changes inside that Rust payload remain visible across rewinds and copies.

Breakpoints (including conditions), watchpoints, tracepoints and their hit counts, trace settings, coverage counts, the event handler and translation-cache size stay with the target emulator. Rewind clears the JNI crash-history ring because its calls belong to the discarded state. Reset coverage explicitly with `reset_coverage()` when comparing runs. Trace selections are reapplied when the module list changes. Changed mappings and pages lose their translated code; unchanged pages in matching mappings keep it warm. Checkpoints do not use Unicorn's built-in memory snapshot mode, so guest `mprotect`, `munmap` and thread switching keep working.

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

To stop inside a constructor, first call `let module = emu.load_library_without_constructors(path)?`, set the breakpoint, then call `emu.run_constructors(module, &mut handler)?`. Pending dependencies run depth first before the module's `DT_INIT` and `DT_INIT_ARRAY`. Stops reach `handler.stopped`, and hooked imports reach `handler.import`. Ordinary `load_library` uses `()` for constructors, so stops auto-continue and imports pass through to their originals.

`emu.add_breakpoint(address)?` stops before the instruction at that address. `emu.remove_breakpoint(address)?` removes it and errors if none is there. Both clear the Thumb bit; adding the same address twice keeps one breakpoint. Breakpoints also work on code that already ran and in any guest thread. With no breakpoints, watchpoints, tracepoints, step or finish active, no debugger hook is installed.

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

### Expressions, conditional breakpoints and tracepoints

Both features parse expressions once when added. Errors include the bad text; a rejected expression leaves the existing point unchanged. A failed tracepoint addition or replacement leaves all tracepoints and their counts unchanged. Load a library first so register names and pointer size are known.

| expression | value |
|---|---|
| `REG` | any name accepted by `emu.register`, including `x20`, `r4`, `sp`, `pc` and `d0` on the appropriate architecture |
| `0xHEX` | a 64-bit constant |
| `[EXPR]` | memory at that address, read as a pointer-size word (4 bytes on ARM32, 8 on arm64) |
| `[EXPR+0xHEX]` / `[EXPR-0xHEX]` | memory at that address plus/minus a constant; address arithmetic wraps at 64 bits |
| `[EXPR]:1`, `:2`, `:4` or `:8` | memory with an explicit byte width, little-endian and zero-extended to 64 bits |

Memory expressions can nest up to 16 brackets per operand; deeper nesting errors with `nesting deeper than 16`. For example, `[x20+0x18]` reads a pointer field and `[[x20+0x18]]:1` reads the byte it points to. Spaces around `+`, `-`, `==` and `!=` are accepted. Arithmetic outside memory brackets and comparisons other than equality/inequality are not supported.

`emu.add_conditional_breakpoint(address, "x20 == 0x40001000")?` stops with `StopReason::Breakpoint` only when the condition holds. A condition is `EXPR==EXPR` or `EXPR!=EXPR`; either unreadable operand makes it false, even for `!=`. Evaluation happens before the instruction. The Thumb IT stop rule above still applies. Plain and conditional breakpoints at the same address replace each other; `remove_breakpoint` removes either. `Continue` skips that hit once, not later loop iterations.

```rust
emu.add_tracepoint(dispatch, &unidroid::Tracepoint {
    values: vec!["[x20+0x18]".into(), "[[x20+0x18]]:1".into()],
    log: true,
})?;
let counts: Vec<(u64, u64)> = emu.tracepoint_hits(dispatch)?;
emu.reset_tracepoint_hits();
emu.remove_tracepoint(dispatch)?;
```

Tracepoints are independent of `Trace` settings; `set_trace` does not remove them. A tracepoint reads values before the instruction runs, in any guest thread or nested call. It never stops or calls `stopped`. Skipped Thumb IT instructions do not hit; taken ones capture their pre-instruction values even when a debugger stop is deferred. It works alongside code/call traces, coverage and watchpoints, keeping event order.

`Tracepoint` has `values: Vec<String>` and `log: bool`. At least one expression is required, otherwise the error is `tracepoint needs at least one value`. Every hit counts the first expression’s value, whether logged or not; an unreadable first value is not counted. `tracepoint_hits(address)` returns `(value, count)` pairs sorted by value. `reset_tracepoint_hits()` clears all tracepoints’ counts. Adding the same address replaces its expressions and log setting and clears its counts. Add, remove and query clear the Thumb bit. Removing or querying a missing point errors with `no tracepoint at <symbolize(address)>`. Removing the last point removes its hooks. Rewind leaves points and counts alone.

With `log: true`, each hit emits `Event::Tracepoint { at, address, thread, values }`: `at` is from `symbolize`, `address: u64` has the Thumb bit cleared, `thread: u32`, and `values: Vec<(String, Option<u64>)>` preserves the supplied expression text and order. Unreadable values are `None`. Text uses `=?` for them:

```text
tracepoint libfoo.so+0x1234 (f+0x10) thread 1234 [x20+0x18]=0x40001000 [[x20+0x18]]:1=0x2a
```

JSON uses hex strings for the address and values, and `null` for unreadable values:

```json
{"event":"tracepoint","at":"libfoo.so+0x1234 (f+0x10)","address":"0x10001234","thread":1234,"values":[["[x20+0x18]","0x40001000"],["[[x20+0x18]]:1","0x2a"]]}
```

### Bytecode interpreters

1. Find the native dispatch address using a code/call trace or a coverage hot spot. Identify the virtual PC register or memory field.
2. Add a tracepoint there with the virtual PC and opcode as values. For example, serve `{"cmd":"tracepoint","at":"libfoo.so+0x1234","values":["x20","[x20]:1"]}` gives a bytecode trace. `tracepoint_hits` gives per-virtual-PC coverage; use `"log":false` for counts only.
3. Break on one bytecode address with `{"cmd":"break","at":"libfoo.so+0x1234","if":"x20==0x40001000"}`.
4. To step one bytecode at a time, use a plain breakpoint on the dispatch address and `continue` at each stop. Each return to dispatch is the next bytecode, including loop iterations; CPU `step` instead executes one native instruction (or one Thumb IT block).

Bytecode-level step-over, virtual register files and source mapping are not provided.

### Memory and registers

`emu.resolve(text)?` accepts `libfoo.so+0x1234`, an absolute hex address such as `0x50001000`, or an exported symbol name searched in module load order. Errors name the unknown module, bad number or unknown symbol. `emu.modules()` returns `Vec<ModuleInfo>` in load order: each entry has `name: String`, the host `path: PathBuf` recorded at load, `base: u64` and `size: u64`. Its index is the `Module` number.

`emu.maps()?` returns `Vec<Mapping>` sorted by start, one entry per current Unicorn region. Each entry has `start` and exclusive `end` addresses, `perms` such as `r-x` or `rw-`, and `name`. Names are checked in order: a loaded module's name, `[trampoline]`, `[host malloc]` (allocations from `emu.malloc` and `new_cstring`), `[mmap]`, `[heap]` (brk), `[stack]`, `[vectors]` (ARM32), or an empty string for an unnamed range. Use it to identify an address before reading memory.

`emu.search(pattern: &[u8])?` returns matching addresses in ascending order across all mapped memory, up to `SEARCH_LIMIT` (1000). Matches can overlap or span adjacent mappings. An empty pattern errors with `empty search pattern`. Each call reads the current memory mapping by mapping and keeps no copies afterward. A full list of 1000 means there may be more: narrow the pattern. Serve's `search` adds the same locations as snapshot diff, including library offsets and names such as `[host malloc]`.

`emu.crypto_constants()?` returns `CryptoConstant` entries for known hash, cipher, checksum and encoding constants found in mapped memory. Each entry has the name, address and a location such as `libfoo.so+0x1234` or `0x40001000 [host malloc]`; results are in address order. It reads memory only for the call and keeps no scan state. It finds tables stored as data, including tables built during initialization, but not values assembled by instructions such as `movz`/`movk` or `movw`/`movt`. A custom Base64 alphabet is not recognized.

`emu.snapshot()?` captures the current thread ID, general registers, modules and every mapped byte, including code and memory without read permission. With `Trace.heap` enabled it also captures the live known allocations described under Heap and objects below. `Snapshot::save(path)` writes a JSON header followed by raw memory; `Snapshot::load(path)` reads it back. `before.diff(&after)` returns changed registers, newly mapped and unmapped ranges, and changed bytes with locations. It compares by address even when mappings split, ignores permission changes, and merges byte changes separated by fewer than eight unchanged bytes. `diff.report()` formats these sections as text, with allocation sections when heap data is present:

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

### Heap and objects

Enable tracking with the builder's `.trace(Trace { heap: true, ..Trace::default() })` or `emu.set_trace(Trace { heap: true, ..Trace::default() })?`. The switch defaults to false. Tracking hooks the exported `malloc`, `calloc`, `realloc`, `free`, `memalign` and `posix_memalign` entry points of the module named `libc.so`, before its constructors run, including when libc loads later. Static and custom allocators are not seen.

Each entry captures the function, arguments, thread and the same frame-pointer backtrace used by `backtrace()`, starting at the caller's return address (Thumb bit cleared). A call completes when that thread reaches its saved return address with sp at least its entry sp. Per-thread stacks pair nested calls; each distinct return address has one hook retained while tracking is on. Host-started guest allocations, including JNI string and array buffers, complete at the host return boundary. Their first frame is `0x20000` with symbol `host return`. Backtraces can be short when code omits frame pointers. Re-entering an entry after a debugger stop updates its arguments while keeping its original order and backtrace. Failed calls discard abandoned pending entries for both the thread that started the call and the thread running at the failure.

`emu.heap() -> Vec<Allocation>` returns completed libc allocation records in allocation-entry order, live and freed. Each `Allocation` has `address: u64`, `size: u64`, `live: bool`, `function: String`, `thread: u32`, `backtrace: Vec<Frame>` and `freed: Option<Vec<Frame>>`. Sizes are the requested byte counts; calloc uses count × size. `freed` holds the entry backtrace of the free or realloc that released the block. Releases and in-place reallocs apply to the record identified at entry, even if its address has since been reused. `free(NULL)` and failed allocations add no records. A successful realloc that moves frees the old record and creates one named `realloc`; an in-place realloc updates the existing size. Successful posix_memalign reads its pointer from the out-parameter after return; an unreadable out-parameter skips the record.

Turning tracking off removes all its hooks and keeps its records. They stop updating while it is off. A newly recorded allocation or an in-place realloc that grows retires every overlapping live record, keeping the resized record itself. Such a retired record has `live: false, freed: None`: memory reuse was observed, but its release was not. It remains in `heap()` history and is not a candidate for the most-recently-freed lookup. `emu.reset_heap()` clears the records. With tracking never enabled, there is no tracker state or hook. `emu.allocation(address) -> Option<Allocation>` finds the live tracked block containing the address, otherwise the most recently freed containing block, otherwise a current host allocation.

Host allocations from `emu.malloc` and `new_cstring` are known even with tracking off. Queries read them from the host allocator's existing map: `function` is `host`, `live` is true, `thread` is the main thread ID (1234), the backtrace is empty and `freed` is None. Their size is the host allocator's rounded block size. They carry no allocation history and are not in `heap()`; freeing one removes it from these queries.

`emu.object(address, depth) -> Result<HeapObject>` accepts any address inside a live tracked block or current host allocation. Depth must be at most 64; larger values error with `depth must be at most 64`, in both the library and serve's `object`. An address outside those allocations errors with `0x<address> is not inside a known allocation`, adding module or mapping context when available. The result describes the whole containing block: `address`, `size`, `allocation`, `fields` and `referrers`.

- Each `Field { offset, value, target, string }` represents a pointer-size aligned word; a trailing partial word is omitted. `Target::Block { address, offset, object }` names a known live block and the offset into it. `Target::Location { location }` names a module symbol or mapping. Other values have no target. `string` is set only for a pointer to printable ASCII (space through tilde), at least four characters, with a NUL terminator within the first 64 bytes. There is no structure or string-table detection.
- Each `Referrer { address, location, block, object }` is an aligned word whose value points anywhere into the block. All mapped memory is scanned, including the stack, excluding words in the block itself. Results are in address order, capped at `SEARCH_LIMIT` (1000) per object. `block: Option<u64>` names the known live block containing the referring word, when one exists.
- Depth 0 includes the root's fields, strings and referrer list. Depth 1 also expands directly pointed-to blocks and referrer blocks through their `object: Option<Box<HeapObject>>` fields. Each additional level follows one more link. A block expands at most once per call, including the root; later mentions retain its address and offset, with no nested object, so cycles terminate. Field targets are visited before referrers. Memory is read once per call, mapping by mapping, and the temporary copies are shared by all expansion and string reads.

With heap tracking on, `snapshot().heap` holds live tracked blocks and current host allocations. With it off, that list is empty even if retained tracker records exist. Saved snapshot headers preserve the list; headers without `heap` load with an empty list. Diff's `allocated` and `freed` lists compare live blocks by address and size: resizing in place appears in both lists, while freeing and reusing the same address and size between snapshots is indistinguishable. `Change.allocation: Option<(u64, u64)>` identifies the start and size of the block containing the change's first byte, checking the after snapshot first, then before. The text report adds `allocated:` and `freed:` after `unmapped:` when either snapshot has heap data, and appends ` in block 0x<start>+0x<offset>` to those change lines. The `unidroid diff` command uses this report automatically.

Checkpoint heap records follow guest memory: rewind restores live and freed records and their release order, including into a fresh emulator. `Trace.heap` and its hooks stay with the target emulator. Only completed records are checkpointed between calls; pending call stacks are not checkpoint data.

### Traces and events

`Trace` has four booleans (`jni`, `syscalls`, `imports` and `heap`, default `false`) and three specification lists (`code: Vec<String>`, `calls: Vec<String>` and `coverage: Vec<String>`, default empty). Set them with `.trace(Trace { .. })` on the builder, or `emu.set_trace(Trace { .. })?` at runtime. `emu.trace()` reads the settings. Each change replaces all settings. With traces off, no trace hook is installed.

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

Import hooks appear in import traces under their symbol name with module `<hook>`. Returning `None` can then enter the original export; it preserves the original call's stack and floating-point registers. Hooks and saved originals are restored by rewind, including into a fresh emulator.

All diagnostics go through `Event`: `Jni`, `JniCall`, `Syscall`, `Call`, `FunctionCall`, `Instruction`, `Watch`, `Tracepoint` or `Warning`. By default they print their text form on stderr. Capture them with the builder's `on_event`:

```rust
let emu = Emulator::builder()
    .trace(Trace { syscalls: true, ..Trace::default() })
    .on_event(|event| eprintln!("{}", serde_json::to_string(event).unwrap()))
    .build()?;
```

Events serialize as objects tagged with `event`: `jni`, `jni_call`, `syscall`, `call`, `function_call`, `instruction`, `watch`, `tracepoint` or `warning`. Arguments are arrays of text; JNI member arguments keep their typed debug form, such as `Object(Handle(7))`. Syscall numbers and returned results are JSON numbers. If dispatch fails, ends the thread or process, or switches away from the thread, `result` is `null` and `errno` is absent. Its text line ends with `= (no return)`. The event is emitted before any crash propagates. Otherwise `errno` is present only for a Linux error result. JNI locations keep the existing text; syscall and import locations include symbols.

`Probe::default()` is a `JniHandler` that answers calls with null, zero, false or void. `is_assignable` answers false. Pass it to `call_jni_onload` to explore calls without writing a handler. `probe.calls()` returns `&[ProbedCall]` in first-seen order. Each entry has `op`, `is_static`, `class`, `name`, `signature`, `count` and the first `from` location. For `is_assignable`, `class` is the source and `name` the target class. Defaults can make the library return early or crash.

## Serve

For the basic protocol, calling and object commands, see [README.md](README.md#serve-drive-it-from-your-program).

`unidroid batch` uses this protocol without a driver. Its jobs and setup refuse `break`, `unbreak`, `watch`, `unwatch`, `tracepoint`, `untracepoint`, `continue`, `step`, `finish`, `return`, `abort`, `answer`, `unhandled`, `checkpoint`, `rewind` and `drop_checkpoint`. Unanswered JNI calls fail immediately; traces, coverage, snapshots and crash responses remain available. See [README.md](README.md) for batch jobs and reports.

### Crash responses

A guest failure adds `crash` to the error response. Other errors still have only `error`. Inside `crash`, frame addresses, register values, and both entries of each `stack` pair are decimal strings; the thread ID is a number. `stack` keeps up to 32 pointer-size words as `[address, value]` pairs starting at the crash's sp, in address order. `registers` is an object keyed by register name, so `crash.registers.pc` reads the program counter. `jni` is the same list of recent JNIEnv strings as the library's `Crash::jni`, present even with tracing off. For example:

```json
{"error":"unhandled JNI call: ...","crash":{"message":"unhandled JNI call: ...","thread":1234,"registers":{"pc":"65820","x0":"7"},"backtrace":[{"address":"268471236","location":"libjnidispatch.so+0x8bc4","symbol":"JNI_OnLoad+0x3c0"}],"stack":[["1879048160","0"],["1879048168","268471236"]],"jni":["JNIEnv->FindClass(java/lang/Void) from libjnidispatch.so+0x8b64","JNIEnv->GetStaticFieldID(java/lang/Void->TYPELjava/lang/Class;) from libjnidispatch.so+0x8bb4","JNIEnv->GetStaticObjectField java/lang/Void->TYPELjava/lang/Class; from libjnidispatch.so+0x8bc4"]}}
```

The example abbreviates the registers, stack and JNI history. If a bad JNI answer replaces the command's error message, `crash` still holds the original guest failure.

### Traces

Change traces during a session with `{"cmd":"trace","jni":true,"syscalls":true,"imports":false,"heap":true,"code":["libfoo.so+0x1000-0x2000"],"calls":["libfoo.so"],"coverage":["libfoo.so"]}`. Omitted booleans become `false`; omitted `code`, `calls` and `coverage` become empty. `{"cmd":"trace"}` turns all traces off. Log format is chosen when starting the CLI; JSON logs go to stderr and protocol replies go to stdout.

Heap and object replies use decimal strings for addresses, sizes, offsets and word values. Thread IDs are numbers and `live` is a boolean. Allocation `backtrace` and `freed` frames use the same JSON as a crash’s `backtrace`; `freed` is null while the block is live and also for a retired record whose release was not observed. A field’s `target` is null, `{kind:"block", address, offset, object}`, or `{kind:"location", location}`. Referrers have `{address, location, block, object}`; `block` is a decimal string or null, and `object` is the nested object or null. Fields without a printable string have `string: null`. For example, enable `{"cmd":"trace","heap":true}`, run the call, then inspect `{"cmd":"heap"}` or `{"cmd":"object","at":"0x50001010","depth":1}`.

### Stops and questions

Serve `load` runs constructors with the session as handler, so constructor breakpoints and hooked imports reach the driver. To set a breakpoint after mapping but before execution, send `{"cmd":"load","path":"libfoo.so","constructors":false}`, then `break`, then `{"cmd":"run_constructors","module":0}`. The last command returns `{"ok":null}` after pending constructors finish; the module number is the one returned by `load`.

Import questions use `{"import":{"name":"strlen","args":["4096","0","0","0"],"original":"268439552","from":"libfoo.so+0x1234"}}`; the example is ARM32, and ARM64 has eight arguments. Reply with `{"cmd":"answer","value":"5"}` or `{"cmd":"unhandled"}`. `unhandled` tail-jumps to the original, or fails with the symbol and caller when `original` is `null`. As with JNI questions, nested commands work, including calling `original`; resume commands error and keep the import question open. Answer or resume the innermost question first. The command list is in [README.md](README.md); the Rust hook example is in [ENVIRONMENT.md](ENVIRONMENT.md).

Use `{"cmd":"watch","at":"0x40001000","size":4,"kind":"write","log":true}` to record hits throughout a call. Log hits go to stderr as text or JSON according to `--log-format`, just like traces, and never ask a `break` question. Omitting `log` or setting it to `false` selects stop mode.

Serve `break` accepts optional `"if":"EXPR==EXPR"` or `"if":"EXPR!=EXPR"`. Omitting `if` installs a plain breakpoint. Tracepoint commands and expressions are described above; their JSON events go to stderr with `--log-format json`, never to protocol stdout.

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
| `trace` | `jni`, `syscalls`, `imports`, `heap` (booleans, omitted means false), `code`, `calls`, `coverage` (lists of specifications, omitted means empty) | `null` |
| `coverage` | `reset` (boolean, omitted means false) | list of `{address, location, count}` in address order; address is a decimal string, count is a number; `reset: true` returns the collected counts and then clears them |
| `heap` | `reset` (boolean, omitted means false) | libc allocation records in allocation-entry order; `reset: true` returns the collected records and then clears them |
| `object` | `at` (text for `resolve`), `depth` (number, default 1, at most 64) | containing live known block, fields and referrers; depth 0 includes only the root, depth 1 also expands direct targets and referrer blocks, each further level follows one more link |
| `break` / `unbreak` | `at` (text for `resolve`) | address with Thumb bit cleared / `null` |
| `break` (conditional) | `at` (text for `resolve`), optional `if` (condition text; omitted means plain breakpoint) | address with Thumb bit cleared (decimal string) |
| `tracepoint` | `at` (text for `resolve`), `values` (nonempty list of expression strings), `log` (boolean, default true) | address with Thumb bit cleared (decimal string); replaces the point and clears its counts |
| `untracepoint` | `at` (text for `resolve`) | `null` |
| `tracepoint_hits` | `at` (text for `resolve`), `reset` (boolean, default false) | list of `{value, count}` sorted by value; value is a decimal string, count a number; reset clears only this point’s counts after reading; allowed in batch |
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

The recording starts with `{"record":1,"version":"<crate version>","options":{...}}`. `options` holds `rootfs`, `process_name`, `prop`, `alias`, `superclass`, `fixed_clock`, `random_seed` and `translation_cache_mb`; the three repeatable settings are arrays of key/value pairs. Defaults are recorded too. `fixed_clock` and `random_seed` are the built emulator's effective values, including values supplied only through the builder; explicit `Options` values take precedence over builder values. Trace and log options are not in the header. Subsequent entries stay in consumption order:

With `--policy`, the header also holds `"policy":{"rules":[...]}`: the parsed rules, with their original `values` lists. The policy path is not stored. Replay uses these embedded rules from the first sequence value, even if the original file changed or was deleted. A recording made without a policy has no `policy` entry. See [README.md](README.md) for the policy format and API.

| entry | what it holds |
|---|---|
| `{"in":"<line>"}` | the input exactly as read, with its line ending removed |
| `{"out":<JSON value>}` | a response or question |
| `{"time":{"clock":"realtime","sec":123,"nsec":456}}` | a clock read; `clock` can also be `monotonic` |
| `{"random":"<base64>"}` | bytes read from `/dev/urandom` or `/dev/random` |

Fixed arm64 counter reads also use `time` entries with `clock: "monotonic"`. `getrandom` also uses `random` entries and serves them from the recording during replay. Fixed clock and seeded random values are recorded in the same consumption order as host values. Replay rebuilds `fixed_clock` from the header, so fixed file metadata matches too.

`replay FILE` rebuilds the recorded setup, feeds the inputs through the serve loop, and compares every response and question as JSON. Guest `clock_gettime`, `gettimeofday` and random-device reads use the recording instead of the host. Keep the same libraries and rootfs; relative rootfs and `load` paths resolve against the current directory, so run replay from the directory the service ran in.

Replaying a recording that took snapshots writes those snapshot files again, using paths relative to the replay process's directory.

Replay accepts only `--verbose`, `--trace-syscalls`, `--trace-imports`, repeatable `--trace-code` and `--trace-calls`, and `--log-format text|json`. It adds no breakpoints. Diagnostics go to stderr. Success prints `replayed <n> commands, all responses match` on stdout and exits 0; `n` counts input lines, including JNI answers. The first difference exits 1 with one stdout line naming the recording line: differing responses include expected and actual JSON; missing or extra responses, wrong time/random order, exhausted values and unused values also name the mismatch.

The library exposes `serve::Options` (the shared options and `build`), `serve::record(options, builder, file, input, output)` and `serve::replay(file, builder)`. Replay returns the input count; the builder supplies diagnostic settings. The clock/random source and recording entries remain crate-internal.

### `diff`: compare two memory snapshots

Take snapshots before and after a call with serve's `snapshot` command or the library API, then run:

```bash
target/release/unidroid diff before.snap after.snap
```

It prints the differences and exits 0, even when memory changed. For example, without heap data:

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

With heap data, the report also shows allocation history and the block containing each change's first byte:

```text
registers:
  (none)
mapped:
  (none)
unmapped:
  (none)
allocated:
  0x50001000, 32 bytes, malloc from libfoo.so+0x1234 (make+0x10)
freed:
  0x50000000, 64 bytes, calloc from libfoo.so+0x1200 (prepare+0x8)
changes:
  0x50001008 [mmap], 4 bytes in block 0x50001000+0x8
    0000 - 00 00 00 00
    0000 + 01 02 03 04
```
