# DonutHLE Android app

DonutHLE is an experimental **Android 1.x high-level emulator**. This Android project is the modern host shell: it loads the Rust core through JNI, presents the emulated framebuffer, forwards touch input, and provides the APK library and compatibility log.

## Use the app

1. Build or download the APK.
2. Install it on an Android 6.0+ device or emulator.
3. Open **Game Library**.
4. Tap **Import APK**, select an APK you own, and then tap **Run / Inspect**.
5. Open **Emulator Log** to see what the runtime implemented and which APIs or paths still need work.

The home screen keeps the common actions visible, while Options contains portable storage information. The launcher icon is the DonutHLE donut/Android artwork used in the repository README.

## Build on a normal development machine

Install Android Studio with SDK Platform 35, Build Tools 35.0.0, NDK 27.0.12077973, CMake 3.22.1, Java 17, and Rust.

From this directory:

```sh
gradle assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n org.donuthle.android/.MainActivity
```

Android Studio can open the `android/` directory directly.

The Android build supports:

- `arm64-v8a`
- `armeabi-v7a`
- `x86_64`

GitHub Actions cross-compiles the Rust static library for each ABI before Gradle packages the application. If a local build does not include a prebuilt Rust library, the shell remains usable as a UI-only fallback and reports that state in **About DonutHLE** and the log.

## App storage

The app creates a `DonutHLE/` directory below its external-files directory:

- `DonutHLE_apps/` — imported APK library
- `DonutHLE_sandbox/` — writable per-game data
- `DonutHLE_log.txt` — UTF-8 compatibility log
- `DonutHLE_options.txt` — options notes

Use **Options → Open folder picker** to choose or reconnect a folder through Android's document picker.

## Release artifacts

The repository's `Build DonutHLE` workflow runs on pushes, pull requests, and manual runs. It uploads debug/release APK artifacts and checksums. A tag such as `v0.1.2` publishes a GitHub release automatically.
