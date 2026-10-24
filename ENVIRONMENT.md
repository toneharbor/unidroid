# The Android environment

unidroid emulates Android for native libraries; you can replace imported C calls with host logic.

## APK

Open one APK when building the emulator. The builder parses it before any CPU is created; errors name the APK path and the problem.

```rust
let mut emu = Emulator::builder().apk("app.apk").build()?;
let module = emu.load_apk_library("libfoo.so")?;
let apk = emu.apk().unwrap();
println!("{} {:?} {}", apk.package, apk.version_name, apk.version_code);
```

`EmulatorBuilder::apk(self, path: impl Into<PathBuf>) -> Self` selects the APK. `Emulator::apk(&self) -> Option<&Apk>` and `apk_mut(&mut self) -> Option<&mut Apk>` expose its data. Without an APK they return `None`. `Apk` is exported from the crate root:

```rust
pub struct Apk {
    pub path: PathBuf,             // Canonical host path
    pub package: String,
    pub version_name: Option<String>,
    pub version_code: i64,
    pub signatures: Vec<Vec<u8>>,      // PackageInfo.signatures
    pub signers: Vec<Vec<u8>>,         // SigningInfo.getApkContentsSigners()
    pub signing_history: Vec<Vec<u8>>, // Rotation lineage, oldest first
    // Private shared archive indexes and readers
}
```

Only the root `<manifest>` attributes of binary `AndroidManifest.xml` are parsed. `package` without a namespace is required. Android `versionCode`, `versionCodeMajor` and `versionName` use their resource-map IDs when present, otherwise their names in the Android namespace. UTF-8 and UTF-16 string pools, string values and decimal/hex integer values work. Integer `versionName` follows Android `TypedValue.coerceToString`: `TYPE_INT_DEC` uses signed decimal (for example `-1`), and `TYPE_INT_HEX` uses `0x` and lowercase hex (for example `0x2a`). `version_code` is `(versionCodeMajor << 32) | versionCode`, with missing components zero, like `PackageInfo.getLongVersionCode`. A resource-reference `versionName` is left unresolved as `None`; an absent name is also `None`. Parsing follows Android's lenient approach: it ignores unused strings and trailing manifest/signature data, decodes needed strings lossily, and accepts BER indefinite-length v1 containers while still rejecting truncated fields it needs.

The certificate lists have distinct rules:

1. `signers` contains the v3 signer's first certificate (`0xf05368c0`), otherwise each v2 signer's first certificate (`0x7109871a`) in order, otherwise the v1 certificates, otherwise an empty list.
2. `signing_history` contains the v3 proof-of-rotation lineage (`0x3ba06f8c`), oldest first and current last. It is empty when the attribute is absent. The parser reads its version and each node's signed-data certificate; it does not verify the rotation signatures.
3. `signatures` contains the v1 certificates when v1 block files exist, matching unidbg 0.9.9's apk-parser. Without v1 it contains only the oldest lineage certificate when there is a lineage, as Android's `GET_SIGNATURES` does; otherwise it equals `signers`. Unidbg returns no certificates for v2/v3-only APKs.

V1 block files are directly in `META-INF/` (`.RSA`, `.DSA` or `.EC`, case-insensitive). Every certificate from each PKCS#7 SignedData certificates field is kept in central-directory file order and certificate order; nested files are ignored.

Certificate bytes are copied exactly, without decoding and re-encoding. These are extracted certificates, not verification of the APK's cryptographic signatures. Through `apk_mut`, a host can replace metadata or certificates, such as the original certificate of a re-signed APK. Changing `path` or `package` does not move the guest files and directories already installed, change the process name, or rename already loaded modules. Checkpoints share unchanged `Apk` data and restore it, including into a fresh emulator; a later mutation makes a private copy.

`load_apk_library(name: &str) -> Result<Module>` accepts a library file name such as `libfoo.so`, maps and relocates it and its dependencies, and runs constructors. `load_apk_library_without_constructors(name: &str) -> Result<Module>` defers those constructors until `run_constructors`; both error without an APK. APK libraries count as app libraries for traces.

Every named load, dependency and guest `dlopen` searches the APK first, then the directories of loaded host libraries, then rootfs. Modules are identified by file name: if `libfoo.so` is already loaded, `dlopen` returns that module even for an explicit path to another `libfoo.so`. Otherwise, before the name search, `dlopen` honors an argument naming an existing file under rootfs when the argument contains no `..`; for example `/vendor/lib64/libfoo.so` loads that exact file even if the APK contains another `libfoo.so`. Arm64 searches `lib/arm64-v8a/`; arm32 searches `lib/armeabi-v7a/` then `lib/armeabi/`. Within an architecture, the base APK is searched before the neighboring `config.arm64_v8a.apk` or `config.armeabi_v7a.apk`, if present. Before a CPU exists, lookup tries arm64 first, then arm32; the selected ELF sets the architecture. Explicit `load_library(host_path)` continues to load that host file. Unusable ELF images, including missing dynamic tables, invalid packed-relocation ranges or invalid `PT_LOAD` segments, fail with the host path or APK path and entry name; failed images are not cached. Before caching or loading, every `PT_LOAD` must have a file range within the ELF, `p_filesz <= p_memsz`, and virtual and page-aligned virtual ranges that do not overflow. A relocation outside every load segment fails with the library name and relocation offset; guest `dlopen` load errors also name the caller.

