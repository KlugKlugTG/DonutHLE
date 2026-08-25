# DonutHLE

![DonutHLE app icon](docs/images/donuthle-logo.png)

**DonutHLE** is an experimental high-level emulator for applications built for Android 1.x-2.x (API levels 1–8). It replaces selected historical Android APIs with host-side implementations instead of emulating a complete phone. The core is written in Rust; the Android app is a modern shell connected through JNI.

> **Status: research prototype.** DonutHLE can inspect APKs, resolve launchers, execute a growing subset of Dalvik 035 bytecode, boot selected activity paths, load common libGDX assets, record and rasterize a legacy GLES command stream, and present the framebuffer through an Android GLES 2.0 surface. Compatibility remains application-specific and incomplete.

## Start here

### Android app

1. Download a debug or release APK from the repository's **Actions → Build DonutHLE → Artifacts** page, or build it locally.
2. Install the APK on an Android 6.0+ device or emulator.
3. Open **DonutHLE** and choose **Game Library → Import APK**.
4. Select an APK you own or are legally allowed to analyze.
5. Press **Run / Inspect**. The app records compatibility diagnostics in its emulator log.

The Android shell includes three focused areas:

- **Game Library** — import, scan, inspect, and launch APKs.
- **Game Sandbox** — open the app-owned writable storage area.
- **Emulator Log** — review startup, launch, unsupported API, and runtime errors.

Files are stored below the app's external-files directory in `DonutHLE/`:

```text
DonutHLE/
├── DonutHLE_apps/       imported APK files
├── DonutHLE_sandbox/    writable per-game data
├── DonutHLE_log.txt     UTF-8 compatibility log
├── DonutHLE_options.txt local options notes
└── README.txt           storage explanation written by the app
```

### Command line

Inspect an APK without executing it:

```sh
cargo run -- inspect path/to/game.apk
```

Validate its archive, manifest, resources, and `classes.dex` header:

```sh
cargo run -- validate path/to/game.apk
```

Run the experimental desktop path:

```sh
cargo run -- run path/to/game.apk
```

The Windows artifact is a console executable. Run it from PowerShell with an APK argument:

```powershell
DonutHLE-windows-x86_64.exe run path\to\game.apk
```

## What works today

### APK and Android foundation

- Safe ZIP/APK inspection with deterministic file listing.
- Android binary XML manifest parsing and launcher resolution.
- Dalvik 035 header parsing and a guarded interpreter with register bounds checks, call-depth/step limits, and unsupported-call diagnostics.
- Android 1.x–2.x target profile covering API levels 1–8, with API 8 as the default and a 320×480 virtual screen.
- Partial resource-table discovery and `resources.arsc` decoding.
- Activity, Context, View, lifecycle, message queue, input, audio, and resource framework shims.

### Graphics and libGDX compatibility

- Software framebuffer with viewport, scissor, depth, matrix, blending, and clear-state support.
- Legacy GLES compatibility modeled on a fixed-function adapter, with Android presentation on a GLES 2.0 context.
- Client-array staging, fixed-point conversion, matrix stacks, OES matrix-palette CPU skinning, indexed and array draws.
- PNG/JPEG APK asset decoding and normalized `assets/` paths.
- libGDX-style textures, `TextureRegion`, `TextureAtlas`, atlas lookup, and `SpriteBatch` textured quad rendering.
- Android JNI framebuffer export and host GLES presentation.
- Persistent render sessions so an application listener can render across frames.

### Diagnostics

DonutHLE is designed to fail visibly rather than silently claiming compatibility. The Android log records launch attempts, imports, runtime failures, and unsupported or incomplete compatibility paths when they are reached. A successful boot or visible frame is not a claim that the game is fully playable.

## Build locally

Install the stable Rust toolchain. For Android, use Java 17, Android SDK Platform 35, Build Tools 35.0.0, NDK 27.0.12077973, CMake 3.22.1, and Gradle 8.13.

### Rust core

```sh
cargo fmt --all -- --check
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

### Android APK

Open `android/` in Android Studio or run:

```sh
cd android
gradle assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n org.donuthle.android/.MainActivity
```

GitHub Actions cross-compiles the Rust static library for `arm64-v8a`, `armeabi-v7a`, and `x86_64`, then packages debug and release APKs with SHA-256 checksums. See [`android/README.md`](android/README.md) and [`docs/actions-build.md`](docs/actions-build.md).

## Architecture

```text
APK
 │
 ├── ZIP / manifest / resources / DEX inspection
 │
 ├── Rust HLE core
 │   ├── guarded Dalvik interpreter
 │   ├── Android 1.x–2.x framework shims
 │   ├── libGDX compatibility layer
 │   └── Legacy GLES command stream + GLES 2.0 presentation + software framebuffer
 │
 └── Android shell + JNI
     ├── APK library and storage
     ├── compatibility log
     ├── GLES surface
     └── touch bridge
```

## Roadmap

1. Expand Dalvik 035 opcode coverage and improve method dispatch, class initialization, exceptions, and arrays.
2. Complete the Android 1.x–2.x framework surface needed by real applications.
3. Harden libGDX constructors, overloads, texture filtering/wrapping, atlas metadata, transforms, and resource lifetime.
4. Improve fixed-function GLES correctness, clipping, blending factors, depth behavior, and indexed rendering.
5. Connect Android touch/key events and audio output to emulated applications instead of placeholders.
6. Add reproducible compatibility fixtures, frame captures, traces, and per-application reports.

## Project principles

- **HLE, not a full-device emulator:** implement the APIs applications use rather than reproducing an entire phone.
- **Clean-room boundaries:** do not copy proprietary Android framework code or ship copyrighted application assets.
- **Measured compatibility:** back supported behavior with tests, traces, or reproducible application results.
- **Honest diagnostics:** expose unsupported methods and incomplete rendering paths in logs.

Only test APKs you own or are legally allowed to analyze. DonutHLE does not ship game files and does not bypass licensing, DRM, signature checks, or online services.

## Relationship and license

DonutHLE is an independent prototype. It is not affiliated with Google, Android, touchHLE, PocketHLE, libGDX, or their authors. Android is a trademark of Google LLC. libGDX is an open-source framework maintained by its contributors.

MIT License. See [`LICENSE`](LICENSE).

## GitHub Actions and releases

- Pushes and pull requests run repository checks; changes targeting `main` also run Linux, Windows, and Android builds.
- Android debug/release APKs, Linux x86_64, and Windows x86_64 artifacts are uploaded by the workflow.
- Each packaged artifact includes a checksum file.
- A tag such as `v0.1.2` publishes a GitHub release automatically.
- Manual workflow runs can publish a release when `publish_release` is enabled.

---

The launcher icon and this README image use the same DonutHLE artwork.
