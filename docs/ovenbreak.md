# OvenBreak compatibility target

`apks/ovenbreak.apk` is the second compatibility target (after Tiny Santa). It
is a **native-code** application, which makes it the driver for the native
execution roadmap below.

## Application profile

- Package: `com.com2us.ovenbreak.normal.paidfull.samsungapps.kr.android.samsung`
- versionCode 1 (1.0.0), `minSdkVersion: 4` (Android 1.6), API 1-8 target range
- Landscape (`screenOrientation="landscape"`, expected frame 480x320), fullscreen theme
- Launcher: `...samsung.MainActivity` extends `com.com2us.wrapper.WrapperActivity` extends `android.app.Activity`

## Architecture

The game logic lives entirely in native libraries; the Java side is a thin wrapper.

```text
MainActivity (Java)
 └─ GLSurfaceView + WrapperRenderer (GLSurfaceView.Renderer)
     └─ WrapperJinterface.nativeInit / nativeRender / nativeEvent ...
         └─ libgame.so   Com2uS "CSFB" engine, all game logic, ARM 32-bit
             └─ JNI callbacks back into com.com2us.wrapper.* for audio, UI, input
libnativeinterface.so   Samsung "zirconia" license check (11 KB)
```

Measured facts (ELF build attributes and dynamic-symbol inventory):

- ARMv5TE, Thumb-1 (no Thumb-2, no VFP). Float math uses the soft-float ABI
  (`__aeabi_fadd`/`f2i`/`d2f`-style imports), so no FPU emulation is required —
  the `__aeabi_*` helpers can be implemented natively in Rust.
- Single-threaded engine: no `pthread_create` import; only mutexes and TLS.
- Imports: ~50 GLES 1.0 fixed-function calls (map onto the existing
  `gles.rs` command stream), ~120 libc/libm/libstdc++/liblog symbols
  (including `socket`/`connect`/`getaddrinfo` for score submission),
  2205 exported symbols, 20 JNI entry points (`Java_com_com2us_wrapper_*`).
- Dependencies: `libc.so`, `libstdc++.so`, `libm.so`, `libGLESv1_CM.so`,
  `libdl.so`, `liblog.so`.
- Asset naming quirk: `assets/game_res/*.png.jpg` / `*.dat.jpg` — decode by
  magic bytes, never by file-name extension.

## Policy decisions

- **License check:** the zirconia check is part of the APK's own code. The
  core executes the APK as-is (Dalvik classes run in the VM, native callbacks
  run in the interpreter like any other code) and implements no license logic
  of its own.
- **Network:** score submission and analytics are stubbed fail-closed with log
  entries; no sockets are opened.

## Native execution roadmap

1. **M0 (done):** native-library detection in `inspect`/`validate`/launch
   reports; ELF32 header, dynamic section, and symbol inventory in
   `src/native.rs`; `System.loadLibrary` logs that native code will not run.
2. **M1 (done):** ELF32 loader + dynamic linker and an ARMv5TE/Thumb-1
   interpreter reaching `init_array`.
   - `src/mem.rs` sparse guest memory; `src/elfload.rs` program loading,
     R_ARM relocations (`.rel.dyn` + `.rel.plt`/DT_JMPREL), host-symbol
     binding with diagnostic slots for unimplemented imports.
   - `src/arm.rs` CPU: ARMv4T + v5TE additions (BLX, CLZ, DSP multiplies,
     QADD family, LDRD/STRD, SWP) and full Thumb-1; unsupported classes
     (VFP, coprocessor, v6 atomics, Thumb-2) stop with a visible fault.
   - `src/host.rs` bionic libc/libm shims plus the full `__aeabi_*`
     soft-float set; network and file operations fail closed with logs.
   - Verified against the real libraries: both `.so` files load and link
     (949 + 23 relocations), every JNI entry point resolves, and
     `libgame.so`'s `init_array[0]` executes cleanly
     (`tests/native_loader.rs`, set `DONUTHLE_OVENBREAK_LIBS`).
   - Decoder bugs the real binary flushed out: LDR/STR immediate/register
     bit (25) inverted, Thumb format-2 operand positions, unsigned branch
     offsets, missing DT_JMPREL, and ARM R_ARM_RELATIVE = 23 (not 8).
3. **M2 (in progress):** JNI bridge both directions; `nativePreInit`/
   `nativeInit` complete; `Java_com_com2us_wrapper_WrapperUserDefined_StartGame`
   reachable.
   - Imported data symbols (STT_OBJECT) are auto-backed with writable guest
     memory in `0x7200_0000..`, with deterministic values for `__page_size`
     (4096), `__stack_chk_guard`, and `__dso_handle`; `__sF` is zeroed and
     readable. Verified on the real libraries: the four data imports are
     backed and the unresolved import count dropped to 50/6 (from 53/7).
4. **M3 (de-risk gate):** first frame rendered through `nativeRender` into the
   existing GLES 1.x command stream.
5. **M4:** menu navigation via touch (`WrapperEventHandler` -> `nativeEvent`).
6. **M5:** audio (`SoundManager` -> `MediaPlayer`/`SoundPool` backends).
7. **M6:** playable; save-data syscalls mapped to the sandbox directory.

## Current boot observation

`cargo run -- run ovenbreak.apk` completes the launcher activity:

```text
booted launcher; onCreate complete; 2 native libraries loaded by the wrapper
launcher: com.com2us.ovenbreak.normal.paidfull.samsungapps.kr.android.samsung.MainActivity
```

`MainActivity.<init>` through `WrapperActivity.<init>` (constructor chain,
singleton `WrapperData` materialized) and the whole of `onCreate` — including
`InitializeSecurityModule()` (executed as-is per the license policy) — now run
in the Dalvik VM. Fixes along the way: the launcher constructor chain was never
executed, `invoke_args` mishandled wide (J/D) argument pairs, view shims sat
behind an unreachable catch-all, and `getPackageName`/`PackageManager`/
`TelephonyManager`/`ClassLoader`/`java.io.File` shims were missing.

## Dalvik -> native bridge (M2)

`System.loadLibrary("game")` in `MainActivity.<clinit>` now loads
`lib/armeabi/libgame.so` into the ARM machine for real: the boot message
reports `loaded libgame.so: 949 relocations, 51 unresolved imports`. The
`Vm.native_dispatch` hook (vm.rs) routes ACC_NATIVE methods and loadLibrary
into `src/native_bridge.rs`, which owns the machine, stages library bytes from
the APK, installs the `jni.rs` environment, and marshals Dalvik values into
`(env, jclass, args...)` with JNI handles for Java arrays/strings.

The remaining step to a first frame is dispatching the wrapper's render loop:
`WrapperRenderer.onDrawFrame` -> `WrapperJinterface.run()` ->
`nativeRender(timerIndex)` and the earlier `nativePreInit`/`nativeInit` calls,
now that their entry points resolve in the machine.