An APK library's guest path, used by `dl_iterate_phdr`, dynamic-loader names and `/proc/self/maps`, is `/data/app/<package>-1/lib/arm64/<name>` on arm64 or `/data/app/<package>-1/lib/arm/<name>` on arm32. `emu.modules()` and serve `modules` report its host source as `<canonical APK path>!/lib/<abi>/<name>`. Host-loaded libraries keep their existing names and maps labels.

The builder exposes the host APK at `/data/app/<package>-1/base.apk` through `add_host_file`, with no copy. It creates overlay directories `/data/data/<package>`, `/data/data/<package>/files` and `/data/data/<package>/cache`. The process name defaults to the package name; explicit `.process_name(...)` or `--process-name` wins. Without an APK it defaults to `unidroid`.

The base APK and both neighboring architecture splits that exist are opened and their central directories read once at build. An invalid split fails `build()` with its path in the error. Splits that appear after build are not used. Indexes and retained file readers are shared across checkpoints and metadata copies, including `apk_mut`; subsequent lookups use the indexes without opening the APK, and an image-cache miss seeks to and reads only the requested entry. `Emulator::module_bytes(&self, module: Module) -> Result<&[u8]>` borrows the original ELF bytes from the image cache before relocation or guest writes; `inspect --apk` uses this accessor and does not open or inflate the entry again.

A stored entry whose data begins at a 4096-aligned APK offset maps directly from the APK only when its compressed and uncompressed sizes agree and its full data range fits in the file; otherwise loading fails with the APK path and entry name. Other stored entries and deflated entries are read into memory. All APK library images use the process-wide image cache keyed by canonical APK path and entry name, so batch workers share the read-only code. Keep the APK and any split APKs available and unchanged while images are cached or checkpoints are in use.

`--apk FILE` is shared by `inspect`, `serve`, `batch` and `fields`. With it, `inspect` takes a library name instead of a host path. Serve adds `load_apk_library` (`name`, optional `constructors`, default true) returning the module index; `apk` returning metadata with base64 certificates or `null`; and `set_apk` with any of `package`, `version_name`, `version_code`, `signatures`, `signers` and `signing_history` (base64 lists), returning `null`. Omitted fields stay unchanged; `version_name: null` clears the name. Setting metadata without an APK errors. Recording keeps the APK argument and effective process name for replay. Package JNI answers and `AAsset` functions are covered in “APK Java answers and assets” below.

## APK Java answers and assets

With an APK, these Android answers run before `JniHandler`, policy rules, `Probe` and the serve driver. They cover the entry points below and objects created by these built-ins. Other methods and fields still reach your handler. Without an APK, these answers, the Android hierarchy and the asset function bindings are disabled.

Change answers through `emu.apk_mut()` or serve `set_apk`, for example to supply the original certificate of a re-signed APK. PackageInfo, ApplicationInfo and SigningInfo read current metadata when called; a Signature holds a copy of its certificate bytes. Changing `package` changes subsequent Java package names and computed paths, but does not move files already installed or change the process name. Returned objects, arrays and strings have fresh local references. Null package names, null Signature constructor arrays and null asset-list paths raise pending `java/lang/NullPointerException` and return null.

A receiver is a Context when either its object's class or the call's declared class is assignable to `android/content/Context`. This uses the hierarchy below together with explicit `superclass` declarations. It also applies to host `new_object` receivers, preserving their Rust payload. For example:

```rust
emu.superclass("com/example/MyContext", "android/app/Application");
let context = emu.new_object("com/example/MyContext", ());
```

Context entry points:

| method | answer |
|---|---|
| `getPackageName()Ljava/lang/String;` | APK `package` |
| `getPackageManager()Landroid/content/pm/PackageManager;` | a built-in PackageManager |
| `getApplicationInfo()Landroid/content/pm/ApplicationInfo;` | a built-in ApplicationInfo |
| `getPackageCodePath()Ljava/lang/String;` | `/data/app/<package>-1/base.apk` |
| `getPackageResourcePath()Ljava/lang/String;` | `/data/app/<package>-1/base.apk` |
| `getAssets()Landroid/content/res/AssetManager;` | a built-in AssetManager |
| `getApplicationContext()Landroid/content/Context;` | the emulator's Application |

Static entry points:

