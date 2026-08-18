# DonutHLE

![DonutHLE logo](docs/images/donuthle-logo.png)

**DonutHLE** is an experimental, open-source high-level emulator (HLE) for applications built for Android 1.6 **Donut**. It is written in Rust, with an Android shell and JNI bridge for running the core on modern Android devices.

The project follows the same broad idea as [touchHLE](https://github.com/touchHLE/touchHLE) and [PocketHLE](https://github.com/j92580498-max/PocketHLE): replace the original operating-system APIs with clean-room, host-side implementations instead of emulating an entire phone or device. DonutHLE is an independent project, not a fork of or an affiliated project with either emulator.

> **Status:** research prototype. The runtime can inspect and validate APKs, resolve the launcher, execute a growing subset of Dalvik 035 bytecode, boot the launcher lifecycle, load selected libGDX assets, record and rasterize a GLES 1.x-style command stream, and present the software framebuffer through the Android GLES surface. Compatibility is still application-specific and incomplete.

## What is implemented

### APK and Android 1.6 platform foundation

- ZIP/APK inspection with deterministic file listing and safety limits.
- Android binary XML (`AndroidManifest.xml`) parsing and launcher resolution.
- DEX header validation and parsing for Dalvik 035 files.
- An explicit Android 1.6 target profile: API level 4, Dalvik VM, ARMv5TE-era application assumptions, and a 320×480 default virtual screen.
- Resource-table discovery and partial `resources.arsc` decoding.
- Activity, Context, View, lifecycle, message-queue, input, audio, and resource framework shims.
- A Rust Dalvik interpreter with register bounds checks, method dispatch, call-depth/step limits, and clear unsupported-call diagnostics.
- `ReturnVoid` handling for void methods, including lifecycle paths that previously exposed VM failures.

### Rendering and libGDX path

- Software framebuffer with viewport, scissor, depth-buffer, matrix, blending, and clear-state support.
- A portable GLES 1.x compatibility layer modeled on HyperHLE's GLES1-on-GL2 design: fixed-point array conversion, client-array staging, matrix stacks, OES matrix-palette CPU skinning, and indexed/array draw forwarding into the host GL2-style renderer.
- GLES 1.x-style command recording and rasterization for points, lines, triangle primitives, indexed draws, client arrays, and common state calls.
- PNG and JPEG asset decoding from APK files, including normalized `assets/` paths.
- libGDX-style texture, `TextureRegion`, `TextureAtlas`, and atlas-region lookup support.
- `SpriteBatch`/sprite draw handling that turns loaded image assets into textured framebuffer quads, with a fallback colored quad when an asset cannot be resolved.
- Android framebuffer export through the JNI bridge and presentation of that framebuffer on the host GLES surface.
- A persistent runtime session so the launcher's render listener can be invoked across frames instead of stopping after the first draw.

### Android packaging and development workflow

- Android builds for `arm64-v8a`, `armeabi-v7a`, and `x86_64`.
- A UI-only fallback when the Rust library is not linked into a local Android build.
- GitHub Actions for formatting, tests, Clippy, Linux and Windows builds, Android debug/release APKs, checksums, and optional release publishing.
- The repository intentionally contains no proprietary APKs, Android system images, signing keys, or copyrighted game assets.

## Current state of Slice Ice support

Slice Ice is the first concrete libGDX/Android compatibility target in the repository. DonutHLE now gets beyond the splash/screen-manager stage: it can capture the application listener, execute its creation path, keep the render session alive, resolve assets and atlas regions, route texture-backed sprite draws through the software GLES renderer, and copy the resulting framebuffer to Android.

This does **not** mean that Slice Ice is fully playable yet. Remaining failures can come from unimplemented Dalvik instructions, framework methods, resource formats, libGDX overloads, input, audio, timing, or game-specific assumptions. A successful boot or visible frame is evidence that a compatibility path works; it is not a general compatibility claim.

## Build

Install the stable Rust toolchain. For the Android build, use Android Studio or the command-line SDK with Java 17, SDK Platform 35, Build Tools 35.0.0, NDK 27.0.12077973, CMake 3.22.1, and Gradle 8.13.

### Rust core

```sh
cargo fmt --all -- --check
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

### Android APK

GitHub Actions builds the Rust static library for each ABI before Gradle packages the app. For a local Android build, open `android/` in Android Studio or follow [`android/README.md`](android/README.md). The workflow produces debug and release APK artifacts and records SHA-256 checksums.

## Usage

Inspect an APK without executing it:

```sh
cargo run -- inspect path/to/game.apk
```

Validate the archive, manifest, resources, and `classes.dex` header:

```sh
cargo run -- validate path/to/game.apk
```

Run the current experimental launcher/runtime path:

```sh
cargo run -- run path/to/game.apk
```

Only test APKs you own or are legally allowed to analyze. DonutHLE does not ship game files and does not bypass licensing, DRM, signature checks, or online services.

## Roadmap

1. Expand Dalvik 035 opcode coverage and improve `invoke-direct`, `invoke-virtual`, interface, class initialization, exception, and array behavior.
2. Complete the Android 1.6 framework surface needed by real applications: `Activity`, `View`, `SurfaceView`, resources, `Canvas`, timers, storage, and lifecycle edge cases.
3. Harden the libGDX compatibility layer: more constructor/signature variants, texture filtering/wrapping, atlas rotation/trim metadata, SpriteBatch transforms, and reliable texture lifetime management.
4. Improve GLES 1.x correctness: full fixed-function state, color/texture pointers, blending factors, clipping, depth behavior, and more complete indexed rendering.
5. Connect Android touch/key events and audio output to the emulated application instead of returning placeholders.
6. Add reproducible compatibility fixtures, frame captures, traces, and per-application compatibility reports.
7. Test legally obtained original APKs and document what works, what is missing, and which version/build was tested.
8. Add optional native ARM support only when a target application demonstrably requires it; this is separate from the GLES work.

## Project principles

- **HLE, not a full-device emulator:** implement the APIs applications use rather than reproducing an entire Android phone.
- **Clean-room boundaries:** do not copy proprietary Android framework code or ship copyrighted application assets.
- **Measured compatibility:** every supported behavior should be backed by tests, traces, or a reproducible application result.
- **Honest status reporting:** unsupported methods and incomplete rendering paths should be visible instead of silently pretending to work.

## Thanks and inspiration

Thank you to the developers and contributors of [touchHLE](https://github.com/touchHLE/touchHLE) for demonstrating a practical high-level emulation approach for historical mobile applications, and to [PocketHLE](https://github.com/j92580498-max/PocketHLE) for showing how the same idea can be applied to another legacy mobile platform with a modern Rust host. Their projects are valuable references and inspiration for the design direction of DonutHLE. DonutHLE has its own codebase, scope, and compatibility goals.

## Name and relationship to other projects

DonutHLE is an independent prototype. It is not affiliated with Google, Android, touchHLE, PocketHLE, libGDX, or the authors of those projects. Android is a trademark of Google LLC. libGDX is an open-source framework maintained by its contributors.

## License

MIT. See [`LICENSE`](LICENSE).

## GitHub Actions and releases

The repository includes a `Build DonutHLE` workflow. The desktop runtime always constructs `HostGles`, whose implementation routes GLES 1.x calls through the GLES1-on-GL2 adapter; there is no separate raw-GLES path for the PC build.

- pushes and pull requests run the repository checks; pushes and pull requests targeting `main` also run the full Linux, Windows, and Android build workflow;
- Android debug/release APKs, a Linux x86_64 binary, and a Windows x86_64 `.exe` are uploaded as artifacts;
- each packaged artifact includes a checksum file;
- pushing a tag such as `v0.1.1` publishes a GitHub release automatically;
- a manual workflow run can publish a release when `publish_release` is enabled.

See [`docs/actions-build.md`](docs/actions-build.md) for the workflow details.
