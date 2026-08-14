package org.donuthle.android;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Set;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

final class ApkCompatibility {
    private static final int MAX_DEX_BYTES = 64 * 1024 * 1024;

    private ApkCompatibility() {}

    static Report inspect(File apk) throws IOException {
        Report report = new Report(apk.getName(), apk.length());
        try (ZipFile zip = new ZipFile(apk)) {
            ZipEntry manifest = zip.getEntry("AndroidManifest.xml");
            if (manifest == null) {
                report.unimplemented.add("APK manifest is missing");
            } else {
                report.present.add("AndroidManifest.xml");
                report.unimplemented.add("AXML manifest parser: using Android package metadata as a temporary bridge");
            }

            ZipEntry dexEntry = zip.getEntry("classes.dex");
            if (dexEntry == null) {
                report.unimplemented.add("Dalvik classes.dex is missing");
            } else {
                byte[] dex = readEntry(zip, dexEntry);
                try {
                    DexInfo info = DexInfo.parse(dex);
                    report.dex = info;
                    report.present.add("Dalvik DEX " + info.version + " (" + info.fileSize + " bytes)");
                    report.unimplemented.add("Dalvik bytecode interpreter: DEX is parsed, instructions are not executed yet");
                    findFrameworkRequests(info.strings, report);
                } catch (IOException error) {
                    report.unimplemented.add("Dalvik DEX parser rejected classes.dex: " + error.getMessage());
                }
            }

            boolean nativeCode = false;
            boolean assets = false;
            for (java.util.Enumeration<? extends ZipEntry> entries = zip.entries(); entries.hasMoreElements();) {
                String name = entries.nextElement().getName();
                nativeCode |= name.startsWith("lib/") && name.endsWith(".so");
                assets |= name.startsWith("assets/");
            }
            if (nativeCode) report.unimplemented.add("JNI/native libraries: host ABI loading is not implemented");
            if (assets) report.present.add("APK assets");
            report.unimplemented.add("Android framework HLE: Activity, Context, Looper, Binder and system services are incomplete");
            report.unimplemented.add("Graphics/audio/input backends: not implemented for the game session");
        }
        return report;
    }

    private static void findFrameworkRequests(List<String> strings, Report report) {
        String[][] markers = {
                {"Landroid/app/", "Android Activity/Application framework classes"},
                {"Landroid/content/", "Android Context/content providers/intents"},
                {"Landroid/view/", "Android View/window/input classes"},
                {"Landroid/graphics/", "Android Canvas/graphics classes"},
                {"Landroid/opengl/", "OpenGL ES 1.x bridge"},
                {"Landroid/media/", "Android audio/media classes"},
                {"Ljava/net/", "Java networking and sockets"},
                {"Landroid/os/", "Android threads, files and Binder services"}
        };
        for (String[] marker : markers) {
            for (String value : strings) {
                if (value.contains(marker[0])) {
                    report.unimplemented.add(marker[1] + ": referenced by the game");
                    break;
                }
            }
        }
    }

    private static byte[] readEntry(ZipFile zip, ZipEntry entry) throws IOException {
        if (entry.getSize() > MAX_DEX_BYTES) throw new IOException("DEX exceeds diagnostic limit");
        try (InputStream input = zip.getInputStream(entry); ByteArrayOutputStream output = new ByteArrayOutputStream()) {
            byte[] buffer = new byte[8192];
            int count;
            while ((count = input.read(buffer)) != -1) {
                output.write(buffer, 0, count);
                if (output.size() > MAX_DEX_BYTES) throw new IOException("DEX exceeds diagnostic limit");
            }
            return output.toByteArray();
        }
    }

    static final class Report {
        final String fileName;
        final long fileSize;
        final Set<String> present = new HashSet<>();
        final Set<String> unimplemented = new HashSet<>();
        DexInfo dex;
        String packageName = "unknown.package";

        Report(String fileName, long fileSize) {
            this.fileName = fileName;
            this.fileSize = fileSize;
        }