| class | member | answer |
|---|---|---|
| `android/app/ActivityThread` | `currentPackageName()Ljava/lang/String;` | APK `package` |
| `android/app/ActivityThread` | `currentProcessName()Ljava/lang/String;` | the effective process name |
| `android/app/ActivityThread` | `currentApplication()Landroid/app/Application;` | the emulator's Application |
| `android/content/pm/PackageManager` | `GET_SIGNATURES:I` | `0x40` |
| `android/content/pm/PackageManager` | `GET_SIGNING_CERTIFICATES:I` | `0x08000000` |

There is one `android/app/Application` object per emulator. `currentApplication` and `getApplicationContext` return fresh references to that same object, even after earlier references are released. Checkpoints preserve its identity.

The PackageManager object's class is `android/app/ApplicationPackageManager`. It answers `getPackageInfo(Ljava/lang/String;I)Landroid/content/pm/PackageInfo;` and `getApplicationInfo(Ljava/lang/String;I)Landroid/content/pm/ApplicationInfo;` for the APK's own package. Other package names reach the handler, which can model other installed apps. PackageInfo remembers the requested flags.

`android/content/pm/PackageInfo` members:

| member | answer |
|---|---|
| `packageName:Ljava/lang/String;` | APK `package` |
| `versionName:Ljava/lang/String;` | `version_name`, or null when absent |
| `versionCode:I` | the low 32 bits of `version_code` |
| `getLongVersionCode()J` | the full `version_code` |
| `signatures:[Landroid/content/pm/Signature;` | a Signature array from `signatures` when flags include `0x40`, otherwise null |
| `signingInfo:Landroid/content/pm/SigningInfo;` | a SigningInfo when flags include `0x08000000`, otherwise null |
| `applicationInfo:Landroid/content/pm/ApplicationInfo;` | a built-in ApplicationInfo |

`android/content/pm/Signature` answers `<init>([B)V`, copying the supplied bytes; `toByteArray()[B`, returning a new copy; `toCharsString()Ljava/lang/String;`, returning lowercase hex; `hashCode()I`, using `java.util.Arrays.hashCode(byte[])` with signed bytes; and `equals(Ljava/lang/Object;)Z`, comparing certificate bytes with another built-in Signature. Signature keys in built-in collections use this hash and equality too.

`android/content/pm/SigningInfo` members:

| method | answer |
|---|---|
| `getApkContentsSigners()[Landroid/content/pm/Signature;` | APK `signers` |
| `hasMultipleSigners()Z` | more than one signer |
| `hasPastSigningCertificates()Z` | `signing_history` is nonempty |
| `getSigningCertificateHistory()[Landroid/content/pm/Signature;` | null for multiple signers; otherwise nonempty `signing_history`, or `signers` when there is no history |

`android/content/pm/ApplicationInfo` fields all have signature `Ljava/lang/String;`: `packageName` is APK `package`; `sourceDir` and `publicSourceDir` are `/data/app/<package>-1/base.apk`; `nativeLibraryDir` is `/data/app/<package>-1/lib/arm64` on arm64 or `/data/app/<package>-1/lib/arm` on arm32; `dataDir` is `/data/data/<package>`; `processName` is the effective process name. These paths use the same helpers as APK file installation and library loading. Other fields reach the handler.

`android/content/res/AssetManager` answers `list(Ljava/lang/String;)[Ljava/lang/String;`. It returns sorted names of files and folders directly under `assets/<path>` in the base APK. `""` names the assets root; missing or empty directories return an empty array. It can be passed to `AAssetManager_fromJava`.

All these objects answer `getClass()Ljava/lang/Class;`, `toString()Ljava/lang/String;`, `hashCode()I` and `equals(Ljava/lang/Object;)Z`. Hash and equality use identity except for Signature. The default text is the dotted class name, `@` and the hash in lowercase hex.

`IsInstanceOf`, `IsAssignableFrom` and the Context test share these built-in parent chains:

- `android/app/Application` → `android/content/ContextWrapper` → `android/content/Context` → `java/lang/Object`.
- `android/app/Activity` → `android/view/ContextThemeWrapper` → `android/content/ContextWrapper`.
- `android/app/Service` → `android/content/ContextWrapper`.
- `android/app/ApplicationPackageManager` → `android/content/pm/PackageManager` → `java/lang/Object`.
- PackageInfo, Signature, SigningInfo, ApplicationInfo and AssetManager → `java/lang/Object`.

Declared app superclasses extend these chains. Unrelated Android and Java targets return false for known built-in sources; app-specific targets still reach `is_assignable`. `GetSuperclass` retains its explicit-declaration rule.

With an APK, these C imports bind to host functions even though rootfs has no `libandroid.so`. `find_symbol` and `dlsym` resolve them, `hook_import` wraps them with the built-in as `original`, and import traces identify the built-ins as `libandroid.so`.

