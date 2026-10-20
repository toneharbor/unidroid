# minidbg

[![CI](https://github.com/zhoupb01/minidbg/actions/workflows/ci.yml/badge.svg)](https://github.com/zhoupb01/minidbg/actions/workflows/ci.yml)

A low-memory unidbg. It runs Android `.so` files on a computer, no phone needed. Both 64-bit (`arm64-v8a`) and 32-bit (`armeabi-v7a`) work. The most common use is calling an app's signing or encryption functions.

It's written in Rust on top of Unicorn, the same CPU emulator unidbg uses. There's no Java VM. Use it as a Rust library, or from any other language through the `minidbg` command.

## How much it saves

Both run nine of unidbg's own sample `.so` files and give exactly the same results. Measured on an Apple Silicon Mac, peak memory (max RSS):

| sample | unidbg 0.9.9 | minidbg |
|---|---|---|
| JNA `libjnidispatch.so`, 1 instance | 91 MB | 7.9 MB |
| Telegram `libtmessages.29.so`, 1 instance | 131 MB | 15.4 MB |
| Telegram, 4 instances in one process | 258 MB | 26.6 MB |
| JNA `libjnidispatch.so`, 32-bit, 1 instance | 93 MB | 9.1 MB |
| ttEncrypt `libttEncrypt.so`, 32-bit, 1 instance | 92 MB | 8.8 MB |
| ttEncrypt, 32-bit, 4 instances in one process | 106 MB | 20.4 MB |
| Telegram `libtmessages.29.so`, 32-bit, 1 instance | 128 MB | 16.0 MB |
| Anjuke SignUtil `libsignutil.so`, 32-bit, 1 instance | 94 MB | 10.5 MB |
| QDReader `libd-lib.so`, 32-bit, 1 instance | 97 MB | 11.8 MB |
| Kanxue crackme `libnative-lib.so`, 32-bit, 1 instance | 317 MB | 11.4 MB |
| Kanxue crackme, 32-bit, 4 instances in one process | 411 MB | 25.7 MB |
| Thread test `libthread-lib.so`, 32-bit, 1 instance | 99 MB | 12.4 MB |

It's also about 3x faster. Telegram's PBKDF2 at 100,000 iterations: unidbg about 4.5 s, minidbg about 1.4 s. The Kanxue crackme tries 64,589 inputs: unidbg about 3 s, minidbg about 1 s.

Why it's smaller:

- No Java VM.
- A library's read-only parts (code, constants) are never copied. They're mapped straight from the file. Code that never runs never uses memory.
- Several instances in one process share a single copy of that read-only code. Each instance keeps only its own writable data.

Re-measure: `bench/compare.sh 1 4`. It runs unidbg and minidbg side by side and checks that their outputs match. It needs Java 21 and Maven.

## Usage

Add it to a Rust project:

```bash
cargo add minidbg
```

The crate doesn't include `rootfs`. Copy the `rootfs` directory from this repository, or take it from a release download.

```rust
use minidbg::{Emulator, JniHandler, JniCall, Handle, JniValue};

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
    .verbose(true)                        // Print every JNI call, handy while setting up
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

32-bit or 64-bit is picked by the first library you load. There's no setting for it. Load that library before `malloc`, `read_bytes`, `write_bytes` or `call_function`; before it, they return an error. One emulator runs one kind: loading a 64-bit library after a 32-bit one is an error.

On 32-bit, `call_function` passes each argument as one 32-bit word and returns only `r0`. To pass a 64-bit value, give it as two words, low word first. If it would start at an odd word position (counting from 0), put a `0` word in front of it. `call_static_jni_method` and `call_jni_method` handle this for you.

Objects you create (strings, arrays, `malloc` memory) live until you free them with `release` / `free`. In a long-running service, remember to free them. Reading a handle that was released, or that holds another kind of object, returns an error.

If a library needs a function that rootfs has under another name, point it there before loading. For example, Android 6.0's libcrypto has no `DES_set_key_unchecked`, only `DES_set_key`:

```rust
emu.symbol_alias("DES_set_key_unchecked", "DES_set_key");
emu.load_library("libd-lib.so")?;
```

The alias is used only when no loaded library has the real name.

There are no Java classes, so minidbg doesn't know which class extends which. If a library calls `GetSuperclass`, or registers a native method on a parent class and you call it through a child class, tell minidbg the parents first:

```rust
emu.superclass("com/example/Sign", "com/example/Base");
emu.superclass("com/example/Base", "java/lang/Object");
```

Now `GetSuperclass` answers, and `call_static_jni_method(.., "com/example/Sign", ..)` also finds a native method registered on `com/example/Base`. Without it, `GetSuperclass` stops and says which class to declare.

## The `minidbg` command

For other languages (TypeScript, Python, Go, ...) there's a command-line program. Download it from [Releases](https://github.com/zhoupb01/minidbg/releases): there are builds for Linux (x86_64, arm64) and macOS (Apple Silicon). Each download has `rootfs` inside. Or build it yourself:

```bash
cargo build --release
```

It lands at `target/release/minidbg`. It has two subcommands.

### `inspect`: look at a library first

```bash
target/release/minidbg inspect --rootfs rootfs libfoo.so
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

### `serve`: drive it from your program

```bash
target/release/minidbg serve --rootfs rootfs
```

Your program starts this as a child process. Then the two talk through stdin and stdout, one JSON object per line. Logs go to stderr, so they never mix with the messages.

You send commands. minidbg answers each one with `{"ok": ...}` or `{"error": "..."}`. When the library calls Java in the middle of a command, minidbg sends a `{"jni": ...}` question instead, and you answer it. This is the same job `JniHandler` does in Rust, written in your own language:

Part of a real run of the SignUtil example (`→` you send, `←` minidbg replies; some lines left out):

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

The same program works for local testing and inside an HTTP service: the service starts `minidbg serve` and answers the Java calls the same way. One `serve` process runs one instance, one command at a time. For calls in parallel, start several.

Options, the same for both subcommands:

| option | what it does |
|---|---|
| `--rootfs DIR` | Android system libraries (default `rootfs`) |
| `--process-name NAME` | process name the library sees |
| `--prop KEY=VALUE` | a system property, repeatable |
| `--alias NAME=TARGET` | same as `symbol_alias`, repeatable |
| `--superclass CLASS=SUPERCLASS` | same as `superclass`, repeatable |
| `--verbose` | print every JNI call on stderr (`inspect` always does) |

How values are written in JSON:

| Java type | JSON |
|---|---|
| object, string, array | handle number (`0` is `null`) |
| `boolean` | `true` / `false` |
| `byte`, `char`, `short`, `int`, `float`, `double` | number |
| `long` | decimal string, like `"-12"` (a JS number can't hold 64 bits) |

Addresses are decimal strings too. Bytes are base64.

Commands:

| command | fields | result |
|---|---|---|
| `load` | `path` | module number |
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

## What it does and doesn't support

Supported:

- Android arm64 (`arm64-v8a`) and 32-bit ARM (`armeabi-v7a`, both ARM and Thumb code), using Android 6.0 (API 23) system libraries
- Loading a `.so` and its dependencies, relocation (including Android packed relocations), running constructors
- libdl (`dlopen` / `dlsym` and the rest), system properties, the common syscalls
- JNI: classes, methods, fields, strings, arrays, exceptions, global and local references, `RegisterNatives`. Strings and arrays handed to native code come from libc's own `malloc`, as on a phone, so native code may `free` them.
- Guest threads (`pthread_create`, `pthread_join`, and the futex waits behind locks). They take turns on one CPU and switch every 10,000 instructions, like unidbg. A thread that is still running when your call returns keeps going during your next call. An instance that never starts a thread pays nothing for this.
- The 32-bit ARM kernel helpers at `0xffff0f60`–`0xffff0fff`, which old libraries and Android 6.0's own libc use
- Several instances in one process

Not supported:

- iOS
- Signals and `fork`
- Debugger, inline hooks
- File access outside `rootfs`. `/dev/urandom` is allowed. Every other path is "not found".

A library's dependencies are looked for first in the folders of the libraries you loaded, then in rootfs. That's how an app's own `lib/arm64-v8a` folder works on a phone, so copy an app's libraries together and load the one you need. `dlopen` from the library searches the same way.

A dependency found in neither place (such as `libEGL.so`) only prints a warning. If its function is actually called, it stops and names the function.

## Directories

- `src/`: the emulator, and the `minidbg` command (`main.rs`, `serve.rs`)
- `examples/ts/`, `examples/python/`: using the command from TypeScript and Python
- `rootfs/lib/`, `rootfs/lib64/`: Android 6.0 system libraries (32-bit and 64-bit), from unidbg
- `tests/`: tests, including one per sample, checked byte for byte against unidbg
- `bench/`: the unidbg comparison program and script

## License

Apache License 2.0, same as unidbg. See `LICENSE`. Credits are in `NOTICE`.
