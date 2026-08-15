# DonutHLE Android build

This directory is the Android Studio/Gradle shell for DonutHLE. It builds a native Android application for:

- `arm64-v8a`
- `armeabi-v7a`
- `x86_64`

The Android app loads the Rust `donuthle` core through a JNI bridge. GitHub Actions cross-compiles the Rust static library for every supported ABI before Gradle packages the APK. A local Android build without a prebuilt Rust library remains a UI-only fallback and reports that state in the About screen.

## Build on a normal development machine

Install Android Studio with SDK Platform 35, Build Tools 35.0.0, NDK 27.0.12077973, CMake 3.22.1, Java 17, and Rust.

From this directory:

```sh
./gradlew assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n org.donuthle.android/.MainActivity
```

Android Studio can open the `android/` directory directly.

The repository's GitHub Actions workflow builds this project automatically on pushes, pull requests, and manual runs. It uploads debug/release APK artifacts. To create a release, push a tag such as `v0.1.1` or start the `Build DonutHLE` workflow with `publish_release` enabled.
