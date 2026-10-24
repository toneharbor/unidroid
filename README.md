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
- A library's read-only parts (code, constants) are mapped straight from host files or aligned stored APK entries. Compressed and unaligned APK entries are read into shared memory; see [ENVIRONMENT.md](ENVIRONMENT.md).
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

// Answer app-specific Java calls here.
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

For repeatable guest memory and block coverage, the builder supports `.fixed_clock(1_700_000_000)` and `.random_seed(42)`. See [deterministic runs](DEBUGGING.md#deterministic-runs) for their behavior and the inputs to keep the same.

When the `.so` calls Java you haven't answered, it stops with a message like this:

```
unhandled JNI call: CallStaticObjectMethodV java/lang/System->getProperty(Ljava/lang/String;)Ljava/lang/String; from libjnidispatch.so+0x97c4
```

Add the matching method to `JniHandler` and run again. It's the same idea as setting up the environment in unidbg.

See [ENVIRONMENT.md](ENVIRONMENT.md) for built-in Java classes.

For crash reports, debugger controls, memory inspection and traces, see [the library debugging reference](DEBUGGING.md#library). [Checkpoints and rewind](DEBUGGING.md#checkpoints-and-rewind) let you try inputs from one prepared state or copy it into independent emulators. `emu.crypto_constants()` scans mapped memory when called and names known algorithm tables and constants. It sees values stored as data, including tables made during `JNI_OnLoad`; values built by instructions and custom Base64 alphabets are not found.

Enable `Trace { heap: true, ..Trace::default() }` to track libc allocations and their callers. `emu.heap()` returns allocation history; `emu.object(address, depth)` expands a live block into pointer fields, strings and referrers. Serve exposes `trace` with `heap`, `heap` and `object`. Tracked snapshots and `unidroid diff` include allocated and freed blocks. See Heap and objects in [DEBUGGING.md](DEBUGGING.md) for depth rules, host allocations and allocator limits.

32-bit or 64-bit is picked by the first library you load; rewinding a checkpoint into a fresh emulator picks the checkpoint's architecture. There's no setting for it. Load that library before `malloc`, `read_bytes`, `write_bytes` or `call_function`; before it, they return an error. One emulator runs one kind: loading a 64-bit library after a 32-bit one is an error.

On 32-bit, `call_function` passes each argument as one 32-bit word and returns only `r0`. To pass a 64-bit value, give it as two words, low word first. If it would start at an odd word position (counting from 0), put a `0` word in front of it. `call_static_jni_method` and `call_jni_method` handle this for you.

Objects you create (strings, arrays, `malloc` memory) live until you free them with `release` / `free`. In a long-running service, remember to free them. Reading a handle that was released, or that holds another kind of object, returns an error.

Before running a library's code, Unicorn translates it and keeps the result in a translation cache, 8 MB by default. When a large library fills the cache, everything is thrown away and translated again, and each call gets several times slower. Give it more with `.translation_cache_size(16 * 1024 * 1024)` on the builder, or `--translation-cache-mb 16` on the command. Memory grows only as the cache fills, so a small library uses no more.

If a library needs a function that rootfs has under another name, point it there before loading. For example, Android 6.0's libcrypto has no `DES_set_key_unchecked`, only `DES_set_key`:

```rust
emu.symbol_alias("DES_set_key_unchecked", "DES_set_key");
emu.load_library("libd-lib.so")?;
```

The alias is used only when no loaded library has the real name.

Apart from the built-in classes, unidroid doesn't know which class extends which. If a library calls `GetSuperclass`, or registers a native method on a parent class and you call it through a child class, tell unidroid the parents first:

```rust
emu.superclass("com/example/Sign", "com/example/Base");
emu.superclass("com/example/Base", "java/lang/Object");
```

Now `GetSuperclass` answers, and `call_static_jni_method(.., "com/example/Sign", ..)` also finds a native method registered on `com/example/Base`. Without it, `GetSuperclass` stops and says which class to declare.

See [ENVIRONMENT.md](ENVIRONMENT.md) for replacing imported C functions and running constructors yourself.

## The `unidroid` command

Agents running or debugging a `.so`: read [skills/unidroid/SKILL.md](skills/unidroid/SKILL.md) for the work order and debugging tools. Read it in place, or copy `skills/unidroid/` into your agent's skills directory to install it.

For other languages (TypeScript, Python, Go, ...) there's a command-line program. Download it from [Releases](https://github.com/zhoupb01/unidroid/releases): there are builds for Linux (x86_64, arm64) and macOS (Apple Silicon). Each download has `rootfs` inside. Or build it yourself:

```bash
cargo build --release
```

It lands at `target/release/unidroid`. It has six subcommands.

### `inspect`: look at a library first

```bash
target/release/unidroid inspect --rootfs rootfs libfoo.so
```

No code needed. With `inspect --apk app.apk libfoo.so`, the library name is looked up in the APK and its split APK, then loaded host-library folders and rootfs (see [ENVIRONMENT.md](ENVIRONMENT.md)). It prints what the library is, which libraries it needs, its `Java_` functions, and whether it has `JNI_OnLoad`. Then it runs `JNI_OnLoad` and prints each Java call it makes (on stderr). It stops at the first call the built-ins do not answer. Last, it lists the methods registered through `RegisterNatives` and the crypto constants found in mapped memory. Now you know what to call, which Java calls you'll have to answer and which algorithm tables to inspect.

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
Crypto constants:
  Base64 alphabet at libc.so+0x6caa0
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

You send commands. unidroid answers each one with `{"ok": ...}` or `{"error": "..."}`. When a Java call is not covered by the built-ins or policy, unidroid sends a `{"jni": ...}` question instead, and you answer it. This is the same job `JniHandler` does in Rust, written in your own language:

Part of a real run of the SignUtil example (`→` you send, `←` unidroid replies; some lines left out):

```
→ {"cmd":"load","path":"tests/fixtures/armeabi-v7a/libsignutil.so"}
← {"ok":0}
   ... create keys "a" → 1, "b" → 3 and byte arrays containing "b" → 2, 4 ...
→ {"cmd":"onload","module":0}
← {"ok":65540}
   ... create "aa" → 5, "bb" → 6 ...
→ {"cmd":"new_java_map","class":"java/util/HashMap","entries":[[1,2],[3,4]]}
← {"ok":7}
   ... create "cc" → 8 ...
→ {"cmd":"call_static","class":"com/anjuke/mobile/sign/SignUtil","method":"getSign0(Ljava/lang/String;Ljava/lang/String;Ljava/util/Map;Ljava/lang/String;I)Ljava/lang/String;","args":[5,6,7,8,10]}
← {"ok":18}
→ {"cmd":"string","handle":18}
← {"ok":"1000ed2f7e5ecd00bb4a6454e1d487b7dc7500040002000acc"}
```

The built-in map answers `size`, `keySet`, `toArray` and `get`, so this run needs no Java answers from the driver. Map views and iterators are snapshots taken when created. While an app-specific question is open you can send other commands to inspect its arguments, except `checkpoint` and `rewind`, which need the call to have ended (see [Checkpoints and rewind](DEBUGGING.md#checkpoints-and-rewind)). A command that runs library code can even bring a question of its own; answer it first. If you don't know the answer, send `{"cmd":"unhandled"}`. The command then fails with the usual `unhandled JNI call: ...` message.

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
| `--apk FILE` | parse the app's APK and load its native libraries; see [ENVIRONMENT.md](ENVIRONMENT.md) |
| `--process-name NAME` | process name the library sees (default: APK package name, otherwise `unidroid`) |
| `--fixed-clock SECONDS` | start guest realtime at these Unix seconds; guest clock syscalls and arm64 counter reads advance time by one microsecond per read; also fix file metadata |
| `--random-seed N` | seed the shared guest random byte stream with SplitMix64 |
| `--prop KEY=VALUE` | a system property, repeatable |
| `--alias NAME=TARGET` | same as `symbol_alias`, repeatable |
| `--file GUEST=HOST` | add a host file at an absolute guest path; writes stay in memory; repeatable |
| `--superclass CLASS=SUPERCLASS` | same as `superclass`, repeatable |
| `--policy FILE` | answer matching Java calls from a JSON file before the driver or Probe |
| `--verbose` | print every JNI call on stderr (`inspect` always does) |
| `--trace-syscalls` | print each guest syscall, arguments, result and caller |
| `--trace-imports` | print app calls into other libraries |
| `--trace-code SPEC` | print instructions in a module or `module+0xSTART-0xEND`, repeatable |
| `--trace-calls SPEC` | print function calls from a module or `module+0xSTART-0xEND`, repeatable |
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

For crash responses, traces and debugger stop questions, see [the serve debugging reference](DEBUGGING.md#serve).

Commands:

| command | fields | result |
|---|---|---|
| `load` | `path` | module number |
| `load` (optional field) | `constructors` (boolean, default `true`; `false` defers constructors) | module number |
| `load_apk_library` | `name`, optional `constructors` (boolean, default `true`) | module number |
| `apk` | | `{path, package, version_name, version_code, signatures, signers, signing_history}`; certificate lists are base64; `null` without an APK |
| `set_apk` | optional `package`, `version_name` (string or `null`), `version_code`, `signatures`, `signers`, `signing_history` (base64 lists); omitted fields stay | `null`; error without an APK |
| `hook_import` | `name` | `null` |
| `run_constructors` | `module` | `null` |
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
| `add_file` | `path`, `base64` | `null` |
| `add_host_file` | `path`, `host_path` | `null` |
| `remove_file` | `path` | `null` |
| `file` | `path` | base64 |
| `crypto` | | list of `{name, address, location}` constants; address is a decimal string |
| `new_string` / `string` | `value` / `handle` | handle / the string |
| `new_array` / `array` | `type` (`Z B C S I J F D`), `base64` / `handle` | handle / base64 |
| `new_object_array` / `object_array` | `class`, `items` / `handle` | handle / list of handles |
| `new_java_map` | `class`, `entries` (list of `[key, value]` handles) | handle |
| `java_map` | `handle` | list of `[key, value]` handles in iteration order |
| `new_java_list` | `class`, `items` (handles) | handle |
| `java_list` | `handle` | list of handles |
| `new_boxed` | `type` (`Z B C S I J F D`), `value` (using the value table) | handle |
| `unboxed` | `handle` | `{type, value}` (using the value table) |
| `new_object` | `class` | handle |
| `class` | `name` | handle |
| `release` | `handle` | `null` |
| `answer` | `value` (left out for `void` and field sets) | (answers the open question) |
| `unhandled` | | (answers the open question) |

For debugging and resume commands, see [DEBUGGING.md](DEBUGGING.md#commands).

A question looks like this:

```json
{"jni":{"op":"call","static":true,"type":"L","class":"java/lang/System","name":"getProperty","signature":"(Ljava/lang/String;)Ljava/lang/String;","receiver":4,"args":[7]}}
```

- `op` is `call`, `get` (read a field), `set` (write a field), `new` (a constructor) or `is_assignable` (that one carries `from` and `to` instead).
- `type` is what your answer must be, using the letters above. `V` means just `{"cmd":"answer"}`.
- `args` are written the same way as values. For `set`, the value being written is in `args`.

Hooked C calls ask `{"import":{"name":"strlen","args":["4096","0","0","0"],"original":"268439552","from":"libfoo.so+0x1234"}}`. `args` holds four ARM32 or eight ARM64 register values; it and `original` use decimal strings, and `original` is `null` if absent. Reply `{"cmd":"answer","value":"5"}` to return 5, or `{"cmd":"unhandled"}` to run the original. A malformed import answer returns an error and leaves the question open for another answer. Nested commands, including `call_function` of `original`, work while waiting; resume commands error and leave the question open. Policies have no import rules; without a driver (batch or fields), imports pass through. Record/replay includes these questions and commands.

### Answering Java calls from a file

Use `serve --policy java.json` or `inspect --policy java.json LIB` to reuse known Java answers. Built-ins answer first, without consuming policy sequences. Uncovered calls still reach the serve driver. Inspect stops at the first uncovered call, or uses `Probe` defaults with `--probe`; its `Probed calls` list includes only calls that reached Probe.

```json
{"rules": [
  {"op":"call", "static":true, "class":"java/lang/System", "name":"getProperty", "signature":"(Ljava/lang/String;)Ljava/lang/String;", "value":"1.8.0"},
  {"op":"get", "class":"android/os/Build", "name":"MODEL", "value":"Pixel 7"},
  {"op":"call", "class":"android/content/Context", "name":"getPackageName", "values":["com.a","com.b"]},
  {"op":"is_assignable", "from":"java/lang/String", "to":"java/lang/Object", "value":true},
  {"op":"set", "class":"com/example/*"}
]}
```

Match fields are `op`, `static`, `class`, `name`, `signature`, `from` and `to`. Each is optional; an omitted field matches anything. Text matches the whole field, with `*` matching any run of characters, including empty; all other characters are literal. `op` names `call`, `get`, `set`, `new` or `is_assignable`, or a wildcard pattern matching at least one of them. Assignability questions have only `op`, `from` and `to`; a rule requiring a missing field does not match.

Rules run in file order; the first matching rule answers. `value` repeats on every match. `values` supplies one answer per match, then the exhausted rule is skipped. Use only one of these fields. Without either, a rule answers void calls and field sets; matching a call that needs a value errors. Errors inside a rule name its zero-based index, and answer errors also name the call and guest location. Errors outside a rule start with `policy:` and have no index.

Policy values use the call's type letter:

| type | policy value |
|---|---|
| `Z` | JSON boolean |
| `B C S I` | integer in that Java type's range |
| `F D` | number |
| `J` | decimal string, such as `"-9007199254740993"` |
| `L` | `null`, a string (creates a new `java/lang/String`), `{"object":"class/Name"}`, or `{"array":"B","base64":"AQID"}` |
| `L` (collections) | `{"map":"java/util/HashMap","entries":[["key","value"]]}` or `{"list":"java/util/ArrayList","items":["value",null]}`; keys, values and items are recursive `L` values |

Collection answers create fresh built-in maps and lists; supported classes and snapshot behavior are described in [ENVIRONMENT.md](ENVIRONMENT.md). If building a collection answer fails, every handle created for that answer is released. Arrays accept `Z B C S I J F D` and use the same little-endian bytes as serve's `new_array`. Object, string and array answers create fresh handles each time. A constructor (`new`) needs an `L` answer even though its signature ends in `V`.

In Rust, load a file with `Policy::load(path)` or JSON text with `Policy::parse(text)`, then wrap your existing handler:

```rust
let mut policy = unidroid::Policy::load("java.json")?;
let mut handler = policy.handler(&mut env);
let result = emu.call_jni_onload(m, &mut handler);
handler.finish(result)?;
```

Call `finish(result)` after each emulator call to surface policy errors across `JniHandler`'s `Option` return type. Uncovered calls and debugger stops go to `env`. Sequence positions stay with the policy across calls and new handler wrappers; loading or parsing a new policy starts them over. The library serve entry point is `serve::serve(&mut emu, Some(policy), input, output)`; pass `None` when no policy is needed. `serve::Options::load_policy()` reads its optional `policy` path; `serve::record` loads it automatically. Without a policy, no policy rules, sequence state or hooks are allocated.

### `batch`: run many inputs and compare reports

```bash
target/release/unidroid batch --setup setup.jsonl --workers 4 \
  --fixed-clock 1700000000 --random-seed 42 jobs.jsonl > report.json
```

`jobs.jsonl` has one job per line. Each worker runs serve commands in order, using a policy file for Java answers. An unanswered Java call fails the command with the usual `unhandled JNI call: ...` error and crash report. A job stops at its first failing command; other jobs continue.

Load the library in `--setup` so jobs share a checkpoint; without a library loaded by setup, every attempt starts a fresh emulator and reruns setup. Every worker completes its initial setup before any job runs. A setup failure exits 1 with `error: setup command N: <message>` on stderr and no report; command numbers here start at 1.

For example, `setup.jsonl` can contain:

```json
{"cmd":"load","path":"rootfs/lib64/libc.so"}
{"cmd":"find_symbol","module":{"$ref":0},"name":"strlen"}
```

And `jobs.jsonl`:

```json
{"id":"hello","commands":[{"cmd":"new_cstring","value":"hello"},{"cmd":"call_function","address":{"$setup":1},"args":[{"$ref":0}]}]}
{"id":"world!","commands":[{"cmd":"new_cstring","value":"world!"},{"cmd":"call_function","address":{"$setup":1},"args":[{"$ref":0}]}],"coverage":["libc.so"],"diff":true}
```

Anywhere inside a command, an object containing only `{"$ref":N}` uses the `ok` value of this job's earlier command N; `{"$setup":N}` uses setup command N in that worker. Both indices start at 0. Setup commands can use `$ref` for earlier setup results. Missing or forward references fail with `bad reference ...`. When setup loads no library, `$setup` refers to the results of that attempt's setup run.

| job field | meaning |
|---|---|
| `commands` | required array of serve commands |
| `id` | any JSON value; defaults to the 1-based input line number |
| `policy` | policy file for this job, replacing `--policy`; each attempt starts at its first rule position |
| `coverage` | list of coverage specs added to the current trace settings; returns this attempt's block counts in serve's `coverage` JSON format |
| `diff` | `true` captures memory and registers before and after the job, including a failed job |

Batch accepts `--rootfs`, `--process-name`, repeatable `--prop`, `--alias`, `--file` and `--superclass`, `--policy`, `--translation-cache-mb`, `--fixed-clock` and `--random-seed` as described above, plus:

| option | meaning |
|---|---|
| `--setup FILE` | optional JSON Lines of serve commands to prepare each worker |
| `--workers N` | worker threads; defaults to `std::thread::available_parallelism()` (1 if unavailable) |
| `--retries N` | up to N extra attempts after a job fails; default 0 |
| `--only ok\|failed` | include only matching jobs in the report; summary and exit code still count all jobs |
| `--log-format text\|json` | stderr diagnostics in text (default) or JSON |

Each checkpoint restores guest state and the batch policy's positions after setup. Policy files are loaded once per path per worker; jobs with their own policy use a fresh clone. Coverage counts and JNI crash history start fresh on each attempt. A job's `trace` settings apply only to that attempt; the worker restores its setup trace settings afterward. Diagnostics are not tagged with a job; `--workers 1` keeps them in job order.

Batch refuses `break`, `unbreak`, `watch`, `unwatch`, `tracepoint`, `untracepoint`, `continue`, `step`, `finish`, `return`, `abort`, `answer`, `unhandled`, `checkpoint`, `rewind` and `drop_checkpoint` in both jobs and setup, with `<cmd> is not available in batch`. Other commands use the same parser, execution and response encoding as serve.

Stdout contains one JSON document: `{"summary":{"jobs":N,"ok":N,"failed":N,"ms":N},"jobs":[...]}`. Jobs stay in input order. Each has `id`, `ok` (boolean), `results` (the `ok` values of completed commands), integer `ms` for its last attempt, and `attempts`. Failed jobs also have `error`, and `crash` when serve would supply one. Malformed JSON or command syntax fails only that job with `bad job: ...`. `summary.ms` measures the whole batch. Exit code is 0 if every job succeeds, otherwise 1.

Requested `coverage` is a list of `{address, location, count}`. Requested `diff` contains `registers` (name to `[before, after]` decimal strings), `mapped` and `unmapped` (lists of `{start, end}` decimal strings), and `changes` (a list of `{address, location, size, after}`; address is a decimal string, size is bytes, and after is base64). Lists keep the library `Diff` order. Before the first library load, the diff baseline has no CPU registers or mapped memory.

To compare reports, keep the same libraries, rootfs, options, policy and trace settings, set `--fixed-clock` and `--random-seed`, and remove the timing fields, for example with `jq -S 'del(.summary.ms, .jobs[].ms)' report.json`. See [DEBUGGING.md](DEBUGGING.md) for other inputs that affect deterministic runs.

The library entry point is `serve::batch(&options, &batch_options, builder_factory, jobs) -> Result<serde_json::Value>`, with `serve::Options`, `serve::BatchOptions` and a `BufRead` jobs source. `BatchOptions::default()` supplies the CLI defaults; `only` takes `serve::BatchOnly::Ok` or `Failed`. The factory is `Fn() -> EmulatorBuilder + Sync`, such as `Emulator::builder`; it runs inside each worker so builders, emulators and event handlers can stay on their own thread. Setup errors return `Err`; failed jobs are recorded in the returned report.

### `fields`: which inputs change which output bytes

```bash
target/release/unidroid fields --workers 4 --fixed-clock 1700000000 \
  --random-seed 42 spec.json > fields.json
```

`spec.json` is one JSON object. `setup` is optional; it and `commands` use batch's serve commands, `$ref` / `$setup` references and refused commands. Load a library in setup to restore each worker's checkpoint before every run; without one, batch rebuilds the emulator and reruns setup. Fields uses the same batch runner and accepts its emulator options, `--workers` (with the same default) and `--log-format`.

```json
{
  "commands": [
    {"cmd":"new_string", "value":"12345678:1234567890"},
    {"cmd":"string", "handle":{"$ref":0}}
  ],
  "output": 1,
  "fields": [
    {"name":"uid", "command":0, "offset":0, "length":8},
    {"name":"ts", "command":0, "offset":9, "length":10, "values":["MTIzNDU2Nzg5MQ=="]}
  ]
}
```

`output` selects a zero-based command index, defaulting to the last command. Its `ok` value becomes bytes: base64-decoded for `read` or `array`, UTF-8 for `string`, and UTF-8 compact JSON text for every other command (including JSON quotes around string values).

Each field selects a byte range in one command's payload: decoded `base64` for `write` / `new_array`, or the UTF-8 bytes of `value` for `new_string` / `new_cstring`. An unsupported command, invalid payload or out-of-range field fails the spec before any setup runs and names the field. Indices and offsets start at zero.

By default, one run XORs every byte of the field with `0x01`; ASCII digits and letters stay printable. With `values`, each base64 value replaces that range in a separate run and must decode to exactly `length` bytes. Invalid base64 or a wrong length fails the spec. An empty list requests no runs. A mutation that makes a string invalid UTF-8 fails only that run. Each run changes only one field, starting from the original commands.

The unmodified job runs twice before any field jobs. A failed base run exits 1 with its error; differing base outputs exit 1 with `error: the unmodified run is not repeatable; use --fixed-clock and --random-seed`. Spec and setup errors also exit 1. These errors go to stderr without a report. Otherwise stdout is one JSON document and exit status is 0, even if field runs fail:

```json
{
  "output":{"length":19,"base64":"MTIzNDU2Nzg6MTIzNDU2Nzg5MA=="},
  "fields":[
    {"name":"uid","runs":1,"changed":[[0,8]]},
    {"name":"ts","runs":1,"changed":[[18,19]]}
  ],
  "by_output":[{"start":0,"end":8,"fields":["uid"]},{"start":18,"end":19,"fields":["ts"]}],
  "unaffected":[[8,18]]
}
```

`changed` is the union of changed byte offsets across successful runs, merged into adjacent half-open `[start,end)` ranges. If output length changes, added or removed bytes count as changed and the field includes `length_changed: true`; `changed` can extend beyond the base length, while `by_output` and `unaffected` describe only `[0, output.length)`. Failed runs contribute no offsets and add messages such as `"value 0: ..."` to the field's `errors`; value indices start at zero, including the default mutation. `runs` counts all requested variants, including failures. Absent errors and false `length_changed` are omitted.

Fields stay in input order. `by_output` groups consecutive base bytes with the same nonempty field set, in offset order; each set keeps input field order. `unaffected` lists base ranges that no successful run changed.

Only the changes tried are observed; an unchanged byte may still depend on a field for other values. A hash-like output that changes everywhere for an input change shows that field on every byte. Non-repeatable runs need `--fixed-clock` and `--random-seed`; other external inputs must also stay the same, as described in [DEBUGGING.md](DEBUGGING.md).

The library entry point is `serve::fields(&options, &fields_options, builder_factory, spec) -> Result<serde_json::Value>`, with `serve::Options`, `serve::FieldsOptions { workers }` and a `serde_json::Value` spec. `FieldsOptions::default()` supplies the CLI worker default. The factory is `Fn() -> EmulatorBuilder + Sync`, like batch's. Implementation: [src/serve/fields.rs](src/serve/fields.rs).

### Record a session and replay it

See [recording and replay](DEBUGGING.md#record-a-session-and-replay-it) for the session format and replay command.

### `diff`: compare two memory snapshots

See [the `diff` command](DEBUGGING.md#diff-compare-two-memory-snapshots) to compare saved snapshots.

## What it does and doesn't support

Supported:

- Android arm64 (`arm64-v8a`) and 32-bit ARM (`armeabi-v7a`, both ARM and Thumb code), using Android 6.0 (API 23) system libraries
- Loading a `.so` from the host or an APK and its dependencies, relocation (including Android packed relocations), running constructors
- libdl (`dlopen` / `dlsym` and the rest), system properties, the common syscalls including `write`/`writev` to stdout and stderr, `uname` and `faccessat` (see [files and system calls](ENVIRONMENT.md#files-and-system-calls))
- JNI: classes, methods, fields, strings, arrays, exceptions, global and local references, `RegisterNatives`. Strings and arrays handed to native code come from libc's own `malloc`, as on a phone, so native code may `free` them.
- Guest threads (`pthread_create`, `pthread_join`, and the futex waits behind locks). They take turns on one CPU and switch about every 10,000 instructions, like unidbg. Switching keeps the translated code. A thread that is still running when your call returns keeps going during your next call. An instance that never starts a thread pays nothing for this.
- The 32-bit ARM kernel helpers at `0xffff0f60`–`0xffff0fff`, which old libraries and Android 6.0's own libc use
- Several instances in one process

Not supported:

- iOS
- Signals and `fork`
- Writing to the host file system. Guest files resolve through an in-memory overlay, generated files and read-only `rootfs`; all writes stay in memory and host files are never changed. Added files, writable files, directories and links are covered in [ENVIRONMENT.md](ENVIRONMENT.md).

A library's dependencies are looked for first in the APK's ABI folders (base, then the neighboring architecture split), then in the folders of loaded host libraries, then in rootfs. `load_apk_library("libfoo.so")` and guest `dlopen` use that same name search; `dlopen` first honors an existing explicit rootfs path without `..`. Without an APK, host folders and rootfs work as before. See [ENVIRONMENT.md](ENVIRONMENT.md) for ABI selection, metadata and guest paths.

A dependency found in none of these locations (such as `libEGL.so`) only prints a warning. Calling an unresolved function stops and names it; with an APK, built-in asset functions work even without `libandroid.so`.

## Directories

- `src/`: the emulator, and the `unidroid` command (`main.rs`, `serve.rs`)
- `examples/ts/`, `examples/python/`: using the command from TypeScript and Python
- `rootfs/lib/`, `rootfs/lib64/`: Android 6.0 system libraries (32-bit and 64-bit), from unidbg
- `tests/`: tests, including one per sample, checked byte for byte against unidbg
- `bench/`: the unidbg comparison program and script

## License

Apache License 2.0, same as unidbg. See `LICENSE`. Credits are in `NOTICE`.
