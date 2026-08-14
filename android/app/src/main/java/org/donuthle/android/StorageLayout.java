package org.donuthle.android;

import android.content.Context;
import android.database.Cursor;
import android.net.Uri;
import android.provider.OpenableColumns;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.text.SimpleDateFormat;
import java.util.Arrays;
import java.util.Date;
import java.util.Locale;

final class StorageLayout {
    static final String ROOT_NAME = "DonutHLE";
    static final String APPS_NAME = "DonutHLE_apps";
    static final String SANDBOX_NAME = "DonutHLE_sandbox";
    static final String LOGS_NAME = "DonutHLE_logs";
    static final String OPTIONS_NAME = "DonutHLE_options.txt";
    static final String LOG_NAME = "DonutHLE_log.txt";

    private StorageLayout() {}

    static File root(Context context) {
        File base = context.getExternalFilesDir(null);
        File root = new File(base == null ? context.getFilesDir() : base, ROOT_NAME);
        root.mkdirs();
        return root;
    }

    static File publicRoot(Context context) {
        File base = context.getExternalFilesDir(null);
        File root = new File(base == null ? context.getFilesDir() : base, ROOT_NAME);
        root.mkdirs();
        return root;
    }

    static File apps(Context context) { return new File(root(context), APPS_NAME); }
    static File sandbox(Context context) { return new File(root(context), SANDBOX_NAME); }
    static File logs(Context context) { return new File(root(context), LOGS_NAME); }
    static File logFile(Context context) { return new File(root(context), LOG_NAME); }
    static File options(Context context) { return new File(root(context), OPTIONS_NAME); }

    static void ensure(Context context) {
        root(context).mkdirs();
        apps(context).mkdirs();
        sandbox(context).mkdirs();
        logs(context).mkdirs();
        if (!options(context).exists()) writeText(options(context), "# DonutHLE options\n# Android 1.6 / API level 4\n# Put one option per line.\n");
        File readme = new File(root(context), "README.txt");
        if (!readme.exists()) writeText(readme, "DonutHLE files\n\nDonutHLE_apps: place legal APK files here.\nDonutHLE_sandbox: per-game writable data.\nDonutHLE_log.txt: readable emulator log.\nDonutHLE_options.txt: emulator options.\n");
    }

    static File[] apks(Context context) {
        ensure(context);
        File[] files = apps(context).listFiles((file, name) -> name.toLowerCase(Locale.US).endsWith(".apk"));
        if (files == null) return new File[0];
        Arrays.sort(files, (left, right) -> left.getName().compareToIgnoreCase(right.getName()));
        return files;
    }

    static File copyApk(Context context, Uri source) {
        ensure(context);
        String name = "game.apk";
        try (Cursor cursor = context.getContentResolver().query(source, null, null, null, null)) {
            if (cursor != null) {
                int index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (cursor.moveToFirst() && index >= 0) name = cursor.getString(index);
            }
        } catch (Exception ignored) {
        }
        if (!name.toLowerCase(Locale.US).endsWith(".apk")) name += ".apk";
        name = name.replaceAll("[^A-Za-z0-9._-]", "_");
        File destination = new File(apps(context), name);
        int suffix = 2;
        while (destination.exists()) {
            destination = new File(apps(context), name.replace(".apk", "-" + suffix++ + ".apk"));
        }
        try (InputStream input = context.getContentResolver().openInputStream(source);
             FileOutputStream output = new FileOutputStream(destination)) {
            if (input == null) return null;
            byte[] buffer = new byte[8192];
            int count;
            while ((count = input.read(buffer)) != -1) output.write(buffer, 0, count);
            return destination;
        } catch (IOException error) {
            return null;
        }
    }

    static void appendLog(Context context, String message) {
        ensure(context);
        try (FileOutputStream stream = new FileOutputStream(logFile(context), true)) {
            String timestamp = new SimpleDateFormat("yyyy-MM-dd HH:mm:ss.SSS", Locale.US).format(new Date());
            stream.write(("[" + timestamp + "] " + message + "\n").getBytes(StandardCharsets.UTF_8));
        } catch (IOException ignored) {
        }
    }

    static String readLog(Context context) {
        return readText(logFile(context));
    }

    static String readText(File file) {
        if (!file.exists()) return "";
        try (FileInputStream input = new FileInputStream(file); ByteArrayOutputStream output = new ByteArrayOutputStream()) {
            byte[] buffer = new byte[4096];
            int count;
            while ((count = input.read(buffer)) != -1) output.write(buffer, 0, count);
            return new String(output.toByteArray(), StandardCharsets.UTF_8);
        } catch (IOException error) {
            return "READ ERROR: " + error.getMessage();
        }
    }

    static void writeText(File file, String value) {
        try (FileOutputStream stream = new FileOutputStream(file)) {
            stream.write(value.getBytes(StandardCharsets.UTF_8));
        } catch (IOException ignored) {
        }
    }
}
