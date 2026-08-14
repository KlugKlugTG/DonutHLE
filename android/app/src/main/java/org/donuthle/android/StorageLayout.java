package org.donuthle.android;

import android.content.Context;
import android.net.Uri;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.text.SimpleDateFormat;
import java.util.Date;
import java.util.Locale;

public final class StorageLayout {
    private StorageLayout() {}

    public static File root(Context context) {
        return new File(context.getExternalFilesDir(null), "DonutHLE");
    }

    public static File apps(Context context) {
        return new File(root(context), "DonutHLE_apps");
    }

    public static File sandbox(Context context) {
        return new File(root(context), "DonutHLE_sandbox");
    }

    public static File logs(Context context) {
        return new File(root(context), "DonutHLE_logs");
    }

    public static File options(Context context) {
        return new File(root(context), "DonutHLE_options.txt");
    }

    public static File readme(Context context) {
        return new File(root(context), "OPTIONS_HELP.txt");
    }

    public static File logFile(Context context) {
        return new File(logs(context), "DonutHLE_log.txt");
    }

    public static File[] apks(Context context) {
        ensure(context);
        File[] files = apps(context).listFiles((directory, name) -> name.toLowerCase(Locale.US).endsWith(".apk"));
        return files == null ? new File[0] : files;
    }

    public static void ensure(Context context) {
        root(context).mkdirs();
        apps(context).mkdirs();
        sandbox(context).mkdirs();
        logs(context).mkdirs();
        if (!options(context).exists()) write(options(context), "DonutHLE options\n\nAPK library: DonutHLE_apps\nGame data: DonutHLE_sandbox\nLogs: DonutHLE_logs\n");
        if (!readme(context).exists()) write(readme(context), "DonutHLE Android 1.6 HLE\n\nPut APK files in DonutHLE_apps, or use IMPORT APK in the app.\nEach selected game receives a folder in DonutHLE_sandbox.\nLogs are written as UTF-8 to DonutHLE_logs/DonutHLE_log.txt.\n");
        if (!logFile(context).exists()) write(logFile(context), "DonutHLE log\n================\n");
    }

    public static String readLog(Context context) {
        ensure(context);
        try (FileInputStream stream = new FileInputStream(logFile(context))) {
            byte[] bytes = new byte[(int) logFile(context).length()];
            int count = stream.read(bytes);
            return new String(bytes, 0, Math.max(0, count), StandardCharsets.UTF_8);
        } catch (IOException error) {
            return "Unable to read DonutHLE_log.txt\n" + error.getMessage();
        }
    }

    public static File copyApk(Context context, Uri source) {
        ensure(context);
        String name = source.getLastPathSegment();
        if (name == null || name.trim().isEmpty()) name = "imported_game.apk";
        name = name.replaceAll("[^A-Za-z0-9._-]", "_");
        if (!name.toLowerCase(Locale.US).endsWith(".apk")) name += ".apk";
        File destination = uniqueFile(apps(context), name);
        try (InputStream input = context.getContentResolver().openInputStream(source);
             FileOutputStream output = new FileOutputStream(destination)) {
            if (input == null) return null;
            byte[] buffer = new byte[8192];
            int count;
            while ((count = input.read(buffer)) != -1) output.write(buffer, 0, count);
            return destination;
        } catch (IOException error) {
            appendLog(context, "APK import error: " + error.getMessage());
            if (destination.exists()) destination.delete();
            return null;
        }
    }

    public static void appendLog(Context context, String message) {
        ensure(context);
        try (FileOutputStream stream = new FileOutputStream(logFile(context), true)) {
            String stamp = new SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.US).format(new Date());
            stream.write((stamp + "  " + message + "\n").getBytes(StandardCharsets.UTF_8));
        } catch (IOException ignored) {
        }
    }

    private static File uniqueFile(File directory, String name) {
        File result = new File(directory, name);
        if (!result.exists()) return result;
        String base = name.substring(0, name.length() - 4);
        int number = 2;
        do {
            result = new File(directory, base + "_" + number++ + ".apk");
        } while (result.exists());
        return result;
    }

    private static void write(File file, String value) {
        try (FileOutputStream stream = new FileOutputStream(file)) {
            stream.write(value.getBytes(StandardCharsets.UTF_8));
        } catch (IOException ignored) {
        }
    }
}
