package org.donuthle.android;

import android.content.Context;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.text.SimpleDateFormat;
import java.util.Date;
import java.util.Locale;

final class StorageLayout {
    static final String ROOT_NAME = "DonutHLE";
    static final String APPS_NAME = "DonutHLE_apps";
    static final String SANDBOX_NAME = "DonutHLE_sandbox";
    static final String LOGS_NAME = "DonutHLE_logs";
    static final String OPTIONS_NAME = "DonutHLE_options.txt";

    private StorageLayout() {}

    static File root(Context context) {
        return new File(context.getExternalFilesDir(null), ROOT_NAME);
    }

    static File apps(Context context) {
        return new File(root(context), APPS_NAME);
    }

    static File sandbox(Context context) {
        return new File(root(context), SANDBOX_NAME);
    }

    static File logs(Context context) {
        return new File(root(context), LOGS_NAME);
    }

    static File options(Context context) {
        return new File(root(context), OPTIONS_NAME);
    }

    static void ensure(Context context) {
        root(context).mkdirs();
        apps(context).mkdirs();
        sandbox(context).mkdirs();
        logs(context).mkdirs();
        File options = options(context);
        if (!options.exists()) {
            write(options, "# DonutHLE options\n# Android 1.6 / API level 4\n# Put one option per line.\n");
        }
        File readme = new File(root(context), "README.txt");
        if (!readme.exists()) {
            write(readme, "DonutHLE files\n\nDonutHLE_apps: place legal APK files here.\nDonutHLE_sandbox: per-game writable data.\nDonutHLE_logs: emulator logs.\n");
        }
    }

    static void appendLog(Context context, String message) {
        ensure(context);
        File log = new File(logs(context), "donuthle.log");
        try (FileOutputStream stream = new FileOutputStream(log, true)) {
            String timestamp = new SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.US).format(new Date());
            stream.write(("[" + timestamp + "] " + message + "\n").getBytes(StandardCharsets.UTF_8));
        } catch (IOException ignored) {
        }
    }

    private static void write(File file, String value) {
        try (FileOutputStream stream = new FileOutputStream(file)) {
            stream.write(value.getBytes(StandardCharsets.UTF_8));
        } catch (IOException ignored) {
        }
    }
}
