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
2. **M1:** ELF32 loader + dynamic linker (R_ARM REL relocations, PLT/GOT,
   init_array) and an ARMv5TE/Thumb-1 interpreter reaching `init_array`.
3. **M2:** JNI bridge both directions; `nativePreInit`/`nativeInit` complete;
   `Java_com_com2us_wrapper_WrapperUserDefined_StartGame` reachable.
4. **M3 (de-risk gate):** first frame rendered through `nativeRender` into the
   existing GLES 1.x command stream.
5. **M4:** menu navigation via touch (`WrapperEventHandler` -> `nativeEvent`).
6. **M5:** audio (`SoundManager` -> `MediaPlayer`/`SoundPool` backends).
7. **M6:** playable; save-data syscalls mapped to the sandbox directory.

## Current boot observation

`cargo run -- run ovenbreak.apk` fails honestly inside the Dalvik VM before
any native code matters:

```text
Dalvik VM error at pc 0 opcode 0x5b: null instance field target:
field=Lcom/com2us/wrapper/WrapperData;->packageName:Ljava/lang/String;
in Lcom/com2us/wrapper/WrapperData;->setPackageName
in Lcom/com2us/ovenbreak/.../MainActivity;->onCreate
```

`MainActivity.onCreate` runs with a `Null` implicit `this`-adjacent argument
path in `WrapperData.setPackageName`; the VM reaches the wrapper code but the
wrapper object is not initialized. Debugging this Dalvik-lifecycle gap is
prerequisite work for M1-M3 bring-up and is tracked as the next step after M0.