| function | behavior |
|---|---|
| `AAssetManager_fromJava(JNIEnv*, jobject)` | returns the same non-null guest manager address on every call |
| `AAssetManager_open(mgr, name, mode)` | opens `assets/<name>` in any mode; missing files return NULL; stored or deflated contents are copied into a guest allocation |
| `AAsset_read(asset, buf, count)` | copies up to `count` bytes, advances the position and returns the byte count; returns 0 at EOF |
| `AAsset_seek(asset, offset, whence)`, `AAsset_seek64(asset, offset, whence)` | SEEK_SET/CUR/END accept a new position in `0..=length`, including exactly EOF; an out-of-range result, signed overflow or bad whence returns -1 without moving |
| `AAsset_getLength(asset)`, `AAsset_getLength64(asset)` | total byte length |
| `AAsset_getRemainingLength(asset)`, `AAsset_getRemainingLength64(asset)` | length minus position, never negative |
| `AAsset_getBuffer(asset)` | guest pointer to the whole contents, valid until close |
| `AAsset_close(asset)` | frees the contents and forgets the asset |
| `AAssetManager_openDir(mgr, dir)` | opens a sorted snapshot of direct files, excluding subfolders; non-null even when empty |
| `AAssetDir_getNextFileName(dir)` | next file's name as a guest C string, valid until the next call or close; NULL at the end |
| `AAssetDir_rewind(dir)` | restarts directory iteration |
| `AAssetDir_close(dir)` | frees the directory and its name buffer |

On arm32, `off_t` is 32-bit and `off64_t` is 64-bit. `AAsset_seek64` takes the asset in r0, skips r1, takes the offset in r2:r3 and reads `whence` at `[sp]`; 64-bit results use r0:r1. On arm64 the seek arguments use x0–x2 and results use x0. Rust `call_function` still returns only r0 on arm32, as described in [README.md](README.md).

Assets use the APK's retained Archive index and reader. A compressed asset is inflated once per open; reads, seeks and buffer access use guest memory. Checkpoints restore the manager address, open assets and their positions, contents, directory snapshots, current name buffers and directory positions, including into a fresh emulator; see [DEBUGGING.md](DEBUGGING.md).

Three deliberate departures from unidbg 0.9.9: package flags are honored (unidbg ignores `GET_SIGNATURES` and `GET_SIGNING_CERTIFICATES`); the returned PackageManager class is `android/app/ApplicationPackageManager` (unidbg uses `android/content/pm/PackageManager`); and requests for other packages reach the handler (unidbg answers any name).

## Built-in Java classes

Common Java calls are answered before `JniHandler`, policy rules, `Probe` and the serve driver. Instance calls use the receiver's built-in kind plus the method name and JNI signature, so looking up `get` on `java/util/Map` works on a built-in HashMap. Constructors, static methods and static fields use the class name. Uncovered methods, charsets and digest algorithms still reach your handler. Objects created with `new_object(class, value)` remain opaque, even when their class is `java/util/HashMap`; with an APK, the Context entry points above are the exception. Other calls still reach your handler.

| class | built-in calls |
|---|---|
| `java/lang/Object` | `getClass`, `hashCode`, `equals` on built-in objects and strings; `toString` on strings, boxes, StringBuilder, maps and lists when their elements have built-in text |
| `java/lang/Class` | `getName` on class objects, using dots, such as `java.lang.String` |
| `java/lang/reflect/Method`, `Constructor`, `Field` | `getName`, `getDeclaringClass`, `getClass`, `equals`, `hashCode`; constructor names use the declaring class name with dots |
| `java/lang/String` | empty, byte-array (with optional range and charset name), string-copy and char-array constructors; `getBytes`, `length`, `isEmpty`, `charAt`, `toCharArray`, `equals`, `equalsIgnoreCase`, `hashCode`, `compareTo`, `toString`, `startsWith`, `endsWith`, `contains`, both `indexOf` forms, both `substring` forms, `toLowerCase`, `toUpperCase`, `trim`, `concat`, `replace`; static `valueOf` for int, long, boolean, char and Object |
| `java/lang/CharSequence` | `length` and `toString` on strings and StringBuilders |
| `java/nio/charset/Charset`, `StandardCharsets` | `Charset.forName`, `name`, `equals`, `hashCode`; static fields `UTF_8`, `US_ASCII`, `ISO_8859_1`, `UTF_16LE`, `UTF_16BE` |
| `java/lang/StringBuilder` | empty, string and capacity constructors; `append` for String, Object, int, long, char and boolean; `length`, `toString` |
| `java/lang/Boolean`, `Byte`, `Character`, `Short`, `Integer`, `Long`, `Float`, `Double` | primitive constructors and static `valueOf`; matching primitive accessor and `toString`; `intValue` and `longValue` on Byte, Short, Integer and Long; static `Integer.parseInt` (optional radix), `Long.parseLong`, `Integer.toString`, `Long.toString`, `Integer.toHexString` |
| `java/util/HashMap`, `LinkedHashMap`, `TreeMap` | empty constructors; capacity constructors on HashMap and LinkedHashMap; `put`, `get`, `getOrDefault`, `containsKey`, `remove(Object)`, `size`, `isEmpty`, `clear`, `keySet`, `values`, `entrySet`; entries answer `getKey`, `getValue` |
| `java/util/HashSet` and map views | empty HashSet constructor and `add`; `contains`, `size`, `isEmpty`, `iterator`, `toArray()` on sets and views |
| `java/util/ArrayList`, `LinkedList` | empty constructors; ArrayList capacity constructor; both `add` forms, `get`, `set`, `remove(int)`, `size`, `isEmpty`, `contains`, `indexOf`, `clear`, `iterator`, `toArray()` |
| `java/util/Iterator` | `hasNext`, `next` |
| `java/security/MessageDigest` | static `getInstance`; `update(byte[])`, `update(byte[], offset, length)`, `update(byte)`; `digest()`, `digest(byte[])`, `reset`, `getAlgorithm` |
| `android/util/Base64` | static `encodeToString(byte[], flags)`, `encode(byte[], flags)`, `decode(String, flags)`, `decode(byte[], flags)` |

