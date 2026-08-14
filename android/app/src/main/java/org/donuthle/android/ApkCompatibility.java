package org.donuthle.android;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;
import java.util.Set;
import java.util.TreeSet;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

final class ApkCompatibility {
    private ApkCompatibility() {}

    static Report inspect(File apk) throws IOException {
        Report report = new Report(apk.getName(), apk.length());
        try (ZipFile zip = new ZipFile(apk)) {
            for (java.util.Enumeration<? extends ZipEntry> entries = zip.entries(); entries.hasMoreElements();) {
                ZipEntry entry = entries.nextElement();
                report.present.add(entry.getName());
                if ("classes.dex".equals(entry.getName())) {
                    report.dexBytes = readLimited(zip.getInputStream(entry), 8 * 1024 * 1024);
                }
            }
        }
        report.hasManifest = report.present.contains("AndroidManifest.xml");
        report.hasDex = report.present.contains("classes.dex");
        report.hasResources = report.present.contains("resources.arsc");
        if (!report.hasManifest) report.gaps.add("AndroidManifest.xml / binary AXML decoder");
        if (!report.hasDex) report.gaps.add("Dalvik classes.dex loader and verifier");
        if (!report.hasResources) report.gaps.add("resources.arsc resource table");
        if (report.dexBytes != null) detectRequests(report);
        return report;
    }

    private static void detectRequests(Report report) {
        String dex = new String(report.dexBytes, StandardCharsets.ISO_8859_1);
        String[][] features = {
                {"android/opengl", "GLES 1.x graphics backend"},
                {"javax/microedition/khronos", "OpenGL ES EGL/Khronos bridge"},
                {"android/media/AudioTrack", "AudioTrack PCM mixer"},
                {"android/media/MediaPlayer", "MediaPlayer backend"},
                {"android/media/SoundPool", "SoundPool effects backend"},
                {"android/view/SurfaceView", "SurfaceView and framebuffer bridge"},
                {"android/view/SurfaceHolder", "Surface lifecycle and buffer queue"},
                {"android/database/sqlite", "SQLite compatibility layer"},
                {"SharedPreferences", "SharedPreferences persistence"},
                {"android/os/Looper", "Looper/Handler message queues"},
                {"android/hardware/Sensor", "sensor compatibility layer"},
                {"android/hardware/Camera", "camera compatibility layer"},
                {"android/location", "location services"},
                {"android/bluetooth", "Bluetooth services"},
                {"android/webkit", "WebView compatibility layer"},
                {"android/net/", "Android network services"},
                {"android/view/MotionEvent", "touch and motion input bridge"},
                {"System.loadLibrary", "JNI/native library loader"},
                {"dalvik/system", "Dalvik system APIs"}
        };
        for (String[] feature : features) {
            if (dex.contains(feature[0])) report.requestedGaps.add(feature[1]);
        }
    }

    private static byte[] readLimited(java.io.InputStream input, int limit) throws IOException {
        try (java.io.InputStream stream = input; ByteArrayOutputStream output = new ByteArrayOutputStream()) {
            byte[] buffer = new byte[8192];
            int total = 0;
            int count;
            while ((count = stream.read(buffer)) != -1) {
                total += count;
                if (total > limit) break;
                output.write(buffer, 0, count);
            }
            return output.toByteArray();
        }
    }

    static final class Report {
        final String fileName;
        final long fileSize;
        final Set<String> present = new TreeSet<>();
        final List<String> gaps = new ArrayList<>();
        final List<String> requestedGaps = new ArrayList<>();
        byte[] dexBytes;
        boolean hasManifest;
        boolean hasDex;
        boolean hasResources;

        Report(String fileName, long fileSize) {
            this.fileName = fileName;
            this.fileSize = fileSize;
        }
    }
}
