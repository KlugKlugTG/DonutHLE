package org.donuthle.android;

import java.io.File;
import java.util.Locale;

final class CompatibilityLog {
    private CompatibilityLog() {}

    static void recordStartup(android.content.Context context) {
        StorageLayout.appendLog(context, "COMPATIBILITY: Dalvik 035 interpreter = NOT IMPLEMENTED");
        StorageLayout.appendLog(context, "COMPATIBILITY: binary AndroidManifest.xml / AXML decoder = NOT IMPLEMENTED");
        StorageLayout.appendLog(context, "COMPATIBILITY: Android framework class library and Activity lifecycle = NOT IMPLEMENTED");
        StorageLayout.appendLog(context, "COMPATIBILITY: resource.arsc decoder = NOT IMPLEMENTED");
        StorageLayout.appendLog(context, "COMPATIBILITY: GLES 1.x renderer = NOT IMPLEMENTED");
        StorageLayout.appendLog(context, "COMPATIBILITY: audio mixer and MediaPlayer = NOT IMPLEMENTED");
        StorageLayout.appendLog(context, "COMPATIBILITY: touch, keyboard and gamepad input bridge = NOT IMPLEMENTED");
        StorageLayout.appendLog(context, "COMPATIBILITY: Android 1.6 services (PackageManager, WindowManager, SensorManager) = NOT IMPLEMENTED");
        StorageLayout.appendLog(context, "COMPATIBILITY: APK launcher = NOT IMPLEMENTED");
    }

    static void inspectApk(android.content.Context context, File apk) {
        StorageLayout.appendLog(context, "APK REQUEST: " + apk.getName());
        StorageLayout.appendLog(context, "APK REQUEST: package manifest inspection requested");
        StorageLayout.appendLog(context, "APK REQUEST: classes.dex inspection requested");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: APK launch cannot execute Dalvik bytecode yet");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: no Android Activity can be created from this APK yet");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: game graphics/audio/input calls will be reported when the runtime is connected");
    }

    static String statusText() {
        return String.format(Locale.US,
                "IMPLEMENTATION STATUS\n\n"
                        + "[READY] APK library and sandbox storage\n"
                        + "[READY] UTF-8 compatibility log\n"
                        + "[PARTIAL] DEX 035 header/table inspection\n"
                        + "[TODO] Dalvik interpreter\n"
                        + "[TODO] AXML manifest decoder\n"
                        + "[TODO] Android 1.6 framework shims\n"
                        + "[TODO] GLES 1.x / audio / input backends\n"
                        + "[TODO] Activity launch and rendering\n");
    }
}