JNI `ToReflectedMethod` and `ToReflectedField` create built-in reflected objects with fresh local references; `FromReflectedMethod` and `FromReflectedField` return their original member IDs without a handler. A method named `<init>` becomes a Constructor. IDs are shared with `GetMethodID`, `GetStaticMethodID`, `GetFieldID` and `GetStaticFieldID`, keyed by declaring class, name, signature, static flag and method/field kind. `ToReflected*` uses the ID's declaring class and stores the supplied `isStatic` flag. Method and Constructor extend `java/lang/reflect/Executable`, which extends `java/lang/reflect/AccessibleObject`; Field extends AccessibleObject, which extends Object. Checkpoints preserve reflected members and object identity.

A wrong-kind or unknown member ID stops the guest with an error naming the JNI function and caller location; an invalid `cls` stays a reference error. `FromReflected*` rejects null, invalid references, wrong reflected kinds and other objects, including host `new_object("java/lang/reflect/Method", ...)`. Its errors name the function, the object's Java class (or `null` / `invalid reference` when no object exists) and caller location. The built-in Java answers in the table run before the handler. Other methods reach the handler with `JniCall.class` equal to the class the method ID was looked up on, as for other built-in objects; for example, `java/lang/reflect/AccessibleObject` for a `setAccessible` lookup on that class. A handler can inspect the reflected member with `emu.reflected_member(call.receiver)`.

Reflected members compare equal when both are built-in reflected objects with the same kind, declaring class, name and signature; the static flag does not affect equality. Separate objects can be equal while `IsSameObject` remains false. Method and Field hashes are the Java `String.hashCode` of the dotted declaring class name XOR the member name's hash. Constructor hashes use only the dotted declaring class name's hash. Built-in collections use these same equality and hash rules.

Rust can create and inspect these built-ins:

```rust
pub fn new_reflected_method(&mut self, class: &str, name: &str, signature: &str, is_static: bool) -> Handle;
pub fn new_reflected_field(&mut self, class: &str, name: &str, signature: &str, is_static: bool) -> Handle;
pub fn reflected_member(&self, handle: Handle) -> Result<ReflectedMember>;
```

The constructors create or reuse the same member IDs as the corresponding `Get*ID` calls, in either order. Their handles are API-owned like `new_string`; release them when done. `ReflectedMember` and `ReflectedMemberKind` are exported from the crate root. The member has public `class`, `name` and `signature` strings, `is_static: bool` and `kind: ReflectedMemberKind` (`Method`, `Constructor`, `Field`). Its `name` retains `<init>` for constructors. `reflected_member` rejects any other object, following `java_map`, `java_list` and `unboxed` errors. There is no reflection serve command or policy value.

For example, a handler can model `Class.getDeclaredMethod` for a known no-argument app method and return an object accepted by `FromReflectedMethod`:

```rust
use unidroid::{Emulator, Handle, JniCall, JniHandler};

struct Reflection;
impl JniHandler for Reflection {
    fn call_object_method(&mut self, emu: &mut Emulator, call: &JniCall<'_>) -> Option<Handle> {
        if call.class == "java/lang/Class"
            && call.name == "getDeclaredMethod"
            && call.signature == "(Ljava/lang/String;[Ljava/lang/Class;)Ljava/lang/reflect/Method;"
            && emu.string(call.args[0].object()).ok()? == "run"
        {
            let parameters = emu.object_array(call.args[1].object()).ok()?;
            let no_parameters = parameters.is_empty();
            for parameter in parameters {
                emu.release(parameter);
            }
            if no_parameters {
                return Some(emu.new_reflected_method("com/example/Signer", "run", "()V", false));
            }
        }
        None
    }
}
```

