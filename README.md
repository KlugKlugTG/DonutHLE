# DonutHLE

**DonutHLE** is an experimental high-level emulator (HLE) prototype for the Android 1.6 **Donut** application platform.

The goal is similar in spirit to touchHLE and PocketHLE: run selected historical applications through a small, host-native compatibility layer instead of emulating an entire phone SoC and Linux device. The first milestone is deliberately narrow: inspect APKs, parse the Android 1.6 application model, and build a testable runtime foundation before attempting broad game compatibility.

> This is a research prototype, not a working Android 1.6 emulator yet. It does not currently launch APKs or run games.

## Current prototype

The Rust crate currently provides:

- APK/ZIP inspection with safety limits and deterministic file listing.
- DEX header validation for Android's Dalvik bytecode format.
- An explicit Android 1.6 target profile: API level 4, Dalvik VM, ARMv5TE-era app assumptions, and a 320×480 default virtual screen.
- A staged runtime object that reports which subsystems are ready and returns clear errors for unimplemented launch phases.
- A clean CLI: `inspect`, `validate`, and `run`.
- Unit tests and GitHub Actions CI.

## Build

Install the current stable Rust toolchain, then:

```sh
cargo test
cargo run -- --help
```

## Usage

Inspect an APK without executing it:

```sh
cargo run -- inspect path/to/game.apk
```

Validate the archive, manifest entry, and `classes.dex` header:

```sh
cargo run -- validate path/to/game.apk
```

The launch command is intentionally explicit about the current boundary:

```sh
cargo run -- run path/to/game.apk
```

It validates the package and reports the next missing runtime milestone rather than pretending that compatibility exists.

## Roadmap

1. Decode binary `AndroidManifest.xml` and resolve the launcher activity.
2. Implement a small Dalvik interpreter for the API-4 instruction set.
3. Add a Java/Android framework shim: `Activity`, `View`, input, resources, lifecycle, and `Canvas`.
4. Add a software framebuffer and host window/input backend.
5. Add audio, filesystem, timers, and a minimal package manager.
6. Build compatibility fixtures and test simple original APKs before targeting commercial games.
7. Add optional native ARM code support only where a game demonstrably requires it.

The repository will not contain proprietary APKs, Android system images, SDK archives, signing keys, or copyrighted game assets. Use applications you own or are legally allowed to test.

## Name and relationship to other projects

DonutHLE is an independent prototype. It is not affiliated with Google, Android, touchHLE, PocketHLE, or the authors of those projects.

## License

MIT. See [`LICENSE`](LICENSE).
