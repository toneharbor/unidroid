# minidbg

A low-memory unidbg. It runs Android `.so` files on a computer, no phone needed. Both 64-bit (`arm64-v8a`) and 32-bit (`armeabi-v7a`) work. The most common use is calling an app's signing or encryption functions.

It's written in Rust on top of Unicorn, the same CPU emulator unidbg uses. There's no Java VM.

## How much it saves

Both run unidbg's own sample `.so` files and give exactly the same results. Measured on an Apple Silicon Mac, peak memory (max RSS):

| sample | unidbg 0.9.9 | minidbg |
|---|---|---|
| JNA `libjnidispatch.so`, 1 instance | 91 MB | 7.9 MB |
| Telegram `libtmessages.29.so`, 1 instance | 131 MB | 15.4 MB |
| Telegram, 4 instances in one process | 258 MB | 26.6 MB |
| JNA `libjnidispatch.so`, 32-bit, 1 instance | 93 MB | 9.1 MB |
| ttEncrypt `libttEncrypt.so`, 32-bit, 1 instance | 92 MB | 8.8 MB |
| ttEncrypt, 32-bit, 4 instances in one process | 106 MB | 20.4 MB |

It's also about 3x faster. Telegram's PBKDF2 at 100,000 iterations: unidbg about 4.5 s, minidbg about 1.4 s.

Why it's smaller:

- No Java VM.
- A library's read-only parts (code, constants) are never copied. They're mapped straight from the file. Code that never runs never uses memory.
- Several instances in one process share a single copy of that read-only code. Each instance keeps only its own writable data.

Re-measure: `bench/compare.sh 1 4`. It runs unidbg and minidbg side by side and checks that their outputs match. It needs Java 21 and Maven.

## Usage

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
println!("{}", emu.string(out.object()));
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

Objects you create (strings, arrays, `malloc` memory) live until you free them with `release` / `free`. In a long-running service, remember to free them.

## What it does and doesn't support

Supported:

- Android arm64 (`arm64-v8a`) and 32-bit ARM (`armeabi-v7a`, both ARM and Thumb code), using Android 6.0 (API 23) system libraries
- Loading a `.so` and its dependencies, relocation (including Android packed relocations), running constructors
- libdl (`dlopen` / `dlsym` and the rest), system properties, the common syscalls
- JNI: classes, methods, fields, strings, arrays, exceptions, global and local references, `RegisterNatives`
- Several instances in one process

Not supported:

- iOS
- Guest threads: `pthread_create` fails
- Debugger, inline hooks
- File access outside `rootfs`. `/dev/urandom` is allowed. Every other path is "not found".

A dependency missing from rootfs (such as `libEGL.so`) only prints a warning. If its function is actually called, it stops and names the function.

## Directories

- `src/`: the emulator
- `rootfs/lib/`, `rootfs/lib64/`: Android 6.0 system libraries (32-bit and 64-bit), from unidbg
- `tests/`: tests, including one per sample, checked byte for byte against unidbg
- `bench/`: the unidbg comparison program and script

## License

Apache License 2.0, same as unidbg. See `LICENSE`. Credits are in `NOTICE`.
