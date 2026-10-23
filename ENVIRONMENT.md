# The Android environment

unidroid emulates Android for native libraries; you can replace imported C calls with host logic.

## Built-in Java classes

Common Java calls are answered before `JniHandler`, policy rules, `Probe` and the serve driver. Instance calls use the receiver's built-in kind plus the method name and JNI signature, so looking up `get` on `java/util/Map` works on a built-in HashMap. Constructors, static methods and static fields use the class name. Uncovered methods, charsets and digest algorithms still reach your handler. Objects created with `new_object(class, value)` remain opaque, even when their class is `java/util/HashMap`; their calls still reach your handler.

| class | built-in calls |
|---|---|
| `java/lang/Object` | `getClass`, `hashCode`, `equals` on built-in objects and strings; `toString` on strings, boxes, StringBuilder, maps and lists when their elements have built-in text |
| `java/lang/Class` | `getName` on class objects, using dots, such as `java.lang.String` |
| `java/lang/String` | empty, byte-array (with optional range and charset name), string-copy and char-array constructors; `getBytes`, `length`, `isEmpty`, `charAt`, `toCharArray`, `equals`, `equalsIgnoreCase`, `hashCode`, `compareTo`, `toString`, `startsWith`, `endsWith`, `contains`, both `indexOf` forms, both `substring` forms, `toLowerCase`, `toUpperCase`, `trim`, `concat`, `replace`; static `valueOf` for int, long, boolean, char and Object |
| `java/lang/CharSequence` | `length` and `toString` on strings and StringBuilders |
| `java/nio/charset/Charset`, `StandardCharsets` | `Charset.forName`, `name`; static fields `UTF_8`, `US_ASCII`, `ISO_8859_1`, `UTF_16LE`, `UTF_16BE` |
| `java/lang/StringBuilder` | empty, string and capacity constructors; `append` for String, Object, int, long, char and boolean; `length`, `toString` |
| `java/lang/Boolean`, `Byte`, `Character`, `Short`, `Integer`, `Long`, `Float`, `Double` | primitive constructors and static `valueOf`; matching primitive accessor and `toString`; `intValue` and `longValue` on Byte, Short, Integer and Long; static `Integer.parseInt` (optional radix), `Long.parseLong`, `Integer.toString`, `Long.toString`, `Integer.toHexString` |
| `java/util/HashMap`, `LinkedHashMap`, `TreeMap` | empty constructors; capacity constructors on HashMap and LinkedHashMap; `put`, `get`, `getOrDefault`, `containsKey`, `remove(Object)`, `size`, `isEmpty`, `clear`, `keySet`, `values`, `entrySet`; entries answer `getKey`, `getValue` |
| `java/util/HashSet` and map views | empty HashSet constructor and `add`; `contains`, `size`, `isEmpty`, `iterator`, `toArray()` on sets and views |
| `java/util/ArrayList`, `LinkedList` | empty constructors; ArrayList capacity constructor; both `add` forms, `get`, `set`, `remove(int)`, `size`, `isEmpty`, `contains`, `indexOf`, `clear`, `iterator`, `toArray()` |
| `java/util/Iterator` | `hasNext`, `next` |
| `java/security/MessageDigest` | static `getInstance`; `update(byte[])`, `update(byte[], offset, length)`, `update(byte)`; `digest()`, `digest(byte[])`, `reset`, `getAlgorithm` |
| `android/util/Base64` | static `encodeToString(byte[], flags)`, `encode(byte[], flags)`, `decode(String, flags)`, `decode(byte[], flags)` |

String lengths and indexes count UTF-16 units, including each half of a surrogate pair. Supported charsets are UTF-8 (the default), US-ASCII, ISO-8859-1, UTF-16LE and UTF-16BE; names ignore case, hyphens and underscores. Object forms of String `valueOf` and StringBuilder `append` use built-in `toString`, with `"null"` for null. Unknown element text falls through to your handler. Map and list text uses Java's `{key=value, ...}` and `[item, ...]` forms.

String keys compare by UTF-16 content; boxed keys compare by primitive type and Java value, including Java's NaN and signed-zero rules. Other keys compare by object identity, including across different handles to the same object. HashMap and HashSet use Java string and box hashes, a power-of-two table starting at 16 by default, a 0.75 load factor and bucket order. Entries within each bucket keep insertion order, including through resizing; removal and reinsertion moves a key to the end of its bucket. Long buckets are not converted to trees. LinkedHashMap keeps insertion order. TreeMap sorts strings by UTF-16 and comparable boxed keys by value; unsupported comparisons fall through to the handler, or return an error from `new_java_map`.

