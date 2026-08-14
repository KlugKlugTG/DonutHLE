# DonutHLE Android build

This directory is the Android Studio/Gradle shell for DonutHLE. It builds a native Android application for:

- `arm64-v8a`
- `armeabi-v7a`
- `x86_64`

The current Android app is a native smoke-test shell. It loads the `donuthle` JNI library and displays the Android 1.6 target profile. The Rust core is already configured as a `cdylib`; the next integration step is to connect the Rust runtime to the JNI bridge.

## Build on a normal development machine

Install Android Studio with SDK Platform 35, Build Tools, NDK 25.2.9519653 or newer, CMake 3.22.1 or newer, Java 17, and Rust.

From this directory:

```sh
./gradlew assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n org.donuthle.android/.MainActivity
```

Android Studio can open the `android/` directory directly.

The repository's GitHub Actions workflow builds this project automatically on pushes, pull requests, and manual runs. It uploads debug/release APK artifacts. To create a release, push a tag such as `v0.1.1` or start the `Build DonutHLE` workflow with `publish_release` enabled.
