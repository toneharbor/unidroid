# The Android environment

unidroid emulates Android for native libraries; you can replace imported C calls with host logic.

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
