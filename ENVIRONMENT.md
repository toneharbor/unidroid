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

Guest paths resolve through three layers: an in-memory overlay, generated files, then read-only `rootfs`. The overlay holds added files, guest changes, directories and whiteouts. Removing a path hides its lower-layer entry and descendants. Recreating an overlay directory there does not bring its old lower-layer children back. Guest writes never create, change or remove host files.

The Rust file API works before loading a library, without a CPU:

```rust
pub fn add_file(&mut self, path: &str, bytes: &[u8]) -> Result<()>;
pub fn add_host_file(&mut self, path: &str, host_path: impl AsRef<Path>) -> Result<()>;
pub fn remove_file(&mut self, path: &str) -> Result<()>;
pub fn file(&self, path: &str) -> Result<Vec<u8>>;
```

`add_file` creates or replaces an overlay file with mode `0o100644`. Both add methods create missing parent directories with mode `0o040755`. `add_host_file` requires an existing regular host file, stores its resolved host path and opens it on demand; it does not read the contents into memory. `remove_file` removes overlay entries and whiteouts the path, including a path that exists only in rootfs. `file` reads the current regular-file contents through all three layers and errors for missing paths and directories. Host API paths must be absolute; errors name the guest path and errno, and host-side failures from `add_host_file` also name the host path and host error.

Serve exposes `add_file` (`path`, `base64`), `add_host_file` (`path`, `host_path`), `remove_file` (`path`) and `file` (`path`). The first three return `null`; `file` returns base64. Repeatable `--file GUEST=HOST` calls `add_host_file` before loading. It is part of shared `serve::Options`, including recordings and replay. See [README.md](README.md) for command rows.

Paths are normalized lexically: repeated slashes and `.` disappear; `..` removes one component and stops at `/`. Relative paths use working directory `/`, or the directory fd for `openat`, `newfstatat`, `faccessat`, `mkdirat`, `unlinkat`, `renameat` and `readlinkat`. Invalid relative-path dirfds return `EBADF`; non-directory fds return `ENOTDIR`. Absolute paths ignore dirfd. `faccessat` checks existence through the layers and still ignores permission modes and flags.

`openat` accepts read and write access. A write open of a rootfs or host-linked file copies it into memory; `O_TRUNC` skips reading its old contents. `O_CREAT` creates a file only when its parent exists, with `(mode & 0o7777) | 0o100000`. `O_CREAT|O_EXCL` rejects an existing path with `EEXIST`; directories reject write opens with `EISDIR`; generated files reject them with `EACCES`. `O_APPEND` writes at the end, and writes past the end zero-fill the gap. Separate opens share file contents but keep separate offsets. `write`, `writev` and `pwrite64` accept at most 33554431 bytes per call; read-only fds reject writes with `EBADF`. `pwrite64` keeps the fd offset unchanged. As on Linux, append mode also places positional writes at the end. `ftruncate` changes the length; `fsync` and `fdatasync` return zero for any open fd, without writing to the host.

Overlay files have a 1 GiB maximum: writes starting at or beyond it and `ftruncate` beyond it return `EFBIG`, while writes crossing it return the shorter count that fits.

`mkdirat` creates overlay directories with `(mode & 0o7777) | 0o040000`. `unlinkat` removes files, or empty directories with `AT_REMOVEDIR`; nonempty directories return `ENOTEMPTY`. `renameat` moves files and directory trees and replaces a compatible target, copying rootfs sources into the overlay and hiding their old paths. Arm64 also supports `renameat2` with flags zero.

Opening a directory, with or without `O_DIRECTORY`, takes a snapshot of its merged entries; `O_DIRECTORY` on a regular file returns `ENOTDIR`. `read` returns `EISDIR`; `getdents64` returns `.` and `..`, then entries sorted by name bytes, excluding whiteouts. Records use the Linux 64-bit layout and eight-byte alignment, with the next entry index as `d_off`. Calls walk the snapshot; zero means end of directory; a buffer too small for the next record returns `EINVAL` if nothing fit. `lseek(fd, 0, SEEK_SET)` restarts it. Directory enumeration works through libc's `opendir`, `readdir` and `closedir`; `/proc/self/fd` contents need not be enumerated.

