---
name: unidroid
description: >-
  Run Android .so files with unidroid: call JNI or C functions, drive signing or encryption functions, and answer Java calls. Debug libraries that crash, stop on an unhandled JNI call, or return a wrong result.
---

# Run and debug with unidroid

## 1. Inspect before writing a driver

Read the command section of [README.md](../../README.md) for the options, commands, fields and JSON shapes you need. In a release download, README.md is next to the binary. If this skill was copied elsewhere, use the README from your unidroid checkout or download; the [online README](https://github.com/zhoupb01/unidroid/blob/main/README.md) describes main.

Run `inspect`, then `inspect --probe` on the target library. These commands use the committed SignUtil sample, from the repo root:

```bash
target/release/unidroid inspect --rootfs rootfs tests/fixtures/armeabi-v7a/libsignutil.so
target/release/unidroid inspect --rootfs rootfs --probe tests/fixtures/armeabi-v7a/libsignutil.so
```

Read the architecture, dependencies, `Java_` exports, registered natives and Java calls reached during `JNI_OnLoad`. Get an exported function's signature from its Java declaration or C header/call site. Keep app dependencies together so the loader can find them. Treat a missing dependency warning as a lead; it becomes a failure when the library calls one of its functions.

Treat probe results as a lower bound. `Probe` answers null, zero, false or void. A null can end `JNI_OnLoad` early, so an empty list only describes the path reached with those answers.

**Done when:** you have the target architecture, entry point and signature, dependency warnings, and the first Java calls or failure reached by both inspections.

## 2. Drive one call and answer Java

Start a `serve` child process with the required rootfs and environment. Build the request loop from the README's serve example. In a checkout, [examples/python/signutil.py](../../examples/python/signutil.py) is a complete driver using only the standard library:

```bash
python3 examples/python/signutil.py
```

It prints:

```text
sign: 1000ed2f7e5ecd00bb4a6454e1d487b7dc7500040002000acc
```

Load the library first. Answer its initialization calls, then invoke the target JNI method or resolve and call its C function. For a C function, check the argument layout in README, especially 64-bit arguments on ARM32.

Flush each input line. Read stdout until the running command ends with `ok` or `error`. Keep stderr separate and drain it as the child runs. A `jni` question suspends the call: match its operation, class, method and signature, read any object arguments, create the needed result, then send `answer`. A nested request can raise another question; finish the innermost one first. A `break` question instead needs a debugger resume command.

For an unknown Java call, send `unhandled` and keep the resulting crash. Add the specific answer from the app's behavior, then rerun from a fresh instance with the same input. Use the answer type from README; distinguish an intentional null from a call you have not implemented. Repeat until the target call completes. Release created handles and free guest allocations after their last use in a service.

**Done when:** one fixed input reaches a decoded result with every Java question answered, or a saved failure identifies the next missing answer or guest fault. Take failures to step 3; take a wrong result to steps 4–6.

## 3. Read the crash before changing the setup

Save the whole error response. For an error without `crash`, check the command and setup against README. For a guest failure, read `crash.message`, the thread, backtrace and registers together. Start with the first guest frame and its symbol. At a host or JNI stub, that frame can identify the caller better than the saved PC. Use `symbolize`d locations in diagnostics to choose the routine to inspect.

Follow a short backtrace with a breakpoint or a narrow trace. Backtraces follow frame pointers; code that omits them can leave only one frame. Read `crash.stack` for app code addresses that may be return addresses; it keeps the words from the moment of the crash. Treat a short backtrace as a limit on the evidence.

Keep protocol addresses as decimal strings. Location text uses hex offsets, such as `libfoo.so+0x1234`; pass that text to `resolve` or a breakpoint's `at`. Use the bare location, without a parenthesized symbol suffix. Read instruction-event addresses as hex strings, as specified in README.

For a concrete failure, breakpoint and replay exchange, read [serve-debugging.md](serve-debugging.md). Its commands run against the committed fixture and show how to interpret the replies.

**Done when:** you have the original failure, the guest caller or fault location, and one question about the input, environment or computation to test next.

## 4. Narrow the cause with one tool

Pick the row that answers the current question:

| Symptom or question | Tool | What to read |
|---|---|---|
| Which file does it try to open? | `--trace-syscalls` | Path, result, errno and caller. Look for `ENOENT`; ordinary paths outside rootfs are reported as not found. Check the requested path against rootfs. |
| What does it ask libc or another dependency? | `--trace-imports` | Imported function, argument values and app caller. Follow the call that first receives an unexpected value. |
| Which Java behavior is missing or wrong? | JNI trace: `--verbose` or serve `trace` with `jni` enabled | Method, signature, arguments and caller. Match these against the driver's answer. |
| What does a routine compute? | `--trace-code` over that routine's range | Instruction bytes, disassembly, registers read before and written after. Find the first value that diverges. |
| What is a value at this point? | Breakpoint, then `registers` and `read` | Argument registers and the bytes at their pointers. Decode `read`'s base64 before interpreting memory. |
| Which memory does a call write? | Serve `snapshot` before and after the call, then `unidroid diff before.snap after.snap` | Changed bytes with locations, newly mapped ranges, and changed registers. |
| Which range is this address in? | Serve `maps` | Start and exclusive end, permissions, and the library or memory area's name. |
| What changes if this function returns a known value? | Breakpoint, then `return` | Whether replacing the result changes the failing path. Use it as an experiment to test the cause. |

Turn traces on with the serve `trace` command just before the call under study, then turn them off after it ends. Each change replaces the whole trace selection; include every trace you want to keep. Use `--log-format json` when a program reads stderr. Start with the caller's module and narrow instruction traces to a range once the routine is known.

At a stop, use `step` to follow an instruction or `continue` to resume. Use `finish` at a function's first instruction, before it changes lr, to inspect its return value. Place a `return` hook at the first instruction too, before the function changes its stack. A Thumb IT block runs as one step; read the reported stop location when a breakpoint lands after that block.

**Done when:** a trace or stopped state shows the first unexpected input, answer or computed value, and rerunning one targeted change confirms or rejects the proposed cause.

## 5. Record where it fails, replay where you debug

Start the service with `serve --record` where the failure happens and drive the same input. Store recordings outside git: they contain the inputs, including keys. Keep snapshots out of git too: they hold all guest memory, including keys and inputs. Keep the libraries and rootfs used by that run.

Run `unidroid replay` from the directory the service ran in so its relative paths still resolve. Add the traces chosen in step 4. The worked session in [serve-debugging.md](serve-debugging.md) gives a complete capture and replay. A successful replay means the recorded responses match, including a recorded error; it does not mean the library call succeeded.

**Done when:** replay reports that all responses match and reproduces the failure with the diagnostic evidence you need. If it reports a mismatch, use the named recording line to check the setup and expected versus actual response first.

## 6. Compare a wrong result with unidbg

When the call completes but returns the wrong bytes, read [bench/compare.sh](../../bench/compare.sh) and the matching sample under [tests/](../../tests/). The repo's samples are checked byte for byte against unidbg 0.9.9. Use Java 21 for that baseline; newer JDKs crash its Unicorn binding.

Match the library, input bytes, Java answers, properties and initialization order in both drivers. Compare decoded output bytes first, then use step 4 to find the earliest differing argument or return value. Keep one input fixed while testing each change.

**Done when:** the result matches byte for byte, or you can show the smallest call whose inputs match and whose outputs differ, with both setups and the first differing byte.
