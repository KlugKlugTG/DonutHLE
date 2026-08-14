package org.donuthle.android;

import android.content.Context;

import java.io.File;
import java.io.IOException;

final class CompatibilityLog {
    private CompatibilityLog() {}

    static void onStartup(Context context) {
        StorageLayout.appendLog(context, "IMPLEMENTED: Rust core is linked through the Android native bridge");
        StorageLayout.appendLog(context, "IMPLEMENTED: AXML manifest and resources.arsc decoding");
        StorageLayout.appendLog(context, "IMPLEMENTED: Dalvik DEX 035 VM and Android framework shim");
        StorageLayout.appendLog(context, "IMPLEMENTED: software framebuffer, GLES rasterizer, audio queue, and input queue");
        StorageLayout.appendLog(context, "STATUS: compatibility is app-dependent; unsupported calls are reported during launch");
    }

    static void recordApk(Context context, File apk) {
        try {
            ApkCompatibility.Report report = ApkCompatibility.inspect(apk);
            StorageLayout.appendLog(context, "APK_SCAN: " + report.fileName + " (" + report.fileSize + " bytes)");
            StorageLayout.appendLog(context, "APK_CONTENT: manifest=" + report.hasManifest + " dex=" + report.hasDex + " resources=" + report.hasResources);
            for (String gap : report.gaps) missing(context, "APK requires " + gap);
            for (String gap : report.requestedGaps) missing(context, "APK references " + gap);
            if (report.requestedGaps.isEmpty()) StorageLayout.appendLog(context, "APK_REQUESTS: no known framework references detected in classes.dex");
        } catch (IOException error) {
            missing(context, "APK inspection failed: " + error.getMessage());
        }
    }

    static void recordLaunchAttempt(Context context, File apk) {
        recordApk(context, apk);
        StorageLayout.appendLog(context, "LAUNCH_ATTEMPTED: " + apk.getName() + " through the Rust runtime bridge");
    }

    private static void missing(Context context, String feature) {
        StorageLayout.appendLog(context, "UNIMPLEMENTED: " + feature);
    }
}