`fcntl` supports `F_DUPFD`, `F_DUPFD_CLOEXEC`, `F_GETFD`, `F_SETFD`, `F_GETFL` and `F_SETFL`. Status flags hold access mode, `O_APPEND` and `O_NONBLOCK`; `F_SETFL` changes only the latter two. Duplicates share offsets and status flags, but each fd has its own close-on-exec flag. `dup` and `dup3` work, including libc's `dup2`; equal fds passed to `dup3` return `EINVAL`. Descriptors 0–2 start open and can be duplicated or closed. New descriptors use the lowest free number. Duplicated stdout/stderr descriptors emit the original stream's `Output` event; closing and reusing an fd changes what it refers to.

`/proc/self/cmdline` and `/proc/1234/cmdline` snapshot the process name followed by one NUL byte when opened. `1234` is the guest process ID. `readlinkat` on `/proc/self/fd/N` or `/proc/1234/fd/N` returns the normalized path used to open that fd, or `ENOENT` if closed. Initial standard streams use `/dev/stdin`, `/dev/stdout` and `/dev/stderr`. `/proc/self/exe` returns `/system/bin/app_process64` or `/system/bin/app_process32`; a rootfs symlink returns its host target, including for a dangling link. Existing non-links return `EINVAL`, missing paths `ENOENT`. Results contain at most `bufsiz` bytes, without a terminating NUL, and return the byte count.

The intentional departures from unidbg match Android and Linux: `/proc/self/exe` returns a real Android device's executable path, `readlink` returns the byte count without a terminating NUL, and `faccessat` resolves relative paths against `dirfd`; sorted directory listings make runs deterministic.

Overlay `stat`/`fstat` use the stored mode, content length, one link, device/uid/gid zero, block size 4096 and size rounded up to 512-byte blocks. The inode is the 64-bit FNV-1a hash of the normalized guest path. All timestamps use the fixed-clock seconds when set, otherwise the last change time. Host-linked files use host metadata like rootfs until copied into memory. Generated maps metadata remains as described below; cmdline uses the same generated-file metadata layout.

Checkpoints save the overlay, whiteouts, directory snapshots and open descriptors, including shared offsets, flags and close-on-exec. Unchanged file contents are shared through `Rc` until a write. Rootfs and host-linked files reopen from their saved host paths on rewind; keep those host files available and unchanged. See [DEBUGGING.md](DEBUGGING.md).

Guest `write` and `writev` to stdout/stderr descriptors (initially fd 1 and fd 2) emit `Event::Output { fd, text }` even with tracing off. Each call emits one event, with `writev` buffers concatenated in order before lossy UTF-8 decoding, and returns the number of bytes accepted. Both calls accept at most 33554431 bytes, the same limit as `getrandom` and random-device reads; `writev` shares this limit across entries and can stop partway through an entry. The default event printer writes to host stderr; the builder's `on_event` can capture the output. Text events use `stdout: TEXT` or `stderr: TEXT` and drop one trailing newline; JSON uses `{"event":"output","fd":1,"text":"hello\n"}` and preserves the text. See [DEBUGGING.md](DEBUGGING.md) for events.

Closed or read-only fds, including the initial stdin, reject writes with `-EBADF`; writable file descriptors update the overlay. Unreadable buffers or iovec arrays return `-EFAULT`. `writev` accepts at most 1024 entries (`-EINVAL` above that), each a pointer and length: 16 bytes on arm64, 8 on arm32. Zero-length entries are allowed. Syscalls return negative Linux errno values; libc converts them to `-1` and sets errno.

`uname` writes all 390 bytes of `struct utsname`: six 65-byte fields, each string followed by zero padding. It returns 0, or `-EFAULT` if the buffer cannot be written. Values match unidbg 0.9.9:

| field | value |
|---|---|
| sysname | `Linux` |
| nodename | `localhost` |
| release | `1.0.0-unidbg` |
| version | `#1 SMP PREEMPT Thu Apr 19 14:36:58 CST 2018` |
| machine | `armv8l` on arm64, `armv7l` on arm32 |
| domainname | `localdomain` |

`faccessat` (used by libc's `access`) uses the same path resolution as `openat`: existing files and directories return 0; failures return the resolver's errno, including `-ENOENT` for missing paths, `-EBADF` for invalid relative-path dirfds and `-ENOTDIR` for non-directory dirfds or path components. It resolves relative paths against `dirfd` (or `/` for `AT_FDCWD`) and ignores permission modes and flags. Overlay entries, rootfs paths, random devices and generated files all use this resolver.

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