        String toLog() {
            StringBuilder out = new StringBuilder();
            out.append("== APK COMPATIBILITY REPORT ==\n");
            out.append("APK: ").append(fileName).append(" ( ").append(fileSize).append(" bytes)\n");
            out.append("package: ").append(packageName).append("\n");
            for (String item : present) out.append("IMPLEMENTED/PRESENT: ").append(item).append("\n");
            for (String item : unimplemented) out.append("UNIMPLEMENTED: ").append(item).append("\n");
            return out.toString();
        }

        String displayText() {
            StringBuilder out = new StringBuilder(toLog());
            if (dex != null) {
                out.append("\nDEX TABLES\n");
                out.append("strings: ").append(dex.stringCount).append("\n");
                out.append("types: ").append(dex.typeCount).append("\n");
                out.append("methods: ").append(dex.methodCount).append("\n");
                out.append("classes: ").append(dex.classCount).append("\n");
            }
            return out.toString();
        }
    }

    static final class DexInfo {
        final String version;
        final int fileSize;
        final int stringCount;
        final int typeCount;
        final int methodCount;
        final int classCount;
        final List<String> strings;

        private DexInfo(String version, int fileSize, int stringCount, int typeCount, int methodCount, int classCount, List<String> strings) {
            this.version = version;
            this.fileSize = fileSize;
            this.stringCount = stringCount;
            this.typeCount = typeCount;
            this.methodCount = methodCount;
            this.classCount = classCount;
            this.strings = strings;
        }

        static DexInfo parse(byte[] bytes) throws IOException {
            if (bytes.length < 112 || bytes[0] != 'd' || bytes[1] != 'e' || bytes[2] != 'x' || bytes[3] != '\n' || bytes[4] != '0' || bytes[5] != '3' || bytes[6] != '5' || bytes[7] != 0) throw new IOException("expected Dalvik DEX 035");
            int fileSize = u32(bytes, 0x20);
            if (fileSize > bytes.length || u32(bytes, 0x70) != 112 || u32(bytes, 0x28) != 0x12345678) throw new IOException("invalid DEX header");
            int stringCount = u32(bytes, 0x38);
            int stringOffset = u32(bytes, 0x3c);
            int typeCount = u32(bytes, 0x40);
            int methodCount = u32(bytes, 0x58);
            int classCount = u32(bytes, 0x60);
            List<String> strings = new ArrayList<>();
            int limit = Math.min(stringCount, 100000);
            for (int i = 0; i < limit; i++) {
                int offset = u32(bytes, stringOffset + i * 4);
                strings.add(readString(bytes, offset));
            }
            return new DexInfo("035", fileSize, stringCount, typeCount, methodCount, classCount, strings);
        }

        private static String readString(byte[] bytes, int offset) throws IOException {
            if (offset < 0 || offset >= bytes.length) throw new IOException("string offset outside DEX");
            int cursor = offset;
            readUleb(bytes, cursor);
            while (cursor < bytes.length && bytes[cursor] != 0) cursor++;
            if (cursor >= bytes.length) throw new IOException("unterminated DEX string");
            int start = offset;
            int lengthBytes = 0;
            while (start < bytes.length && (bytes[start++] & 0x80) != 0) lengthBytes++;
            lengthBytes++;
            int end = offset + lengthBytes;
            while (end < bytes.length && bytes[end] != 0) end++;
            return new String(bytes, end == bytes.length ? offset + lengthBytes : offset + lengthBytes, Math.max(0, end - (offset + lengthBytes)), StandardCharsets.UTF_8);
        }

        private static int readUleb(byte[] bytes, int offset) throws IOException {
            int result = 0;
            int shift = 0;
            for (int i = 0; i < 5 && offset + i < bytes.length; i++) {
                int value = bytes[offset + i] & 0xff;
                result |= (value & 0x7f) << shift;
                if ((value & 0x80) == 0) return result;
                shift += 7;
            }
            throw new IOException("invalid ULEB128");
        }

        private static int u32(byte[] bytes, int offset) throws IOException {
            if (offset < 0 || offset + 4 > bytes.length) throw new IOException("DEX table outside file");
            return (bytes[offset] & 0xff) | ((bytes[offset + 1] & 0xff) << 8) | ((bytes[offset + 2] & 0xff) << 16) | ((bytes[offset + 3] & 0xff) << 24);
        }
    }
}
