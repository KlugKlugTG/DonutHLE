# DonutHLE — Full Development History and Feature Inventory

This document records **everything that was built in this repository**, derived from the complete git history (221 commits, 2026-08-14 → 2026-09-07) and the current source tree. It complements the [README](../README.md), which describes the current state, and [docs/ARCHITECTURE.md](ARCHITECTURE.md).

## What DonutHLE is

DonutHLE is an experimental **high-level emulator (HLE) for Android 1.x–2.x applications** (API levels 1–8), written in Rust, with an Android shell app (Java + JNI + a small C++ bridge). Instead of emulating a complete phone, it implements the historical Android APIs that old games actually use: a guarded Dalvik 035 bytecode interpreter, Android framework shims, a libGDX compatibility layer, a legacy GLES 1.x command stream rendered to a software framebuffer, and — since the September patches — a full **ARMv5TE/Thumb-1 machine that executes real native `.so` libraries from APKs**.

Primary compatibility targets: **Tiny Santa / Slice Ice** (Dalvik + Canvas/libGDX apps) and **OvenBreak** (native Com2uS "CSFB" engine + Samsung zirconia license check).

## Codebase size

- `src/` — 19,323 lines of Rust across 23 modules.
- `android/app/src/main/java/org/donuthle/android/` — 953 lines of Java (7 classes) plus JNI C++ bridge, Gradle build, manifest, resources, launcher icons.
- `tests/` — `core.rs` (core regression tests) and `native_loader.rs` (loads and links real OvenBreak `.so` files; activated with `DONUTHLE_OVENBREAK_LIBS`).
- CI: `.github/workflows/ci.yml` (fmt/test/clippy on Linux) and `build-donuthle.yml` (Linux, Windows, Android artifacts with SHA-256 checksums; tag → GitHub release).

## Feature inventory by subsystem

### APK foundation (`apk.rs`, `manifest.rs`, `resources.rs`, `compat.rs`)
- Safe ZIP/APK inspection with deterministic file listing.
- Android binary XML (`AndroidManifest.xml`) parsing and launcher-activity resolution.
- Partial `resources.arsc` resource-table decoding.
- DEX 035 header parsing and validation, hardened against malformed legacy headers (bogus sizes, resource bytes, header-size fields).
- Centralized compatibility-feature registry: APK DEX references → gap report; every unsupported API is logged, never silently claimed.

### Dalvik VM (`dalvik.rs`, `vm.rs`)
- Guarded Dalvik 035 interpreter: register bounds checks, call-depth (256) limit and a per-invocation step budget (50M steps, resets at each top-level VM entry: boot lifecycle calls, game activation, frames, touch dispatch), optional register tracing, visible VM errors with pc/opcode. Rendering frames get the same budget per frame; an exceeded frame budget unwinds the frame and is logged.
- Value model: int/long/float/double/object/string/null, with correct wide (J/D) register pairing, wide move/arithmetic/conversion opcodes, shift-count decoding, divide-by-zero handling, boxed primitives, high16 constants.
- Full string/int/bool/`Math` (incl. trigonometry and round) library behavior, `StringBuilder`, boxed collections, `java.lang.Class.forName` with argument validation.
- Arrays: creation, reads/writes, length, multi-dimensional arrays, bounds errors; later mirrored bidirectionally into JNI arrays for native calls.
- Method dispatch: static/virtual/interface, inherited Activity lifecycle resolution, declared-code-before-inherited-fallback, DEX method-owner lookup, abstract-lifecycle handling.
- Class initialization, static fields, comparison/switch payloads (packed/sparse).

### Android 1.x–2.x framework shims (`framework.rs`, `runtime.rs`)
- Activity, Context, Application, View hierarchy, WindowManager, Looper/message queue, lifecycle (onCreate→onResume→onPause), touch/key input callbacks.
- Framework services reported by the log: PackageManager, TelephonyManager, ClassLoader, `java.io.File`, broadcast receivers, application-context methods.
- Launcher boot pipeline: constructor chain, singleton game-data classes, per-game activation, dynamic window title, reset-and-rerun of the runtime.
- 320×480 virtual screen, Android 1.x–2.x target profile with API 8 default.