Map views (`keySet`, `values`, `entrySet`) and iterators are **snapshots taken when created**. This differs from Java's live views. They keep references to their elements. Releasing an element's host handle does not remove it from a collection. All built-in object state is copied by checkpoints, preserving aliases and cycles; rewind restores collection contents, iterator positions, StringBuilder contents and digest state.

`IsInstanceOf` and `IsAssignableFrom` recognize built-in class hierarchies after explicit `superclass` declarations. A declared app subclass of HashMap is also a Map. Built-in sources answer false for unrelated `java/` or `javax/` targets; app-specific targets still reach `is_assignable`. `GetSuperclass` continues to use explicit declarations.

Digests support MD5, SHA-1 (also SHA1 and SHA), SHA-224, SHA-256, SHA-384 and SHA-512, ignoring case. `digest` returns bytes and resets the digest. `getAlgorithm` preserves the requested name. Base64 flags are Android's `DEFAULT` 0, `NO_PADDING` 1, `NO_WRAP` 2, `CRLF` 4 and `URL_SAFE` 8. Wrapped encoding breaks after 76 characters; nonempty output ends in exactly one line break, and empty output has none. Decoding accepts padded or unpadded input and skips non-alphabet bytes as Android does. Invalid padding throws `IllegalArgumentException` with `bad base-64`.

Data errors set pending JNI exceptions and return zero or null. These include `NumberFormatException` for invalid numbers, `IndexOutOfBoundsException` for invalid list indexes and `NoSuchElementException` for exhausted iterators. Native code can inspect and clear them with JNI exception functions.

Create and inspect built-in collections and boxes from Rust:

```rust
pub fn new_java_map(&mut self, class: &str, entries: &[(Handle, Handle)]) -> Result<Handle>;
pub fn java_map(&mut self, handle: Handle) -> Result<Vec<(Handle, Handle)>>;
pub fn new_java_list(&mut self, class: &str, items: &[Handle]) -> Result<Handle>;
pub fn java_list(&mut self, handle: Handle) -> Result<Vec<Handle>>;
pub fn new_boxed(&mut self, value: JniValue) -> Result<Handle>;
pub fn unboxed(&self, handle: Handle) -> Result<JniValue>;
```

Map classes are `java/util/HashMap`, `java/util/LinkedHashMap` and `java/util/TreeMap`; list classes are `java/util/ArrayList` and `java/util/LinkedList`. Entries are inserted in the supplied order. `new_boxed` accepts the eight primitive `JniValue` variants and rejects `Object` and `Void`. Wrong classes, invalid handles and wrong object kinds return errors. `java_map` returns iteration order; `java_list` returns list order. Both create fresh host handles to the existing elements, with `Handle(0)` for null. Release the returned handles when done.

```rust
let key = emu.new_string("count");
let value = emu.new_boxed(JniValue::Int(2))?;
let map = emu.new_java_map("java/util/HashMap", &[(key, value)])?;
let entries = emu.java_map(map)?;
assert_eq!(emu.unboxed(entries[0].1)?, JniValue::Int(2));
for handle in [key, value, map, entries[0].0, entries[0].1] {
    emu.release(handle);
}
```

Serve has the same six command names; see the command table and recursive map/list policy values in [README.md](README.md). The SignUtil Rust, Python and TypeScript examples use built-in maps and need no map handlers.

## Files and system calls

Guest `write` and `writev` to fd 1 (stdout) and fd 2 (stderr) emit `Event::Output { fd, text }` even with tracing off. Each call emits one event, with `writev` buffers concatenated in order before lossy UTF-8 decoding, and returns the number of bytes accepted. Both calls accept at most 33554431 bytes, the same limit as `getrandom` and random-device reads; `writev` shares this limit across entries and can stop partway through an entry. The default event printer writes to host stderr; the builder's `on_event` can capture the output. Text events use `stdout: TEXT` or `stderr: TEXT` and drop one trailing newline; JSON uses `{"event":"output","fd":1,"text":"hello\n"}` and preserves the text. See [DEBUGGING.md](DEBUGGING.md) for events.

Other fds, including stdin and rootfs files, return `-EBADF`. Unreadable buffers or iovec arrays return `-EFAULT`. `writev` accepts at most 1024 entries (`-EINVAL` above that), each a pointer and length: 16 bytes on arm64, 8 on arm32. Zero-length entries are allowed. Syscalls return negative Linux errno values; libc converts them to `-1` and sets errno.

`uname` writes all 390 bytes of `struct utsname`: six 65-byte fields, each string followed by zero padding. It returns 0, or `-EFAULT` if the buffer cannot be written. Values match unidbg 0.9.9:

