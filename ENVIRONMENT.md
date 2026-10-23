# The Android environment

unidroid emulates Android for native libraries; you can replace imported C calls with host logic.

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