### Graphics (`gles.rs`, `gles1_on_gl2.rs`, `gles_native.rs`, `assets.rs`)
- Software framebuffer with viewport, scissor, depth, matrix stacks, blending, clear state.
- GLES 1.x fixed-function compatibility adapter: client arrays, fixed-point conversion, OES matrix-palette CPU skinning, indexed and array draws.
- GLES1-on-GL2 backend (desktop always; Android presentation through a GLES 2.0 surface), plus a native GLES1 presentation path.
- Texture sampling and depth-write improvements; `TextureRegion`/`TextureAtlas`/atlas lookup; `SpriteBatch` textured-quad rendering.
- PNG/JPEG decoding of APK assets (magic-byte based, handles `assets/game_res/*.png.jpg` quirks), normalized `assets/` paths, legacy Canvas sprite rendering with alpha.
- Persistent render sessions so an application listener renders across frames; framebuffer orientation fixes (Android and native).

### Native-code execution (`arm.rs`, `elfload.rs`, `mem.rs`, `native.rs`, `native_bridge.rs`, `jni.rs`, `host.rs`)
- ELF32 inspection: header, dynamic section, symbol/import/export/JNI inventories, ARM build attributes; per-launch diagnostics.
- Sparse guest memory (`mem.rs`); ELF loader + dynamic linker with R_ARM relocations (`.rel.dyn` + `.rel.plt`/DT_JMPREL), writable backing for imported data symbols (`__page_size`, `__stack_chk_guard`, `__dso_handle`, `__sF`).
- ARMv4T + v5TE interpreter (`arm.rs`): BLX, CLZ, DSP multiplies, QADD saturation family, LDRD/STRD, SWP, full Thumb-1; unsupported classes (VFP, coprocessors, Thumb-2, v6 atomics) stop with a visible fault; 64-entry PC trace in fault dumps.
- Bionic libc/libm/libstdc++/liblog shims (`host.rs`) including the full `__aeabi_*` soft-float set; `vsprintf` modeled with correct ARM EABI va_list/8-byte alignment; network and file operations fail closed with logs.
- JNI environment (`jni.rs`) with handles for Java arrays/strings, `Get*ArrayElements` vtable behavior, scratch storage for unknown handles.
- Dalvik→native bridge (`native_bridge.rs`): ACC_NATIVE dispatch, `System.loadLibrary` loads real `.so` into the machine, marshalling to `(env, jclass, args…)`, heap snapshot/restore around each dispatch.
- GL imports bound to the software rasterizer through `BasicHost::call_gl`.
- OvenBreak native boot progress: full zirconia license chain executes end-to-end (real ARM SHA-1 code), `nativePreInit` completes, `initialize()` driven as Dalvik code; remaining stop: engine C++ construction-order dependency in `nativeInit` (documented with trace).

### Android shell (`android/`)
- Game Library (import/scan/inspect/launch APKs), Game Sandbox (app-owned writable storage), Emulator Log (UTF-8, deterministic).
- Storage under `DonutHLE/` external-files dir; DocumentsProvider-backed custom root, file picker with remembered tree URI, APK folder scanning.
- Compatibility log records launch attempts, imports, runtime failures, and every unsupported/incomplete compatibility path when reached.
- Launcher icon + branding, GLES 1.x `SurfaceView`, touch bridge, JNI framebuffer export.
- CI cross-compiles the Rust core for `arm64-v8a`, `armeabi-v7a`, `x86_64` and packages debug/release APKs.

