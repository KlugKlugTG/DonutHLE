package org.donuthle.android;

import android.content.Context;

import java.io.File;
import java.util.ArrayList;
import java.util.List;

final class CompatibilityLog {
    private static final List<String> lastReport = new ArrayList<>();
    private CompatibilityLog() {}

    static void recordStartup(Context context) {
        StorageLayout.appendLog(context, "COMPATIBILITY: Android 1.6 API level 4 target");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: Dalvik interpreter execution");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: binary AXML manifest decoding");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: Android framework HLE (Activity, Context, Looper, Binder, services)");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: GLES 1.x graphics backend");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: audio mixer/backend");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: game input event bridge");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: JNI/native library execution");
    }

    static void inspectApk(Context context, File apk) {
        lastReport.clear();
        try {
            ApkCompatibility.Report report = ApkCompatibility.inspect(apk);
            lastReport.add(report.displayText());
            for (String item : report.unimplemented) StorageLayout.appendLog(context, "UNIMPLEMENTED: " + item);
            for (String item : report.present) StorageLayout.appendLog(context, "PRESENT: " + item);
            StorageLayout.appendLog(context, "COMPATIBILITY REPORT COMPLETE: " + apk.getName());
        } catch (Exception error) {
            String message = "APK inspection failed: " + error.getMessage();
            lastReport.add(message);
            StorageLayout.appendLog(context, "UNIMPLEMENTED/ERROR: " + message);
        }
    }

    static String statusText() {
        if (lastReport.isEmpty()) return "No APK compatibility scan has been run in this session.";
        StringBuilder result = new StringBuilder("\n== LAST APK SCAN ==\n");
        for (String item : lastReport) result.append(item).append('\n');
        return result.toString();
    }
}