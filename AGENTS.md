# unidroid

A low-memory unidbg in Rust on Unicorn: it runs Android arm64 and armv7 `.so` files on a computer. `README.md` covers the library API, the `unidroid` command and the `serve` protocol; `DEBUGGING.md` covers crash reports, breakpoints, watchpoints, traces, snapshots and replay; `ENVIRONMENT.md` covers the Android environment, replacing imported C functions and running constructors yourself.

## Rules

- Debugging a sample: read [skills/unidroid/SKILL.md](skills/unidroid/SKILL.md).
- Keep every hand-written file at 500 lines or fewer. When one would grow past that, split it by responsibility: `src/x.rs` stays the module root, its children go in `src/x/`, and its unit tests in `src/x/tests.rs` (or `src/x/tests/` when they need several files). Generated files (`Cargo.lock`) and binaries are exempt.
- Match unidbg byte for byte. Each sample under `tests/` is checked against unidbg 0.9.9's output, and `bench/compare.sh` re-runs the comparison. unidbg needs Java 21; newer JDKs crash its Unicorn binding.
- Committed tests use only committed fixtures, `rootfs/`, the synthetic ELF in `tests/support/elf.rs`, or code bytes written into guest memory. A test of JNI or CPU behaviour loops over both architectures.
- Errors from guest code name where it happened (`... from libfoo.so+0x1234`, via `Emulator::location`).
- When the library API, a `unidroid` option or a `serve` command changes, update `README.md`, `DEBUGGING.md` or `ENVIRONMENT.md`, whichever covers it, in the same change.
- Stage files by path. Some local fixture folders hold proprietary libraries and captured keys; they stay out of git.

## Checks

Run what CI runs: `.github/workflows/ci.yml`.