String lengths and indexes count UTF-16 units, including each half of a surrogate pair. Supported charsets are UTF-8 (the default), US-ASCII, ISO-8859-1, UTF-16LE and UTF-16BE; names ignore case, hyphens and underscores. `String.valueOf(Object)` returns the object's `toString()` result, including the same object for a String; uncovered `toString` calls reach your handler. StringBuilder `append(Object)` uses built-in text. Both use `"null"` for null. Unknown element text falls through to your handler. Map and list text uses Java's `{key=value, ...}` and `[item, ...]` forms.

`Integer`, `Short`, `Byte` and `Long` cache `valueOf` results for `-128..=127`; `Character` caches `0..=127`, and `Boolean` caches both values. Constructors and host `new_boxed` still create new objects. Each `StandardCharsets` field also returns the same object on every read. These caches belong to JNI state, survive released handles and preserve identity across checkpoints. Returned references are fresh local references. Charset equality compares canonical names, and its hash is the canonical name's Java `String.hashCode`; for example `Charset.forName("utf-8")` equals `StandardCharsets.UTF_8`.

String keys compare by UTF-16 content; boxed keys compare by primitive type and Java value, including Java's NaN and signed-zero rules. Signature keys compare certificate bytes; Charset keys compare canonical names; reflected keys use the member equality and hash rules above; other keys compare by object identity, including across different handles to the same object. HashMap and HashSet use Java string and box hashes, a power-of-two table starting at 16 by default, a 0.75 load factor and bucket order. Entries within each bucket keep insertion order, including through resizing; removal and reinsertion moves a key to the end of its bucket. Long buckets are not converted to trees. LinkedHashMap keeps insertion order. TreeMap sorts strings by UTF-16 and comparable boxed keys by value; unsupported comparisons fall through to the handler, or return an error from `new_java_map`.

Map views (`keySet`, `values`, `entrySet`) and iterators are **snapshots taken when created**. This differs from Java's live views. They keep references to their elements. Releasing an element's host handle does not remove it from a collection. All built-in object state is copied by checkpoints, preserving aliases and cycles; rewind restores collection contents, iterator positions, StringBuilder contents and digest state. Dropping or rewinding JNI state frees container cycles even after their last handles were released.

`IsInstanceOf` tests the object's runtime class: a class object such as `String.class` is a `java/lang/Class`. A null object returns true for any valid class; invalid class handles remain reference errors. `IsAssignableFrom` compares the represented classes. Both recognize built-in class hierarchies after explicit `superclass` declarations. A declared app subclass of HashMap is also a Map. Built-in sources answer false for unrelated `java/` or `javax/` targets; app-specific targets still reach `is_assignable`. `GetSuperclass` continues to use explicit declarations.

Digests support MD5, SHA-1 (also SHA1 and SHA), SHA-224, SHA-256, SHA-384 and SHA-512, ignoring case. `digest` returns bytes and resets the digest. `getAlgorithm` preserves the requested name. Base64 flags are Android's `DEFAULT` 0, `NO_PADDING` 1, `NO_WRAP` 2, `CRLF` 4 and `URL_SAFE` 8. Wrapped encoding breaks after 76 characters; nonempty output ends in exactly one line break, and empty output has none. Decoding accepts padded or unpadded input and skips non-alphabet bytes as Android does. Invalid padding throws `IllegalArgumentException` with `bad base-64`.

Data errors set pending JNI exceptions and return zero or null. Null charset names in String constructors and `getBytes(String)` raise `NullPointerException`. Null arrays also raise `NullPointerException`, including in `MessageDigest.update(byte[])` and `digest(byte[])`, except that `MessageDigest.update(byte[], int, int)` raises `IllegalArgumentException` with message `No input buffer given`. Other data errors include `NumberFormatException` for invalid numbers, `IndexOutOfBoundsException` for invalid list indexes and `NoSuchElementException` for exhausted iterators. Invalid non-null handles remain reference errors. Native code can inspect and clear pending exceptions with JNI exception functions. If native code returns with an uncleared exception, the error ends with ` from <location>`, naming the guest JNI call that raised it. This origin is retained for built-in answers, `Throw`, `ThrowNew` and handler or policy answers, and survives checkpoints.

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

Paths are normalized lexically: repeated slashes and `.` disappear; `..` removes one component and stops at `/`. Relative paths use working directory `/`, or the directory fd for `openat`, `newfstatat`, `faccessat`, `mkdirat`, `unlinkat`, `renameat`, `renameat2` and `readlinkat`. Invalid relative-path dirfds return `EBADF`; non-directory fds return `ENOTDIR`. Absolute paths ignore dirfd. Every path-taking file syscall returns `EFAULT` if its path cannot be read from guest memory. `faccessat` checks existence through the layers and still ignores permission modes and flags.

