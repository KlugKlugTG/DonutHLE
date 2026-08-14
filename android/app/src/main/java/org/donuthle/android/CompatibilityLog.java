package org.donuthle.android;

import android.content.Context;
import java.io.File;
import java.io.IOException;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

final class CompatibilityLog {
    private static final String[] FRAMEWORK_MARKERS = {
            "Landroid/app/", "Landroid/content/", "Landroid/view/", "Landroid/graphics/",
            "Landroid/opengl/", "Landroid/media/", "Landroid/os/", "Ljava/net/"
    };
    private CompatibilityLog() {}

    static void recordStartup(Context context) {
        StorageLayout.appendLog(context, "COMPATIBILITY_SCAN: startup");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: Dalvik 035 instruction interpreter");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: Android binary XML (AXML) manifest/resource decoder");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: resources.arsc/resource lookup");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: Android framework HLE (Activity, Context, Looper, Binder, services)");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: GLES 1.x graphics backend");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: audio mixer/backend");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: touch, keyboard and gamepad input bridge");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: JNI/native library loading");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: actual game launch/session window");
    }

    static void inspectApk(Context context, File apk) {
        StorageLayout.appendLog(context, "COMPATIBILITY_SCAN: " + apk.getName());
        try (ZipFile zip = new ZipFile(apk)) {
            ZipEntry manifest = zip.getEntry("AndroidManifest.xml");
            if (manifest == null) StorageLayout.appendLog(context, "UNIMPLEMENTED: APK has no AndroidManifest.xml");
            else StorageLayout.appendLog(context, "PRESENT: AndroidManifest.xml (AXML decoding still unimplemented)");
            ZipEntry dex = zip.getEntry("classes.dex");
            if (dex == null) {
                StorageLayout.appendLog(context, "UNIMPLEMENTED: Dalvik classes.dex is missing");
            } else {
                StorageLayout.appendLog(context, "PRESENT: Dalvik classes.dex, " + dex.getSize() + " bytes");
                StorageLayout.appendLog(context, "UNIMPLEMENTED: Dalvik bytecode execution; classes.dex is only inspected");
                StringBuilder sample = new StringBuilder();
                try (java.io.InputStream input = zip.getInputStream(dex)) {
                    byte[] bytes = new byte[1024 * 1024];
                    int count = input.read(bytes);
                    if (count > 0) {
                        String text = new String(bytes, 0, count, java.nio.charset.StandardCharsets.ISO_8859_1);
                        for (String marker : FRAMEWORK_MARKERS) if (text.contains(marker)) sample.append(marker).append(" ");
                    }
                }
                if (sample.length() > 0) StorageLayout.appendLog(context, "UNIMPLEMENTED: game references framework namespaces: " + sample);
            }
            boolean nativeCode = false;
            boolean assets = false;
            for (java.util.Enumeration<? extends ZipEntry> entries = zip.entries(); entries.hasMoreElements();) {
                String name = entries.nextElement().getName();
                nativeCode |= name.startsWith("lib/") && name.endsWith(".so");
                assets |= name.startsWith("assets/");
            }
            if (assets) StorageLayout.appendLog(context, "PRESENT: APK assets");
            if (nativeCode) StorageLayout.appendLog(context, "UNIMPLEMENTED: native JNI libraries and ABI bridge");
            StorageLayout.appendLog(context, "UNIMPLEMENTED: this APK was not launched; compatibility scan only");
        } catch (IOException error) {
            StorageLayout.appendLog(context, "ERROR: APK scan failed: " + error.getMessage());
        }
    }

    static String statusText() {
        return "UNIMPLEMENTED entries are written during startup and every APK compatibility scan.";
    }
}