### Documentation and tests
- `README.md`, `docs/ARCHITECTURE.md`, `docs/ovenbreak.md` (measured native-target profile, policy decisions, milestone roadmap M0–M6), `docs/tiny-santa-crash.md` (open first-frame crash, reproduced with trace), `docs/actions-build.md`, `android/README.md`, `CONTRIBUTING.md`, `SECURITY.md`, MIT `LICENSE`.
- `tests/core.rs`: VM/framework regressions. `tests/native_loader.rs`: real-library load, link (949 + 23 relocations), JNI resolution, `init_array` execution.
- Clean-room policy: AOSP/BSD reference sources consulted read-only to re-derive shim behavior, never compiled in or shipped.

## Notable bugs the real binaries flushed out
- ARM decoder: LDR/STR bit 25 inverted; Thumb format-2 operand positions; Thumb format-5 `Rt` vs `Rm`; Thumb format-7 B/L bits swapped; unsigned branch offsets; missing DT_JMPREL; `R_ARM_RELATIVE = 23` (not 8).
- Dalvik: `invoke_args` mishandling wide (J/D) argument pairs; goto/16 branch decoding; array-length destination register; high16 constant types; wide register writes; boxed unboxing.
- Bridge: ACC_NATIVE dispatch ordered after framework-owner fallback (misrouting native methods to `java/lang/Object`); class-name descriptors unmangled before JNI symbol lookup; JNI host objects bumped from address 0.

## Full commit history

(oldest → newest, all 221 commits)