Symbolic links in rootfs and the overlay resolve in the guest namespace: absolute targets start at guest `/`, relative targets at the link's directory. Targets pass through overlay, whiteouts, generated files and rootfs; rootfs lookup never follows a link to a host file outside rootfs. Each lookup follows at most 40 links, then returns `ELOOP`. The last component is followed except by `mkdirat`, `openat` with `O_CREAT|O_EXCL`, `readlinkat`, `unlinkat`, `renameat`/`renameat2`, and `newfstatat` with `AT_SYMLINK_NOFOLLOW` (`0x100`). `mkdirat` and exclusive create return `EEXIST` for any final symlink, including a dangling one; plain `O_CREAT` still follows the link and can create its target. `newfstatat` with an empty path and `AT_EMPTY_PATH` (`0x1000`) stats `dirfd` itself, or `/` for `AT_FDCWD`.

`openat` accepts read and write access. A write open of a rootfs or host-linked file copies it into memory; `O_TRUNC` skips reading its old contents. `O_CREAT` creates a file only when its parent exists, with `(mode & 0o7777) | 0o100000`. `O_CREAT|O_EXCL` rejects an existing path with `EEXIST`; directories reject write opens with `EISDIR`; generated files reject them with `EACCES`. `O_APPEND` writes at the end, and writes past the end zero-fill the gap. Separate opens share file contents but keep separate offsets. `write`, `writev` and `pwrite64` accept at most 33554431 bytes per call; read-only fds reject writes with `EBADF`. `pwrite64` keeps the fd offset unchanged. As on Linux, append mode also places positional writes at the end. `ftruncate` changes the length; `fsync` and `fdatasync` return zero for any open fd, without writing to the host.

Overlay files have a 1 GiB maximum: writes starting at or beyond it and `ftruncate` beyond it return `EFBIG`, while writes crossing it return the shorter count that fits. A write open or rename that needs to copy a larger rootfs or host-linked file returns `EFBIG` before reading its contents. `O_TRUNC` still works because it skips the copy.

`read` and `pread64` return `EFAULT` if copying into guest memory fails, leaving the file offset unchanged. `pread64` never changes the file offset. `lseek` supports SEEK_SET/CUR/END, including positions past EOF; a bad whence, negative result or result above `i64::MAX` returns `EINVAL` without moving. Arm32 `_llseek` (libc `lseek64`) uses the same rules with a signed 64-bit offset assembled from high and low 32-bit words. It writes the new position as an eight-byte little-endian value through its result pointer and returns 0; an unmapped result pointer returns `EFAULT` after moving the offset. Both calls return `EBADF` for a bad fd, work with generated and host-linked files, and restart directories with offset 0 and SEEK_SET. Arm32 `lseek` keeps its signed 32-bit offset argument and returns `EOVERFLOW` when the resulting position exceeds `0x7fffffff`, after moving the offset. Arm64 `lseek` keeps its full 64-bit result.

`mkdirat` creates overlay directories with `(mode & 0o7777) | 0o040000`. `unlinkat` removes files, or empty directories with `AT_REMOVEDIR`; nonempty directories return `ENOTEMPTY`. Without `AT_REMOVEDIR`, it removes a symlink even if dangling or pointing to a directory. `renameat` moves files and directory trees and replaces a compatible target, copying rootfs sources into the overlay and hiding their old paths. Moving a rootfs symlink preserves its target text in an overlay symlink; these links can be followed, read, listed as `DT_LNK`, renamed and unlinked. There is no guest `symlinkat`. Rename updates the saved paths of all open descriptors at or below the source, so relative `*at` calls and `/proc/self/fd/N` follow the move. Arm64 also supports `renameat2` with flags zero.

Opening a directory, with or without `O_DIRECTORY`, takes a snapshot of its merged entries; `O_DIRECTORY` on a regular file returns `ENOTDIR`. `read` returns `EISDIR`; `getdents64` returns `.` and `..`, then entries sorted by name bytes, excluding whiteouts. Records use the Linux 64-bit layout and eight-byte alignment, with the next entry index as `d_off`. Calls walk the snapshot; zero means end of directory; a buffer too small for the next record returns `EINVAL` if nothing fit. A failed copy to guest memory returns `EFAULT` and keeps the directory position unchanged. `lseek(fd, 0, SEEK_SET)` restarts it. Directory enumeration works through libc's `opendir`, `readdir` and `closedir`; `/proc/self/fd` contents need not be enumerated.

`fcntl` supports `F_DUPFD`, `F_DUPFD_CLOEXEC`, `F_GETFD`, `F_SETFD`, `F_GETFL` and `F_SETFL`. Status flags hold access mode, `O_APPEND` and `O_NONBLOCK`; `F_SETFL` changes only the latter two. Duplicates share offsets and status flags, but each fd has its own close-on-exec flag. `dup` and `dup3` work, including libc's `dup2`; equal fds passed to `dup3` return `EINVAL`. Descriptors 0–2 start open and can be duplicated or closed. New descriptors use the lowest free number. Duplicated stdout/stderr descriptors emit the original stream's `Output` event; closing and reusing an fd changes what it refers to.

