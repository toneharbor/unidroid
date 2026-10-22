# unidroid

[![CI](https://github.com/zhoupb01/unidroid/actions/workflows/ci.yml/badge.svg)](https://github.com/zhoupb01/unidroid/actions/workflows/ci.yml)

A low-memory unidbg. It runs Android `.so` files on a computer, no phone needed. Both 64-bit (`arm64-v8a`) and 32-bit (`armeabi-v7a`) work. The most common use is calling an app's signing or encryption functions.

It's written in Rust on top of Unicorn, the same CPU emulator unidbg uses. There's no Java VM. Use it as a Rust library, or from any other language through the `unidroid` command.

## How much it saves

Both run nine of unidbg's own sample `.so` files and give exactly the same results. Measured on an Apple Silicon Mac, peak memory (max RSS):

| sample | unidbg 0.9.9 | unidroid |
|---|---|---|
| JNA `libjnidispatch.so`, 1 instance | 91 MB | 7.8 MB |
| Telegram `libtmessages.29.so`, 1 instance | 131 MB | 15.6 MB |
| Telegram, 4 instances in one process | 258 MB | 28.1 MB |
| JNA `libjnidispatch.so`, 32-bit, 1 instance | 93 MB | 9.1 MB |
| ttEncrypt `libttEncrypt.so`, 32-bit, 1 instance | 92 MB | 8.8 MB |
| ttEncrypt, 32-bit, 4 instances in one process | 106 MB | 20.4 MB |
| Telegram `libtmessages.29.so`, 32-bit, 1 instance | 128 MB | 16.2 MB |
| Anjuke SignUtil `libsignutil.so`, 32-bit, 1 instance | 94 MB | 10.2 MB |
| QDReader `libd-lib.so`, 32-bit, 1 instance | 97 MB | 11.9 MB |
| Kanxue crackme `libnative-lib.so`, 32-bit, 1 instance | 317 MB | 19.2 MB |
| Kanxue crackme, 32-bit, 4 instances in one process | 411 MB | 56.5 MB |
| Thread test `libthread-lib.so`, 32-bit, 1 instance | 99 MB | 12.3 MB |

It's also about 3x faster. Telegram's PBKDF2 at 100,000 iterations: unidbg about 4.5 s, unidroid about 1.4 s. The Kanxue crackme tries 64,589 inputs: unidbg about 3 s, unidroid about 0.6 s.

Why it's smaller:

- No Java VM.
- A library's read-only parts (code, constants) are never copied. They're mapped straight from the file. Code that never runs never uses memory.
- Several instances in one process share a single copy of that read-only code. Each instance keeps only its own writable data.

Re-measure: `bench/compare.sh 1 4`. It runs unidbg and unidroid side by side and checks that their outputs match. It needs Java 21 and Maven.

## Usage

Add it to a Rust project:

```bash
cargo add unidroid
```

The crate doesn't include `rootfs`. Copy the `rootfs` directory from this repository, or take it from a release download.

```rust
use unidroid::{Emulator, JniHandler, JniCall, Handle, JniValue, Trace};

// Setting up the environment: whatever Java the .so asks for, you answer here.
struct Env;
impl JniHandler for Env {
    fn call_static_object_method(&mut self, emu: &mut Emulator, call: &JniCall<'_>) -> Option<Handle> {
        match (call.class, call.name) {
            ("java/lang/System", "getProperty") => Some(emu.new_string("UTF-8")),
            _ => None, // Returning None stops and tells you which call needs an answer
        }
    }
}

let mut emu = Emulator::builder()
    .rootfs("rootfs")                     // Android system libraries
    .process_name("com.example.app")
    .system_property("ro.product.model", "Pixel")
    .trace(Trace { jni: true, ..Trace::default() }) // Print every JNI call
    .build()?;

let m = emu.load_library("libfoo.so")?;  // 32-bit or 64-bit: the file decides
let mut env = Env;
emu.call_jni_onload(m, &mut env)?;

// Call a JNI method
let input = emu.new_byte_array(b"hello");
let out = emu.call_static_jni_method(&mut env, "com/example/Sign", "sign([B)Ljava/lang/String;", &[input.into()])?;
println!("{}", emu.string(out.object())?);
emu.release(input);
emu.release(out.object());

// Call a plain C function
let f = emu.find_symbol(m, "some_c_function")?;
let ret = emu.call_function(f, &[1, 2])?;
```

When the `.so` calls Java you haven't answered, it stops with a message like this:

```
unhandled JNI call: CallStaticObjectMethodV java/lang/System->getProperty(Ljava/lang/String;)Ljava/lang/String; from libjnidispatch.so+0x97c4
```

Add the matching method to `JniHandler` and run again. It's the same idea as setting up the environment in unidbg.

Guest failures carry a `Crash`. Its `Display` is the same error message; `report()` adds the thread, backtrace and registers:

```rust
if let Err(error) = emu.call_function(f, &[]) {
    if let Some(crash) = error.downcast_ref::<unidroid::Crash>() {
        eprintln!("{}", crash.report());
    }
}
```

`Crash` has `message`, `thread`, `registers` (name/value pairs) and `backtrace`. Each `Frame` has `address`, `location` and an optional `symbol`. Both types serialize with serde. A crash keeps the state at the failure, including failures in guest threads or nested calls. Host function failures start at the guest caller. `emu.registers()?` and `emu.backtrace()?` read the current state. The backtrace follows frame pointers, skips repeated neighbors and stops at 32 frames or a zero return address. Code without frame pointers may give a short backtrace.

`emu.location(address)` stays `libfoo.so+0x1234`. `emu.symbolize(address)` adds a covering function name, like `libc.so+0x1f2a4 (strlen+0x10)`. It uses function symbols with a nonzero size from `.dynsym` and `.symtab`, when present. The symbol table is built on first use and shared between instances.

`emu.add_breakpoint(address)?` stops before the instruction at that address. `emu.remove_breakpoint(address)?` removes it and errors if none is there. Both clear the Thumb bit; adding the same address twice keeps one breakpoint. Breakpoints also work on code that already ran and in any guest thread. With no breakpoints or step active, no debugger hook is installed.

A Thumb `IT` instruction and the instructions it makes conditional run as one step. A breakpoint hit inside an `IT` block is reported at the first instruction after the block, or at the next Unicorn stop if the block branches away; a conditional instruction that is skipped does not hit a breakpoint.

Stops go to the running call's `JniHandler::stopped(&mut self, emu: &mut Emulator, stop: &Stop) -> Resume`. The default returns `Resume::Continue`, including for `()` and `Probe`. `Stop` has `reason: StopReason` (`Breakpoint`, `Step` or `Finish`), `address: u64` (Thumb bit cleared) and `thread: u32`. The handler can inspect or change registers and memory, or make a nested call that has stops of its own. Return one of:

| `Resume` | what happens |
|---|---|
| `Continue` | run on, skipping the current breakpoint hit once |
| `Step` | execute one guest instruction, then stop with `Step`; `bl` enters the callee, and entering a host function (JNI, dl or system property) runs that whole host function as one step |
| `Finish` | save lr and sp; stop with `Finish` in the same thread when `pc == (saved_lr & !1)` and `sp >= saved_sp`, including at the end of a top-level call |
| `Return(bits)` | return immediately to lr; set arm64 `x0`, or arm32 `r0` to the low 32 bits and `r1` to the high 32 bits |
| `Abort` | fail the running call with a `Crash` whose message is `stopped by the debugger at <symbolize(address)>` |

`Finish` relies on lr, so use it at a function's first instruction, before the function changes lr. Its stack check prevents stopping at a deeper recursive return. `Return(bits)` is also meant for the first instruction: it replaces the function's result without running the body. This is a function hook.

For a plain C call, use `emu.call_function_with(&mut handler, function, args)?` to receive stops and JNI questions in that handler. It takes the same arguments and returns the same result as `call_function`.

`emu.resolve(text)?` accepts `libfoo.so+0x1234`, an absolute hex address such as `0x50001000`, or an exported symbol name searched in module load order. Errors name the unknown module, bad number or unknown symbol. `emu.modules()` returns `Vec<ModuleInfo>` in load order: each entry has `name: String`, the host `path: PathBuf` recorded at load, `base: u64` and `size: u64`. Its index is the `Module` number.

`emu.register(name)?` reads a register and `emu.set_register(name, value)?` writes it. Names are `x0`–`x28`, `fp`, `lr`, `sp`, `pc`, `nzcv` on arm64; `r0`–`r12`, `sp`, `lr`, `pc`, `cpsr` on arm32; and `d0`–`d31` on both. The `d` registers hold 64-bit floating-point bits and are available through named reads and writes, including serve's `set_register`. Dumps from `registers()`, crash reports, break questions and serve's `registers` list only the general registers, sp, lr, pc and flags, in the order above. An unknown name lists the valid names.

`Trace` has three booleans (`jni`, `syscalls` and `imports`, default `false`) and `code: Vec<String>` (default empty). Set them with `.trace(Trace { .. })` on the builder, or `emu.set_trace(Trace { .. })?` at runtime. `emu.trace()` reads the settings. Each change replaces all settings. With traces off, no trace hook is installed.

Each `code` entry is a module file name, such as `libfoo.so`, or a module with a hex offset range, such as `libfoo.so+0x1000-0x2000` (end exclusive). It matches libraries already loaded and libraries loaded later, by file name. Bad specifications fail `build` or `set_trace`. For example, `Trace { code: vec!["libfoo.so+0x1000-0x2000".into()], ..Trace::default() }` traces that part of the library.

Instruction events show the bytes in memory, Capstone disassembly, registers read before the instruction, and registers written after it. ARM32 follows the current ARM or Thumb mode, including Thumb conditional blocks. The text looks like this:

```text
[libfoo.so+0x1234 (fn+0x10)] [0010a0e1] mov r1, r0 ; r0=0x7 => r1=0x7
```

`Event::Instruction` has `at` (from `symbolize`), `address`, `bytes` (hex), `text`, `read` and `written`. The last two are name/value pairs (`Vec<(String, u128)>`, covering vector registers too). In JSON, `address` and register values are hex strings, such as `"address":"0x10001000"` and `[["r0","0x7"]]`, preserving full precision in JavaScript. Pairs stay in instruction order; the event tag is `instruction`. Events wait for the next instruction or a run stop so written values are available. A range exit also captures the result before untraced code changes it. Syscall results are captured after dispatch; a faulting instruction has an empty `written` array. Stops before execution do not duplicate an instruction on resume. Turning code tracing off removes all its hooks. If an instruction cannot be decoded or a register cannot be read, a `Warning` names its address and tracing continues. Capstone also names SVE/SME and legacy registers that Unicorn does not expose; those reads warn instead of inventing a value.

Syscall events show the guest number, arguments, signed result, errno name, instruction location and caller. Import events show calls from app libraries into other modules, including dl, property and unresolved-symbol stubs. The previous executed block in the same guest thread must be in an app library; app PLT stubs count, but internal tail calls within a dependency do not. The caller location comes from lr. They show the first four argument registers, with known C strings quoted and capped at 256 bytes. Dependencies next to an app count as app libraries; dependencies found in rootfs do not.

All diagnostics go through `Event`: `Jni`, `JniCall`, `Syscall`, `Call`, `Instruction` or `Warning`. By default they print their text form on stderr. Capture them with the builder's `on_event`:

```rust
let emu = Emulator::builder()
    .trace(Trace { syscalls: true, ..Trace::default() })
    .on_event(|event| eprintln!("{}", serde_json::to_string(event).unwrap()))
    .build()?;
```

Events serialize as objects tagged with `event`: `jni`, `jni_call`, `syscall`, `call`, `instruction` or `warning`. Arguments are arrays of text; JNI member arguments keep their typed debug form, such as `Object(Handle(7))`. Syscall numbers and returned results are JSON numbers. If dispatch fails, ends the thread or process, or switches away from the thread, `result` is `null` and `errno` is absent. Its text line ends with `= (no return)`. The event is emitted before any crash propagates. Otherwise `errno` is present only for a Linux error result. JNI locations keep the existing text; syscall and import locations include symbols.

`Probe::default()` is a `JniHandler` that answers calls with null, zero, false or void. `is_assignable` answers false. Pass it to `call_jni_onload` to explore calls without writing a handler. `probe.calls()` returns `&[ProbedCall]` in first-seen order. Each entry has `op`, `is_static`, `class`, `name`, `signature`, `count` and the first `from` location. For `is_assignable`, `class` is the source and `name` the target class. Defaults can make the library return early or crash.

32-bit or 64-bit is picked by the first library you load. There's no setting for it. Load that library before `malloc`, `read_bytes`, `write_bytes` or `call_function`; before it, they return an error. One emulator runs one kind: loading a 64-bit library after a 32-bit one is an error.

On 32-bit, `call_function` passes each argument as one 32-bit word and returns only `r0`. To pass a 64-bit value, give it as two words, low word first. If it would start at an odd word position (counting from 0), put a `0` word in front of it. `call_static_jni_method` and `call_jni_method` handle this for you.

Objects you create (strings, arrays, `malloc` memory) live until you free them with `release` / `free`. In a long-running service, remember to free them. Reading a handle that was released, or that holds another kind of object, returns an error.

Before running a library's code, Unicorn translates it and keeps the result in a translation cache, 8 MB by default. When a large library fills the cache, everything is thrown away and translated again, and each call gets several times slower. Give it more with `.translation_cache_size(16 * 1024 * 1024)` on the builder, or `--translation-cache-mb 16` on the command. Memory grows only as the cache fills, so a small library uses no more.

If a library needs a function that rootfs has under another name, point it there before loading. For example, Android 6.0's libcrypto has no `DES_set_key_unchecked`, only `DES_set_key`:

```rust
emu.symbol_alias("DES_set_key_unchecked", "DES_set_key");
emu.load_library("libd-lib.so")?;
```

The alias is used only when no loaded library has the real name.

There are no Java classes, so unidroid doesn't know which class extends which. If a library calls `GetSuperclass`, or registers a native method on a parent class and you call it through a child class, tell unidroid the parents first:

```rust
emu.superclass("com/example/Sign", "com/example/Base");
emu.superclass("com/example/Base", "java/lang/Object");
```

Now `GetSuperclass` answers, and `call_static_jni_method(.., "com/example/Sign", ..)` also finds a native method registered on `com/example/Base`. Without it, `GetSuperclass` stops and says which class to declare.

## The `unidroid` command

For other languages (TypeScript, Python, Go, ...) there's a command-line program. Download it from [Releases](https://github.com/zhoupb01/unidroid/releases): there are builds for Linux (x86_64, arm64) and macOS (Apple Silicon). Each download has `rootfs` inside. Or build it yourself:

```bash
cargo build --release
```

It lands at `target/release/unidroid`. It has three subcommands.

### `inspect`: look at a library first

```bash
target/release/unidroid inspect --rootfs rootfs libfoo.so
```

No code needed. It prints what the library is, which libraries it needs, its `Java_` functions, and whether it has `JNI_OnLoad`. Then it runs `JNI_OnLoad` and prints each Java call it makes (on stderr). It stops at the first one, because nothing answers it yet. Last, it lists the methods registered through `RegisterNatives`. Now you know what to call and which Java calls you'll have to answer.

For the SignUtil sample:

```
libsignutil.so: 32-bit ARM (armeabi-v7a)
Needs: liblog.so, libm.so, libdl.so, libc.so
Java_ exports:
  Java_com_anjuke_mobile_sign_SignUtil_getSign0
  Java_com_anjuke_mobile_sign_SignUtil_setCacheFilePath
JNI_OnLoad: yes
JNI_OnLoad returned 0x10004
Registered natives:
  (none)
```

This one asks Java nothing in `JNI_OnLoad`, so it runs to the end. Its two Java methods are plain `Java_` exports.

Use `inspect --probe LIB` to answer Java calls with `Probe` defaults and print a `Probed calls` section, with the first caller and count for each call. If `JNI_OnLoad` fails, inspect prints a crash report after the stop line and still prints the list. For example:

```bash
target/release/unidroid inspect --probe --log-format json libfoo.so
```

A default answer can make the library return early, take another branch or crash. The list shows only the calls it reached.

### `serve`: drive it from your program

```bash
target/release/unidroid serve --rootfs rootfs
```

Your program starts this as a child process. Then the two talk through stdin and stdout, one JSON object per line. Logs go to stderr, so they never mix with the messages.

You send commands. unidroid answers each one with `{"ok": ...}` or `{"error": "..."}`. When the library calls Java in the middle of a command, unidroid sends a `{"jni": ...}` question instead, and you answer it. This is the same job `JniHandler` does in Rust, written in your own language:

Part of a real run of the SignUtil example (`→` you send, `←` unidroid replies; some lines left out):

```
→ {"cmd":"load","path":"tests/fixtures/armeabi-v7a/libsignutil.so"}
← {"ok":0}
→ {"cmd":"onload","module":0}
← {"ok":65540}
→ {"cmd":"new_string","value":"aa"}
← {"ok":7}
   ... bb → 8, a HashMap → 9, cc → 10 ...
→ {"cmd":"call_static","class":"com/anjuke/mobile/sign/SignUtil","method":"getSign0(Ljava/lang/String;Ljava/lang/String;Ljava/util/Map;Ljava/lang/String;I)Ljava/lang/String;","args":[7,8,9,10,10]}
← {"jni":{"args":[],"class":"java/util/HashMap","name":"size","op":"call","receiver":9,"signature":"()I","static":false,"type":"I"}}
→ {"cmd":"answer","value":2}
   ... keySet, toArray ...
← {"jni":{"args":[14],"class":"java/util/HashMap","name":"get","op":"call","receiver":9,"signature":"(Ljava/lang/Object;)Ljava/lang/Object;","static":false,"type":"L"}}
→ {"cmd":"string","handle":14}
← {"ok":"a"}
→ {"cmd":"answer","value":2}
   ... get("b") ...
← {"ok":16}
→ {"cmd":"string","handle":16}
← {"ok":"1000ed2f7e5ecd00bb4a6454e1d487b7dc7500040002000acc"}
```

Look at the `get` question: before answering, the program sends `string` to read which key is wanted (`"a"`), then answers with the handle of that key's value. While a question is open you can send any other command like this. A command that runs library code can even bring a question of its own; answer it first. If you don't know the answer, send `{"cmd":"unhandled"}`. The command then fails with the usual `unhandled JNI call: ...` message.

A full working example in TypeScript, with no npm packages: [examples/ts/signutil.ts](examples/ts/signutil.ts). Run it with:

```bash
node examples/ts/signutil.ts
```

The same example in Python, also with nothing to install: [examples/python/signutil.py](examples/python/signutil.py).

```bash
python3 examples/python/signutil.py
```

The same program works for local testing and inside an HTTP service: the service starts `unidroid serve` and answers the Java calls the same way. One `serve` process runs one instance, one command at a time. For calls in parallel, start several.

Options, the same for `inspect` and `serve`:

| option | what it does |
|---|---|
| `--rootfs DIR` | Android system libraries (default `rootfs`) |
| `--process-name NAME` | process name the library sees |
| `--prop KEY=VALUE` | a system property, repeatable |
| `--alias NAME=TARGET` | same as `symbol_alias`, repeatable |
| `--superclass CLASS=SUPERCLASS` | same as `superclass`, repeatable |
| `--verbose` | print every JNI call on stderr (`inspect` always does) |
| `--trace-syscalls` | print each guest syscall, arguments, result and caller |
| `--trace-imports` | print app calls into other libraries |
| `--trace-code SPEC` | print instructions in a module or `module+0xSTART-0xEND`, repeatable |
| `--log-format text\|json` | text logs (default) or one JSON event per stderr line |
| `--translation-cache-mb MB` | translation cache size (default 8), same as `translation_cache_size` |

How values are written in JSON:

| Java type | JSON |
|---|---|
| object, string, array | handle number (`0` is `null`) |
| `boolean` | `true` / `false` |
| `byte`, `char`, `short`, `int`, `float`, `double` | number |
| `long` | decimal string, like `"-12"` (a JS number can't hold 64 bits) |

Addresses are decimal strings too. Bytes are base64.

A guest failure adds `crash` to the error response. Other errors still have only `error`. Inside `crash`, frame addresses and register values are decimal strings; the thread ID is a number. `registers` is an object keyed by register name, so `crash.registers.pc` reads the program counter. For example:

```json
{"error":"unhandled JNI call: ...","crash":{"message":"unhandled JNI call: ...","thread":1234,"registers":{"pc":"65820","x0":"7"},"backtrace":[{"address":"268471236","location":"libjnidispatch.so+0x8bc4","symbol":"JNI_OnLoad+0x3c0"}]}}
```

The example abbreviates the registers. If a bad JNI answer replaces the command's error message, `crash` still holds the original guest failure.

Change traces during a session with `{"cmd":"trace","jni":true,"syscalls":true,"imports":false,"code":["libfoo.so+0x1000-0x2000"]}`. Omitted booleans become `false`; omitted `code` becomes empty. `{"cmd":"trace"}` turns all traces off. Log format is chosen when starting the CLI; JSON logs go to stderr and protocol replies go to stdout.

Debugger stops arrive as `{"break": ...}` questions. The fields are `reason` (`breakpoint`, `step` or `finish`), `address` (decimal string, Thumb bit cleared), `location` (from `symbolize`), `thread` (number), and `registers` (the same object, in register table order, as `registers` and a crash). While stopped, any command can run, including a nested `call_function`. Resume the innermost stop with `continue`, `step`, `finish`, `return` (with a decimal string `value`), or `abort`. These have the same behavior as Rust's `Resume`; they have no separate reply. The original command continues and ends with its own `ok` or `error`.

A short exchange after loading SignUtil and creating the arguments (register objects abbreviated; Java questions during `finish` are answered as above):

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

A resume command with no stop open errors with `no breakpoint stop is waiting`. An open stop expects a resume command; an open JNI question expects `answer` or `unhandled`. Sending the other kind gives an error and leaves the question open. With nested questions, answer or resume the innermost one first.

Commands:

| command | fields | result |
|---|---|---|
| `load` | `path` | module number |
| `trace` | `jni`, `syscalls`, `imports` (booleans, omitted means false), `code` (list of specifications, omitted means empty) | `null` |
| `break` / `unbreak` | `at` (text for `resolve`) | address with Thumb bit cleared / `null` |
| `resolve` | `at` (module + hex offset, absolute hex or exported symbol) | address (decimal string) |
| `registers` | | register object, values as decimal strings, in table order |
| `set_register` | `name`, `value` (decimal string) | `null` |
| `backtrace` | | frames, in the same JSON as a crash's `backtrace` |
| `modules` | | list of `{name, path, base, size}`, in load order; base and size are decimal strings |
| `continue` / `step` / `finish` / `abort` | | (resumes the open stop) |
| `return` | `value` (decimal string) | (returns from the stopped function) |
| `alias` | `name`, `target` | `null` |
| `superclass` | `class`, `superclass` | `null` |
| `onload` | `module` | `JNI_OnLoad`'s return |
| `call_static` | `class`, `method` (name + signature), `args` | the return value |
| `call` | `class`, `receiver`, `method`, `args` | the return value |
| `find_symbol` | `module`, `name` | address |
| `call_function` | `address`, `args` (decimal strings) | result (decimal string) |
| `malloc` / `free` | `size` / `address` | address / `null` |
| `new_cstring` | `value` | address |
| `read` / `write` | `address`, `size` / `address`, `base64` | base64 / `null` |
| `new_string` / `string` | `value` / `handle` | handle / the string |
| `new_array` / `array` | `type` (`Z B C S I J F D`), `base64` / `handle` | handle / base64 |
| `new_object_array` / `object_array` | `class`, `items` / `handle` | handle / list of handles |
| `new_object` | `class` | handle |
| `class` | `name` | handle |
| `release` | `handle` | `null` |
| `answer` | `value` (left out for `void` and field sets) | (answers the open question) |
| `unhandled` | | (answers the open question) |

A question looks like this:

```json
{"jni":{"op":"call","static":true,"type":"L","class":"java/lang/System","name":"getProperty","signature":"(Ljava/lang/String;)Ljava/lang/String;","receiver":4,"args":[7]}}
```

- `op` is `call`, `get` (read a field), `set` (write a field), `new` (a constructor) or `is_assignable` (that one carries `from` and `to` instead).
- `type` is what your answer must be, using the letters above. `V` means just `{"cmd":"answer"}`.
- `args` are written the same way as values. For `set`, the value being written is in `args`.

### Record a session and replay it

Start the service with `serve --record FILE`. stdin, stdout and JNI questions stay the same. Each recording entry is one JSON line, flushed immediately so completed entries survive a crash. A recording contains every input, including keys. Keep recordings out of git.

```bash
target/release/unidroid serve --rootfs rootfs --record session.jsonl
target/release/unidroid replay session.jsonl --trace-syscalls --trace-code libsignutil.so
```

The recording starts with `{"record":1,"version":"<crate version>","options":{...}}`. `options` holds `rootfs`, `process_name`, `prop`, `alias`, `superclass` and `translation_cache_mb`; the three repeatable settings are arrays of key/value pairs. Defaults are recorded too. Trace and log options are not in the header. Subsequent entries stay in consumption order:

| entry | what it holds |
|---|---|
| `{"in":"<line>"}` | the input exactly as read, with its line ending removed |
| `{"out":<JSON value>}` | a response or question |
| `{"time":{"clock":"realtime","sec":123,"nsec":456}}` | a clock read; `clock` can also be `monotonic` |
| `{"random":"<base64>"}` | bytes read from `/dev/urandom` or `/dev/random` |

`replay FILE` rebuilds the recorded setup, feeds the inputs through the serve loop, and compares every response and question as JSON. Guest `clock_gettime`, `gettimeofday` and random-device reads use the recording instead of the host. Keep the same libraries and rootfs; relative rootfs and `load` paths resolve against the current directory, so run replay from the directory the service ran in.

Replay accepts only `--verbose`, `--trace-syscalls`, `--trace-imports`, repeatable `--trace-code`, and `--log-format text|json`. It adds no breakpoints. Diagnostics go to stderr. Success prints `replayed <n> commands, all responses match` on stdout and exits 0; `n` counts input lines, including JNI answers. The first difference exits 1 with one stdout line naming the recording line: differing responses include expected and actual JSON; missing or extra responses, wrong time/random order, exhausted values and unused values also name the mismatch.

The library exposes `serve::Options` (the shared options and `build`), `serve::record(options, builder, file, input, output)` and `serve::replay(file, builder)`. Replay returns the input count; the builder supplies diagnostic settings. The clock/random source and recording entries remain crate-internal.

## What it does and doesn't support

Supported:

- Android arm64 (`arm64-v8a`) and 32-bit ARM (`armeabi-v7a`, both ARM and Thumb code), using Android 6.0 (API 23) system libraries
- Loading a `.so` and its dependencies, relocation (including Android packed relocations), running constructors
- libdl (`dlopen` / `dlsym` and the rest), system properties, the common syscalls
- JNI: classes, methods, fields, strings, arrays, exceptions, global and local references, `RegisterNatives`. Strings and arrays handed to native code come from libc's own `malloc`, as on a phone, so native code may `free` them.
- Guest threads (`pthread_create`, `pthread_join`, and the futex waits behind locks). They take turns on one CPU and switch about every 10,000 instructions, like unidbg. Switching keeps the translated code. A thread that is still running when your call returns keeps going during your next call. An instance that never starts a thread pays nothing for this.
- The 32-bit ARM kernel helpers at `0xffff0f60`–`0xffff0fff`, which old libraries and Android 6.0's own libc use
- Several instances in one process

Not supported:

- iOS
- Signals and `fork`
- File access outside `rootfs`. `/dev/urandom` and `/dev/random` are allowed. Every other path is "not found".

A library's dependencies are looked for first in the folders of the libraries you loaded, then in rootfs. That's how an app's own `lib/arm64-v8a` folder works on a phone, so copy an app's libraries together and load the one you need. `dlopen` from the library searches the same way.

A dependency found in neither place (such as `libEGL.so`) only prints a warning. If its function is actually called, it stops and names the function.

## Directories

- `src/`: the emulator, and the `unidroid` command (`main.rs`, `serve.rs`)
- `examples/ts/`, `examples/python/`: using the command from TypeScript and Python
- `rootfs/lib/`, `rootfs/lib64/`: Android 6.0 system libraries (32-bit and 64-bit), from unidbg
- `tests/`: tests, including one per sample, checked byte for byte against unidbg
- `bench/`: the unidbg comparison program and script

## License

Apache License 2.0, same as unidbg. See `LICENSE`. Credits are in `NOTICE`.
