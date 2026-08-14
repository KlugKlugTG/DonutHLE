package org.donuthle.android;

import android.content.Context;

import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.HashSet;
import java.util.Set;

final class CompatibilityLog {
    private CompatibilityLog() {}

    static void recordStartup(Context context) {
        StorageLayout.appendLog(context, "SESSION_START: DonutHLE Android 1.6");
        unsupported(context, "Dalvik 035 interpreter and Java method execution");
        unsupported(context, "Android binary XML resource decoding");
        unsupported(context, "resources.arsc and drawable/resource table loading");
        unsupported(context, "GLES 1.x graphics backend");
        unsupported(context, "audio mixer/backend");
        unsupported(context, "Android input/event bridge");
        unsupported(context, "Activity launch and package class loader");
    }

    static void inspectApk(Context context, File apk) {
        StorageLayout.appendLog(context, "APK_SCAN: " + apk.getName());
        Set<String> present = new HashSet<>();
        try (FileInputStream input = new FileInputStream(apk)) {
            byte[] bytes = new byte[(int) Math.min(apk.length(), 8 * 1024 * 1024)];
            int count = input.read(bytes);
            String text = new String(bytes, 0, Math.max(0, count), StandardCharsets.ISO_8859_1);
            if (text.contains("AndroidManifest.xml")) present.add("AndroidManifest.xml");
            if (text.contains("classes.dex")) present.add("classes.dex");
            if (text.contains("resources.arsc")) present.add("resources.arsc");
            if (text.contains("lib/")) present.add("native libraries");
        } catch (IOException error) {
            StorageLayout.appendLog(context, "ERROR[APK_SCAN]: " + error.getMessage());
        }
        for (String item : new String[]{"AndroidManifest.xml", "classes.dex", "resources.arsc", "native libraries"}) {
            if (!present.contains(item)) StorageLayout.appendLog(context, "UNIMPLEMENTED/MISSING: " + item + " inspection");
        }
        StorageLayout.appendLog(context, "REQUESTED: manifest, Dalvik classes, resources, native libraries and launcher activity");
        unsupported(context, "APK installation/manifest parsing and Activity launch");
        unsupported(context, "Dalvik opcode execution and class linking");
        unsupported(context, "Android framework methods requested by the game");
        unsupported(context, "GLES calls requested by the game");
        unsupported(context, "audio APIs requested by the game");
        unsupported(context, "input APIs requested by the game");
        unsupported(context, "JNI/native method bridge requested by the game");
    }

    static void record(Context context, String category, String detail) {
        StorageLayout.appendLog(context, "REQUESTED[" + category + "]: " + detail);
        StorageLayout.appendLog(context, "UNIMPLEMENTED[" + category + "]: compatibility handler is not implemented");
    }

    static void unsupported(Context context, String feature) {
        StorageLayout.appendLog(context, "UNIMPLEMENTED: " + feature);
    }

    static String statusText() {
        return "Compatibility tracing active\nUNIMPLEMENTED entries are written during startup and APK scans.";
    }
}