`/proc/self/cmdline` and `/proc/1234/cmdline` snapshot the process name followed by one NUL byte when opened. `1234` is the guest process ID. `readlinkat` on `/proc/self/fd/N` or `/proc/1234/fd/N` returns the resolved guest path of that fd, updated by rename, or `ENOENT` if closed. Initial standard streams use `/dev/stdin`, `/dev/stdout` and `/dev/stderr`. `/proc/self/exe` returns `/system/bin/app_process64` or `/system/bin/app_process32`; a rootfs or overlay symlink returns its target text, including for a dangling link. Existing non-links return `EINVAL`, missing paths `ENOENT`. Results contain at most `bufsiz` bytes, without a terminating NUL, and return the byte count.

The intentional departures from unidbg match Android and Linux: `/proc/self/exe` returns a real Android device's executable path, `readlink` returns the byte count without a terminating NUL, `faccessat` resolves relative paths against `dirfd`, and `_llseek` supports SEEK_CUR and adds the offset for SEEK_END; sorted directory listings make runs deterministic.

Overlay `stat`/`fstat` use the stored mode, content length, one link, device/uid/gid zero, block size 4096 and size rounded up to 512-byte blocks. The inode is the 64-bit FNV-1a hash of the normalized guest path. All timestamps use the fixed-clock seconds when set. Otherwise, a change marks the node; its first subsequent stat reads the guest realtime clock once and caches that change time until the next change. Path lookups and file changes never read the clock. Host-linked files use host metadata like rootfs until copied into memory. A symlink's own stat uses mode `0o120777`, size equal to the target text length, and rootfs metadata for the other fields, including the fixed-clock rules. Generated maps metadata remains as described below; cmdline uses the same generated-file metadata layout.

Checkpoints save the overlay, whiteouts, directory snapshots and open descriptors, including shared offsets, flags and close-on-exec. Unchanged file contents are shared through `Rc` until a write. Rootfs and host-linked files reopen from their saved host paths on rewind; keep those host files available and unchanged. See [DEBUGGING.md](DEBUGGING.md).

Guest `write` and `writev` to stdout/stderr descriptors (initially fd 1 and fd 2) emit `Event::Output { fd, text }` even with tracing off. Each call emits one event, with `writev` buffers concatenated in order before lossy UTF-8 decoding, and returns the number of bytes accepted. Both calls accept at most 33554431 bytes, the same limit as `getrandom` and random-device reads; `writev` shares this limit across entries and can stop partway through an entry. The default event printer writes to host stderr; the builder's `on_event` can capture the output. Text events use `stdout: TEXT` or `stderr: TEXT` and drop one trailing newline; JSON uses `{"event":"output","fd":1,"text":"hello\n"}` and preserves the text. See [DEBUGGING.md](DEBUGGING.md) for events.

Closed or read-only fds, including the initial stdin, reject writes with `-EBADF`; writable file descriptors update the overlay. Unmapped buffers or iovec arrays return `-EFAULT`. These and all other syscalls do not check guest page permissions, matching unidbg. `writev` accepts at most 1024 entries (`-EINVAL` above that), each a pointer and length: 16 bytes on arm64, 8 on arm32. Zero-length entries are allowed. Syscalls return negative Linux errno values; libc converts them to `-1` and sets errno.

`uname` writes all 390 bytes of `struct utsname`: six 65-byte fields, each string followed by zero padding. It returns 0, or `-EFAULT` if the buffer is unmapped. Values match unidbg 0.9.9:

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

- Loaded modules use device `b3:19`. APK libraries use `/data/app/<package>-1/lib/arm64/NAME` or `/data/app/<package>-1/lib/arm/NAME`; host-loaded modules use `/system/lib64/NAME` on arm64 or `/system/lib/NAME` on arm32, or `/system/bin/NAME` when their name has no `.so` suffix.
- The stack uses device `00:00` and label `[stack]`.
- Every other mapping uses device `00:00` and label `anonymous`.

The open file keeps its original bytes when mappings change. It supports `read`, `pread64`, `lseek` with SET/CUR/END, `fstat`, `mmap` and `close`; `newfstatat` and `faccessat` also recognize these paths. Checkpoints retain each open maps file's bytes and offset, and rewind restores both, including into a fresh emulator.

Maps file metadata follows unidbg's `ByteArrayFileIO.fstat`: `st_dev = 1`, `st_ino = 1`, `st_mode = 0o100000` (`S_IFREG`, no permission bits), `st_nlink = 0`, uid/gid/rdev zero, `st_size` equal to the snapshot length, `st_blksize = 4096`, and `st_blocks` equal to the length rounded up to 4096-byte units. All three timestamps use the supplied `fixed_clock` seconds with zero nanoseconds, without reading the clock; otherwise each stat reads the guest realtime clock, which is recorded and replayed. These values apply on both architectures; host files keep their existing metadata behavior.

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
