package org.donuthle.android;

import android.content.Context;

import java.io.File;
import java.io.IOException;

final class CompatibilityLog {
    private CompatibilityLog() {}

    static void onStartup(Context context) {
        missing(context, "Dalvik 035 interpreter and method execution");
        missing(context, "Android framework Activity, Context, Binder, and lifecycle shims");
        missing(context, "Android binary XML resource decoding");
        missing(context, "resources.arsc and compiled resource lookup");
        missing(context, "GLES 1.x graphics backend");
        missing(context, "SurfaceView/framebuffer presentation");
        missing(context, "audio mixer, SoundPool, MediaPlayer, and AudioTrack backends");
        missing(context, "touch, keyboard, sensor, and gamepad input bridge");
        missing(context, "SQLite and SharedPreferences compatibility layer");
        missing(context, "Android services, Looper, and Handler queues");
        missing(context, "JNI/native library loading");
        missing(context, "APK activity launch and process isolation");
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
        missing(context, "launch requested for " + apk.getName() + ": compatibility execution is not available");
    }

    private static void missing(Context context, String feature) {
        StorageLayout.appendLog(context, "UNIMPLEMENTED: " + feature);
    }
}
