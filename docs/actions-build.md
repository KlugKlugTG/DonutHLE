# GitHub Actions build

The Build DonutHLE workflow builds the Android 1.x-2.x emulator core, Linux and Windows binaries, and Android APK artifacts. The Android shell presents the shared software framebuffer through a GLES 2.0 context; the desktop runtime uses the legacy GLES compatibility adapter through the shared `HostGles` type.