| field | value |
|---|---|
| sysname | `Linux` |
| nodename | `localhost` |
| release | `1.0.0-unidbg` |
| version | `#1 SMP PREEMPT Thu Apr 19 14:36:58 CST 2018` |
| machine | `armv8l` on arm64, `armv7l` on arm32 |
| domainname | `localdomain` |

`faccessat` (used by libc's `access`) uses the same path resolution as `openat`: a file that opens read-only returns 0, otherwise `-ENOENT`. It ignores `dirfd`, `mode` and `flags`, as unidbg does. Rootfs paths, random devices and the generated maps files all use this resolver.

Opening `/proc/self/maps`, `/proc/1234/maps` or `/proc/self/task/1234/maps` generates an in-memory snapshot of `Emulator::maps()`. `1234` is the main thread ID and the value returned by `getpid`. Each mapping has one line, sorted by start address, with an exclusive end address. The line format is:

```rust
format!("{start:08x}-{end:08x} {perms}p 00000000 {dev} 0          {label}\n")
```

Addresses are lowercase hex, at least eight digits; `perms` is the three `rwx`/`-` characters. There are ten spaces after inode `0`.

- All loaded modules, including app libraries, use device `b3:19` and label `/system/lib64/NAME` on arm64 or `/system/lib/NAME` on arm32. A module name without a `.so` suffix uses `/system/bin/NAME` on either architecture.
- The stack uses device `00:00` and label `[stack]`.
- Every other mapping uses device `00:00` and label `anonymous`.

The open file keeps its original bytes when mappings change. It supports `read`, `pread64`, `lseek` with SET/CUR/END, `fstat`, `mmap` and `close`; `newfstatat` and `faccessat` also recognize these paths. Checkpoints retain each open maps file's bytes and offset, and rewind restores both, including into a fresh emulator.

Maps file metadata follows unidbg's `ByteArrayFileIO.fstat`: `st_dev = 1`, `st_ino = 1`, `st_mode = 0o100000` (`S_IFREG`, no permission bits), `st_nlink = 0`, uid/gid/rdev zero, `st_size` equal to the snapshot length, `st_blksize = 4096`, and `st_blocks` equal to the length rounded up to 4096-byte units. All three timestamps use the supplied `fixed_clock` seconds with zero nanoseconds, otherwise the current time when stat is called. These values apply on both architectures; host files keep their existing metadata behavior.

## Replacing imported C functions

This handler returns `EGL_NO_DISPLAY` for `eglGetDisplay` when libEGL is missing. Other hooked calls log their name and caller, then run the original.

```rust
use unidroid::{Emulator, ImportCall, JniHandler};
struct Imports;
impl JniHandler for Imports {
    fn import(&mut self, emu: &mut Emulator, call: &ImportCall<'_>) -> Option<u64> {
        if call.name == "eglGetDisplay" { return Some(0); } // No original; no display.
        eprintln!("{} from {}", call.name, emu.location(call.return_address));
        None
    }
}

let mut emu = Emulator::builder().rootfs("rootfs").build()?;
emu.hook_import("eglGetDisplay")?;
emu.hook_import("getuid")?;
let module = emu.load_library_without_constructors("libfoo.so")?;
emu.run_constructors(module, &mut Imports)?;
```

To wrap a call, use `emu.call_function_with(self, call.original?, call.args)` inside the handler, handle its `Result`, then change the value and return `Some(value)`.

`hook_import(name)` works before or after loading. Repeated names do nothing. Relocations and symbol lookups, including `dlsym`, bind the hook. Hooks survive checkpoints, including rewind into a fresh emulator.

`ImportCall` has `name`, raw `args` (x0–x7 or r0–r3), `original: Option<u64>` and `return_address` (lr). The original is the replaced built-in override, otherwise the first loaded export in load order. Exports are looked up at call time, with the Thumb bit intact.

`Some(value)` returns through x0 or r0/r1. `None` tail-jumps to the original with registers, stack arguments and lr intact. This preserves variadic and floating-point calls. With no original, it errors with the symbol and caller. Wrappers with stack arguments must supply those words to the nested call too.

Loading without constructors lets you install hooks or breakpoints first. `run_constructors` runs pending dependencies depth first, then the module’s `DT_INIT` and `DT_INIT_ARRAY`. Modules already initialized or initializing are skipped. A guest `dlopen` of a library loaded with `load_library_without_constructors` runs its pending constructors with the caller’s handler.

JNI calls, `call_function_with` and `run_constructors` carry your handler. `load_library` runs constructors with `()`; `call_function` also uses `()`, so hooks pass through there. `PolicyHandler` forwards imports to its fallback. `Probe` passes through.
