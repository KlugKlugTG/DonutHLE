package org.donuthle.android;

import android.content.Context;
import java.util.LinkedHashSet;
import java.util.Set;

final class CompatibilityLog {
    private CompatibilityLog() {}

    static void startup(Context context) {
        StorageLayout.appendLog(context, "COMPATIBILITY: Android 1.6 / API 4 HLE prototype");
        missing(context, "Dalvik 035 interpreter and bytecode execution");
        missing(context, "Android binary XML manifest/resource decoding");
        missing(context, "resources.arsc resource table and AssetManager parity");
        missing(context, "Android framework Activity/Context/View runtime");
        missing(context, "GLES 1.x graphics backend and framebuffer");
        missing(context, "AudioTrack/MediaPlayer audio backend");
        missing(context, "Touch, keyboard, gamepad, and sensor event bridge");
        missing(context, "SQLite and SharedPreferences compatibility layer");
        missing(context, "Android services, lifecycle, Looper, and Binder shims");
        missing(context, "Network policy and legacy Android permissions");
    }

    static void apkScan(Context context, ApkCompatibility.Report report) {
        StorageLayout.appendLog(context, "APK_SCAN: " + report.fileName + " (" + report.fileSize + " bytes)");
        for (String item : report.missing) missing(context, "APK requested " + item);
        for (String item : report.requested) requested(context, item);
        if (report.manifest != null) StorageLayout.appendLog(context, "MANIFEST: package=" + report.manifest.packageName + ", launcher=" + report.manifest.launcherActivity);
    }

    static void launchAttempt(Context context, String name) {
        StorageLayout.appendLog(context, "LAUNCH_REQUEST: " + name);
        missing(context, "full Dalvik execution pipeline for this game");
        missing(context, "native library loading (lib/*.so) and JNI registration");
        missing(context, "real Android activity launch and rendering");
    }

    static void missing(Context context, String feature) {
        StorageLayout.appendLog(context, "UNIMPLEMENTED: " + feature);
    }

    static void requested(Context context, String feature) {
        StorageLayout.appendLog(context, "GAME_REQUESTED: " + feature);
    }

    static void error(Context context, String message) {
        StorageLayout.appendLog(context, "ERROR: " + message);
    }
}
