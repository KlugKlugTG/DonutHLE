# GitHub Actions build

The Build DonutHLE workflow builds Rust tests, Linux, Windows, and Android APK artifacts. The Windows job produces `DonutHLE-windows-x86_64.exe`; the desktop runtime uses the GLES1-on-GL2 adapter through the shared `HostGles` type.