- `fef50e4` 2026-08-14 Initial DonutHLE Android 1.6 HLE prototype
- `efcfae0` 2026-08-14 Add Android Studio native build shell
- `1a50d70` 2026-08-14 Omit CI workflow from Android build push
- `f886da0` 2026-08-14 Add Build DonutHLE GitHub Actions workflow
- `67271b9` 2026-08-14 Publish build workflow files separately
- `a0ce4af` 2026-08-14 Document GitHub Actions build
- `b6f5b34` 2026-08-14 Add Build DonutHLE GitHub Actions workflow
- `4cbf25a` 2026-08-14 Remove unused AndroidX dependency
- `8721755` 2026-08-14 Add game library sandbox and redesigned Android menu
- `0738484` 2026-08-14 Fix DocumentsProvider Android compilation
- `f223ee5` 2026-08-14 Fix Android file picker, options screen, and UTF-8 logs
- `1748fa8` 2026-08-14 Repair Android storage paths and import handling
- `f023054` 2026-08-14 Fix APK library navigation and UTF-8 log display
- `72381b1` 2026-08-14 Add explicit compatibility and unimplemented-feature logging
- `553dadc` 2026-08-14 Add library screen and actionable compatibility log
- `2d2b02b` 2026-08-14 Fix APK folder scanning and UTF-8 log storage
- `002b234` 2026-08-14 Add DEX compatibility diagnostics
- `7c32125` 2026-08-14 Add compatibility log collection
- `b3ba554` 2026-08-14 Connect library, APK import, launch diagnostics, and options screens
- `0e41cd4` 2026-08-14 Make compatibility log deterministic and record unimplemented features
- `f1e42e4` 2026-08-14 Remove unsupported stream API from compatibility logging
- `9047d24` 2026-08-14 Add compatibility gap logging for requested emulator features
- `b31f714` 2026-08-14 Fix compatibility logger API and record missing emulator features
- `5f751bd` 2026-08-14 Fix APK library, folder import, and compatibility log UI
- `d486bef` 2026-08-14 Record unimplemented Android and game-requested features in the log
- `acb748e` 2026-08-14 Log unimplemented compatibility features
- `f38d96e` 2026-08-14 Log unimplemented compatibility features
- `e7d5c89` 2026-08-14 Record APK-requested compatibility gaps
- `53ead0e` 2026-08-14 Log unimplemented compatibility features
- `1a3206f` 2026-08-14 Scan APKs and show compatibility gaps in log
- `1f9b032` 2026-08-14 Record APK compatibility gaps and unimplemented runtime features in UTF-8 log
- `9efb8f2` 2026-08-14 Fix Android APK compatibility build
- `1347cba` 2026-08-14 Implement Donut APK parsing and runtime foundation
- `aaf10a9` 2026-08-14 Add emulator runtime subsystems and launch pipeline
- `cab53f7` 2026-08-14 Add Dalvik VM and Android framework runtime
- `29f5993` 2026-08-14 Fix Dalvik array-length decoding
- `db383d2` 2026-08-14 Wire resources and activity objects into VM boot
- `074026a` 2026-08-14 Add DonutHLE Android launcher icon
- `e9e8a0a` 2026-08-14 Link Rust runtime to Android APK launcher
- `ce74d68` 2026-08-14 Fix Android launcher string literal
- `98cb2fb` 2026-08-15 Build and package the Rust core for Android ABIs
- `69eac38` 2026-08-15 Fix Android NDK compiler environment for native dependencies
- `e1feac8` 2026-08-15 Add centralized compatibility feature registry
- `db14de8` 2026-08-15 Implement compatibility report generation from APK DEX references
- `d6d36ea` 2026-08-15 Merge centralized Android compatibility feature registry
- `a98d4dc` 2026-08-15 Fix DEX header validation and DonutHLE folder navigation
- `d29fab5` 2026-08-15 Format DEX validation error
- `8d6a0a5` 2026-08-15 Handle transformed DEX header errors without panicking
- `05fb2e4` 2026-08-15 Make DonutHLE folder actions open the custom provider root
- `c33bb38` 2026-08-15 Keep implemented compatibility status in the Android log
- `13f9c0e` 2026-08-15 Guard DEX parser against APK resource bytes
- `7033cce` 2026-08-15 Open custom DonutHLE provider root with APK picker fallback
- `8c59235` 2026-08-15 Keep DEX header validation bounded and readable
- `7218379` 2026-08-15 Fix DEX header validation for Android 1.6 APKs
- `6cc8913` 2026-08-15 Fix DonutHLE folder navigation and remembered tree URI
- `a3c4b1d` 2026-08-15 Avoid trusting bogus DEX header sizes from malformed APKs
- `eafcee3` 2026-08-15 Handle legacy DEX headers and make folder picker reliable
- `dcbb07a` 2026-08-15 Implement framework services reported by DonutHLE log
- `543edcf` 2026-08-15 Fix framework clippy warning
- `e2809bf` 2026-08-15 Tolerate malformed DEX size fields from legacy APKs
- `448d0ef` 2026-08-15 Resolve inherited Activity lifecycle methods in Dalvik VM
- `7b41d7e` 2026-08-15 Implement java.lang.Class.forName in Dalvik framework
- `d6558eb` 2026-08-15 Handle abstract lifecycle calls and validate Class.forName arguments
- `8969eba` 2026-08-15 Fix Dalvik argument decoding and framework lifecycle compatibility
- `c3f18c0` 2026-08-15 Preserve source method code when resolving inherited methods
- `55d3f14` 2026-08-15 Preserve source method code when resolving inherited methods
- `c4e7b63` 2026-08-15 Dispatch framework stubs correctly and reject non-string Class.forName args
- `4976cc7` 2026-08-15 Read DEX header size from the correct header field
- `b082e10` 2026-08-15 Fix Dalvik invoke register order for legacy Unity APKs
- `2166fc8` 2026-08-15 Launch Stuntman Steve through Unity Android bootstrap
- `5d18b76` 2026-08-15 Add framework method owner lookup to DEX model
- `ef8e347` 2026-08-15 Expand Dalvik arithmetic opcode coverage for Slice Ice
- `8c43497` 2026-08-15 Implement static fields and comparison opcodes for Slice Ice
- `3dfdac4` 2026-08-15 Prefer declared DEX method code before inherited fallback
- `423d5b4` 2026-08-15 Support Slice Ice DEX method lookup and legacy control flow
- `97433ec` 2026-08-15 Add Math.round compatibility for Slice Ice startup
- `d7f4294` 2026-08-15 Implement Dalvik float arithmetic for Slice Ice
- `ef4cd37` 2026-08-15 Correct numeric coercion for Slice Ice Math calls
- `58fc916` 2026-08-15 Implement Android window timing and wide numeric APIs
- `65346ce` 2026-08-15 Implement Java Integer and Boolean parsing for Slice Ice
- `c2d43d1` 2026-08-15 Pass strict CI after Android compatibility work
- `a4621f0` 2026-08-15 Implement libGDX and OpenGL ES compatibility layer
- `83a0ad8` 2026-08-15 Clarify launcher completion result
- `6fc7fc9` 2026-08-16 Advance Slice Ice runtime compatibility
- `c29e63c` 2026-08-16 Fix Rust core formatting
- `d375c4e` 2026-08-16 Handle Layer addChild calls
- `5cc2fe5` 2026-08-16 Implement Android application context methods
- `de984e9` 2026-08-16 Implement Android broadcast receiver registration
- `c0e8523` 2026-08-16 Make Layer addChild argument handling explicit
- `37b0ac1` 2026-08-16 Finalize Hyperkani Layer compatibility routing
- `7bb7635` 2026-08-16 Preserve Hyperkani framework receiver objects
- `bb2a688` 2026-08-16 Extend libGDX Android runtime compatibility
- `230dec4` 2026-08-16 Fix array-length destination register decoding
- `d9117bc` 2026-08-16 Implement java.lang.String value conversions
- `178a7a4` 2026-08-16 Avoid manually editing large generated source
- `1b07593` 2026-08-16 Fix java.lang.String dispatch ordering
- `d83d86a` 2026-08-16 Format Rust core for CI
- `84d060e` 2026-08-16 Restore libGDX dispatch and fix array writes
- `f308539` 2026-08-16 Fix strict Rust CI lint errors
- `1879849` 2026-08-16 Implement common String operations for Slice Ice
- `920cdb1` 2026-08-16 Expand Slice Ice Android framework compatibility
- `4b2e6b2` 2026-08-16 Add Dalvik numeric conversions for Slice Ice
- `ade4983` 2026-08-16 Implement Dalvik literal and floating arithmetic
- `2757277` 2026-08-16 Fix Dalvik long arithmetic for Slice Ice
- `84f300e` 2026-08-16 Fix wide Dalvik register writes
- `654a534` 2026-08-16 Decode long shift counts as int
- `38f8f79` 2026-08-16 Implement Dalvik wide move opcodes
- `6d69077` 2026-08-16 Fix wide arithmetic and divide-by-zero handling
- `da91a8f` 2026-08-16 Fix int to double wide conversion
- `e195636` 2026-08-17 Improve Dalvik arrays, wide values, and Slice Ice runtime
- `97735e1` 2026-08-17 Keep Slice Ice VM compatible with Rust 1.63
- `eac3e41` 2026-08-17 Add native GLES1 Android presentation path
- `eccaf36` 2026-08-17 Fix duplicate GLES native export
- `8a26e91` 2026-08-17 Fix Clippy warnings in VM
- `f4898c4` 2026-08-17 Expand GLES renderer and VM compatibility
- `f5542d1` 2026-08-17 Fix Dalvik goto/16 branch decoding
- `3b36d4c` 2026-08-17 Present the real GLES framebuffer on Android
- `1bbd01a` 2026-08-17 Restore Android GLES frame export
- `fed97b5` 2026-08-17 Present Slice Ice software framebuffer on Android
- `83873e8` 2026-08-17 Format framebuffer export
- `d7dcf48` 2026-08-17 Document framebuffer copy safety
- `e798776` 2026-08-17 Fix Android framebuffer bridge linkage
- `1aab310` 2026-08-17 Implement libGDX asset loading and textured GLES draws
- `288a566` 2026-08-17 Merge remote-tracking branch 'origin/main'
- `740eaaf` 2026-08-17 Keep Slice Ice render session alive across frames
- `3303eaa` 2026-08-17 Document DonutHLE status and roadmap
- `0d4cea7` 2026-08-18 Advance Slice Ice render compatibility
- `c094572` 2026-08-18 Adapt GLES1 on GL2 backend for desktop
- `4583657` 2026-08-18 Always use GLES1-on-GL2 on desktop
- `3c9d55a` 2026-08-18 Fix Slice Ice screen layer compatibility
- `0edd834` 2026-08-18 Render Sprite draw calls in Slice Ice
- `ace2038` 2026-08-18 Fix Slice Ice pixel-space sprite rendering
- `fbced55` 2026-08-18 Improve GLES texture sampling and depth writes
- `2f07d3a` 2026-08-18 Fix Rust core Clippy failures
- `d0f2ca8` 2026-08-18 Fix Slice Ice texture regions and 2D coverage
- `4fdaa24` 2026-08-18 Keep GLES rendering fix Clippy clean
- `ecba608` 2026-08-18 Fix Android GLES framebuffer presentation
- `5a5b06d` 2026-08-18 Fix Windows artifact startup and runtime errors
- `3de2d79` 2026-08-18 Fix Slice Ice atlas resolution and black-screen fallback
- `8b5ad91` 2026-08-18 Fix Slice Ice startup rendering and Android game surface
- `d1a53aa` 2026-08-18 Fix Slice Ice sprite scaling and Dalvik float constants
- `1b2948f` 2026-08-19 Fix Dalvik high16 constant types
- `3c475b6` 2026-08-19 Fix libGDX sprite draw geometry
- `e77076c` 2026-08-19 Fix Slice Ice frame presentation and sprite draws
- `375f143` 2026-08-19 Fix libGDX atlas region rendering
- `3b49160` 2026-08-20 Fix libGDX splash timing
- `ea1bb6f` 2026-08-20 Expand target to Android 1.x emulator
- `11cf8b0` 2026-08-21 Add Tiny Santa Android 1.x compatibility shims
- `d6bd00c` 2026-08-21 Fix Linux Clippy compatibility in asset decoding
- `01d46a8` 2026-08-21 Fix Clippy lint without changing asset APIs
- `5b7ae25` 2026-08-21 Fix Clippy chunks_exact lint in asset decoding
- `d9da749` 2026-08-21 Render Tiny Santa legacy Canvas on Linux
- `1b67738` 2026-08-21 Install X11 dependency for Linux CI
- `501b950` 2026-08-21 Fix latest Clippy chunk lints
- `0b615da` 2026-08-21 Show the launched APK title dynamically
- `0f5bae9` 2026-08-21 Format dynamic title changes
- `522158c` 2026-08-23 Reset runtime and preserve game-specific rendering
- `92a95e7` 2026-08-23 Add Tiny Santa Canvas gameplay input path
- `3b5915c` 2026-08-23 Fix Android touch callback visibility
- `adcc573` 2026-08-23 Support common Android framework calls
- `d4c0bf8` 2026-08-23 Fix boxed primitive unboxing
- `5d89702` 2026-08-24 Handle primitive reads from null view fields
- `03d6941` 2026-08-24 Implement Java Math trigonometric methods
- `f67f38f` 2026-08-24 Fix Android game surface presentation
- `3ba9726` 2026-08-24 Report requested Android compatibility layers accurately
- `d361e4c` 2026-08-24 Render Tiny Santa through its Canvas path
- `ee42c4d` 2026-08-24 Add playable Tiny Santa gameplay scene
- `381f72e` 2026-08-24 Make Tiny Santa gameplay interactive
- `dde3198` 2026-08-24 Fix Tiny Santa sprite transparency
- `4c419b7` 2026-08-24 Run Tiny Santa through original APK code
- `62a6507` 2026-08-24 Implement Android framework compatibility stubs
- `fdd284a` 2026-08-24 Render Tiny Santa Canvas sprites with alpha
- `4d46090` 2026-08-24 Remove custom Tiny Santa rendering
- `216cdf5` 2026-08-24 Remove all handcrafted Tiny Santa rendering
- `f20ae6f` 2026-08-24 Format removal of custom Tiny Santa path
- `4996d65` 2026-08-24 Remove Tiny Santa-specific runtime fallback
- `6cee5b9` 2026-08-24 Render Android view apps and expose missing API diagnostics
- `caf7337` 2026-08-24 Remove custom emulator gameplay overlay
- `c558432` 2026-08-24 Fix legacy Canvas launch and frame routing
- `3514e5a` 2026-08-24 Remove synthetic fallback game demo
- `95c1c74` 2026-08-24 Support Dalvik switch payloads
- `7b59839` 2026-08-24 Fix Android framebuffer orientation
- `6e3f82d` 2026-08-24 Add optional Dalvik register tracing
- `ca6c21c` 2026-08-24 Remove synthetic sprite fallback
- `7646aba` 2026-08-25 Correct native framebuffer orientation
- `f77e8d1` 2026-08-25 Implement core GLES frame state and transforms
- `5baff46` 2026-08-25 Format GLES compatibility changes
- `d1886e5` 2026-08-25 Resolve legacy Canvas launch PR with current main
- `649066e` 2026-08-25 Restore trace configuration in merged runtime
- `97e61d5` 2026-08-25 Merge legacy Canvas launch fix
- `9610e70` 2026-08-25 Initialize Tiny Santa gameplay before rendering
- `660706e` 2026-08-25 Run Tiny Santa gameplay initialization and restore resource drawables
- `8a3b779` 2026-08-25 Refresh Android app UI and project branding
- `9091e50` 2026-08-25 Fix Tiny Santa activation method lookup
- `447a765` 2026-08-25 Satisfy clippy for Tiny Santa lookup
- `74964eb` 2026-08-25 Fix Tiny Santa activation descriptor
- `2602cf4` 2026-08-25 Fix Tiny Santa activation descriptor format
- `f4ac0f2` 2026-08-25 placeholder
- `299a189` 2026-08-25 Restore Dalvik VM implementation
- `9742cae` 2026-08-25 Implement Hashtable framework methods
- `37ffe8a` 2026-08-25 Fix Tiny Santa activation and Hashtable APIs
- `e0b3bd7` 2026-08-25 Merge latest Hashtable implementation
- `14349cb` 2026-08-25 Document remaining Tiny Santa first-frame crash
- `bfd528b` 2026-08-25 Fix wide Dalvik values and guard Tiny Santa array rendering
- `40de7a9` 2026-08-25 Merge remote-tracking branch 'origin/main' into pr-1-resolved
- `25bea04` 2026-08-25 Format Dalvik runtime fixes
- `6432da4` 2026-08-25 Satisfy strict collection lint
- `9ef955c` 2026-08-25 Add Android 2.x and GLES 2.0 compatibility
- `dfaa4e2` 2026-09-03 Add native-library detection and ELF inventory
- `6f91597` 2026-09-04 Add native-code execution: ARM interpreter, ELF loader, host shims
- `0f7c2db` 2026-09-04 Back imported data symbols with writable guest memory
- `472c087` 2026-09-05 Complete OvenBreak launcher boot in the Dalvik VM
- `d4a9992` 2026-09-05 Bridge Dalvik native-method calls into the ARM machine
- `9eb0997` 2026-09-05 Bind GL imports to the rasterizer; nativePreInit executes
- `ef3798e` 2026-09-05 Execute the zirconia license chain through the native machine
- `424f9a4` 2026-09-05 Drive the wrapper's real initialize() as Dalvik code
- `38c7f36` 2026-09-06 Mirror Dalvik arrays into JNI arrays for native calls
- `633ed86` 2026-09-07 Fix vsprintf va_list ABI; drive the donut renderer lifecycle
- `f523659` 2026-09-07 Add stack dump to native fault diagnostics
- `dd71ca1` 2026-09-07 Add full development log: complete feature inventory and all 219 commits
- `282fe89` 2026-09-07 Fix instruction limit: per-invocation Dalvik step budget
