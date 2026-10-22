# Worked serve debugging session

Use this when you need a concrete crash, trace, breakpoint or replay exchange. Run the shell blocks in order from a built checkout's root, in one shell. They use the committed 32-bit SignUtil fixture and rootfs. A release contains this guide; take the fixtures and examples from a checkout to run it.

## 1. Capture an unhandled Java call

This session deliberately leaves `HashMap.size` unanswered. The temporary directory keeps the recording outside the repo.

```bash
session_dir=$(mktemp -d)
target/release/unidroid serve --rootfs rootfs --log-format json --record "$session_dir/session.jsonl" <<'JSON'
{"cmd":"load","path":"tests/fixtures/armeabi-v7a/libsignutil.so"}
{"cmd":"onload","module":0}
{"cmd":"new_string","value":"aa"}
{"cmd":"new_string","value":"bb"}
{"cmd":"new_object","class":"java/util/HashMap"}
{"cmd":"new_string","value":"cc"}
{"cmd":"call_static","class":"com/anjuke/mobile/sign/SignUtil","method":"getSign0(Ljava/lang/String;Ljava/lang/String;Ljava/util/Map;Ljava/lang/String;I)Ljava/lang/String;","args":[1,2,3,4,10]}
{"cmd":"unhandled"}
JSON
```

Stdout from this run:

```json
{"ok":0}
{"ok":65540}
{"ok":1}
{"ok":2}
{"ok":3}
{"ok":4}
{"jni":{"args":[],"class":"java/util/HashMap","name":"size","op":"call","receiver":3,"signature":"()I","static":false,"type":"I"}}
{"error":"unhandled JNI call: CallIntMethod java/util/HashMap->size()I from libsignutil.so+0x1c05","crash":{"message":"unhandled JNI call: CallIntMethod java/util/HashMap->size()I from libsignutil.so+0x1c05","thread":1234,"registers":{"r0":"1073742768","r1":"3","r2":"2147483648","r3":"65984","r4":"1073742768","r5":"268449305","r6":"65856","r7":"1881079768","r8":"0","r9":"0","r10":"3","r11":"0","r12":"2147483648","sp":"1881079656","lr":"268442629","pc":"65988","cpsr":"2147484115"},"backtrace":[{"address":"268442628","location":"libsignutil.so+0x1c04","symbol":"Java_com_anjuke_mobile_sign_SignUtil_getSign0+0x58"}]}}
```

The next answer to implement is the map's size, an integer. The crash's first frame points into `getSign0`; PC is in the JNI stub. The single frame is enough to find this caller. To complete the signing call, use the Python driver linked in step 2 of [SKILL.md](SKILL.md): it answers size, keySet, toArray and get from the supplied map.

**Done when:** the last reply is the shown unhandled call and its crash names `getSign0+0x58`.

## 2. Replay with evidence

```bash
target/release/unidroid replay "$session_dir/session.jsonl" --verbose --trace-syscalls --trace-imports --trace-code libsignutil.so+0x1bac-0x1bdc --log-format json 2>"$session_dir/replay.jsonl"
```

Stdout:

```text
replayed 8 commands, all responses match
```

Selected stderr events, in their original order, from `replay.jsonl`:

```json
{"event":"syscall","name":"openat","number":322,"args":["0xffffff9c","\"/dev/__properties__\"","0xa8000","0x0"],"result":-2,"errno":"ENOENT","at":"libc.so+0x41484 (__openat+0x8)","from":"libc.so+0x1be74 (open64+0x22)"}
{"event":"instruction","at":"libsignutil.so+0x1bb4 (Java_com_anjuke_mobile_sign_SignUtil_getSign0+0x8)","address":"0x10001bb4","bytes":"0446","text":"mov r4, r0","read":[["r0","0x400003b0"]],"written":[["r4","0x400003b0"]]}
{"event":"call","function":"__aeabi_memclr8","module":"libc.so","args":["0x701eff88","0x39","0x1","0x2"],"from":"libsignutil.so+0x1bd0 (Java_com_anjuke_mobile_sign_SignUtil_getSign0+0x24)"}
{"event":"jni_call","function":"CallIntMethod","class":"java/util/HashMap","name":"size","signature":"()I","args":[],"from":"libsignutil.so+0x1c05"}
```

Here, the early file lookup fails but execution continues. The last event names the Java call that stops this run. The instruction event shows r0 copied into r4; the import event shows the app calling libc. Follow the evidence tied to the failure before changing the environment.

**Done when:** all eight inputs replay, and the log reaches the same `HashMap.size` call with the caller shown above.

## 3. Inspect and replace a C function's result

This fresh service loads the same fixture, then calls `strlen` in its libc dependency. The addresses below are from this exact setup. In your driver, use each reply's address for the next request. In particular, keep the Thumb bit in the resolved call address; the breakpoint address has it cleared.

The first call traces a short range, stops, reads its argument and steps. The second call uses `finish` at entry. The third replaces the result with `return` at entry.

```bash
target/release/unidroid serve --rootfs rootfs --log-format json 2>"$session_dir/strlen.jsonl" <<'JSON'
{"cmd":"load","path":"tests/fixtures/armeabi-v7a/libsignutil.so"}
{"cmd":"resolve","at":"strlen"}
{"cmd":"new_cstring","value":"hello"}
{"cmd":"break","at":"libc.so+0x18c90"}
{"cmd":"trace","code":["libc.so+0x18c90-0x18cb0"]}
{"cmd":"call_function","address":"269388945","args":["1073743392"]}
{"cmd":"registers"}
{"cmd":"read","address":"1073743392","size":6}
{"cmd":"backtrace"}
{"cmd":"step"}
{"cmd":"continue"}
{"cmd":"trace"}
{"cmd":"call_function","address":"269388945","args":["1073743392"]}
{"cmd":"finish"}
{"cmd":"continue"}
{"cmd":"call_function","address":"269388945","args":["1073743392"]}
{"cmd":"return","value":"42"}
{"cmd":"unbreak","at":"libc.so+0x18c90"}
{"cmd":"free","address":"1073743392"}
JSON
```

Selected stdout lines, in order (the full stop and register dumps are left out):

```json
{"ok":0}
{"ok":"269388945"}
{"ok":"1073743392"}
{"ok":"269388944"}
{"ok":null}
{"ok":"aGVsbG8A"}
{"ok":[{"address":"269388944","location":"libc.so+0x18c90","symbol":"strlen"}]}
{"ok":"5"}
{"ok":null}
{"ok":"5"}
{"ok":"42"}
{"ok":null}
{"ok":null}
```

At the first stop, r0 holds `1073743392`. Reading six bytes there returns `aGVsbG8A`, which decodes to `hello` and a null byte. The step stop is at `libc.so+0x18c94 (strlen+0x4)`. On the second call, the finish stop has r0 equal to `5`; continuing completes the pending call. On the third call, `return` makes that call complete with `42`.

The trace log covers only the first call. Resume commands have no separate acknowledgement: after one, keep reading for a new question or the pending call's final response.

**Done when:** the first two calls return `5`, the hooked call returns `42`, and the argument bytes explain the real length.
