package org.donuthle.android;

import android.content.Context;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.provider.DocumentsContract;
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
    static final String LOG_NAME = "DonutHLE_log.txt";
    static final String OPTIONS_NAME = "DonutHLE_options.txt";
    private static final String PREFS = "donuthle_storage";
    private static final String TREE_URI = "apps_tree_uri";

    private StorageLayout() {}

    static File root(Context context) {
        File base = context.getExternalFilesDir(null);
        File root = new File(base == null ? context.getFilesDir() : base, ROOT_NAME);
        root.mkdirs();
        return root;
    }

    static File publicRoot(Context context) { return root(context); }
    static File apps(Context context) { return new File(root(context), APPS_NAME); }
    static File sandbox(Context context) { return new File(root(context), "DonutHLE_sandbox"); }
    static File logFile(Context context) { return new File(root(context), LOG_NAME); }
    static File options(Context context) { return new File(root(context), OPTIONS_NAME); }

    static void ensure(Context context) {
        root(context).mkdirs();
        apps(context).mkdirs();
        sandbox(context).mkdirs();
        if (!options(context).exists()) writeText(options(context), "# DonutHLE options\n# Android 1.x-2.x / API levels 1-8; default profile API level 8\n# Put one option per line.\n");
        File readme = new File(root(context), "README.txt");
        if (!readme.exists()) writeText(readme, "DonutHLE files\n\nDonutHLE_apps: APK library.\nDonutHLE_sandbox: per-game writable data.\nDonutHLE_log.txt: UTF-8 emulator log.\nDonutHLE_options.txt: emulator options.\n");
    }

    static int importFromDefaultFolder(Context context) {
        File publicApps = new File(android.os.Environment.getExternalStorageDirectory(), APPS_NAME);
        if (!publicApps.isDirectory() || publicApps.equals(apps(context))) return 0;
        int imported = 0;
        File[] files = publicApps.listFiles((file, name) -> name.toLowerCase(Locale.US).endsWith(".apk"));
        if (files == null) return 0;
        for (File file : files) {
            try {
                File target = new File(apps(context), file.getName());
                if (!target.exists() || target.length() != file.length()) {
                    try (InputStream input = new FileInputStream(file); FileOutputStream output = new FileOutputStream(target)) {
                        byte[] buffer = new byte[8192];
                        int count;
                        while ((count = input.read(buffer)) != -1) output.write(buffer, 0, count);
                    }
                    imported++;
                }
            } catch (IOException error) {
                appendLog(context, "APK_IMPORT_FAILED: " + file.getName() + ": " + error.getMessage());
            }
        }
        if (imported > 0) appendLog(context, "APK_IMPORTED_FROM_DEFAULT_FOLDER: " + imported);
        return imported;
    }

    static File[] apks(Context context) {
        ensure(context);
        File[] files = apps(context).listFiles((file, name) -> name.toLowerCase(Locale.US).endsWith(".apk"));
        if (files == null) return new File[0];
        Arrays.sort(files, (left, right) -> left.getName().compareToIgnoreCase(right.getName()));
        return files;
    }

    static File copyApk(Context context, Uri source) throws IOException {
        ensure(context);
        String name = displayName(context, source);
        if (!name.toLowerCase(Locale.US).endsWith(".apk")) name += ".apk";
        name = name.replaceAll("[^A-Za-z0-9._-]", "_");
        File destination = new File(apps(context), name);
        int suffix = 2;
        while (destination.exists()) destination = new File(apps(context), name.replace(".apk", "-" + suffix++ + ".apk"));
        try (InputStream input = context.getContentResolver().openInputStream(source); FileOutputStream output = new FileOutputStream(destination)) {
            if (input == null) throw new IOException("provider returned no stream");
            byte[] buffer = new byte[8192];
            int count;
            while ((count = input.read(buffer)) != -1) output.write(buffer, 0, count);
        }
        return destination;
    }

    static int importFromTree(Context context, Uri tree) {
        ensure(context);
        int imported = 0;
        try {
            String treeDocument = DocumentsContract.getTreeDocumentId(tree);
            Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, treeDocument);
            String[] projection = {DocumentsContract.Document.COLUMN_DOCUMENT_ID, DocumentsContract.Document.COLUMN_DISPLAY_NAME, DocumentsContract.Document.COLUMN_MIME_TYPE};
            try (Cursor cursor = context.getContentResolver().query(children, projection, null, null, null)) {
                if (cursor == null) return 0;
                int idIndex = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_DOCUMENT_ID);
                int nameIndex = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_DISPLAY_NAME);
                while (cursor.moveToNext()) {
                    String name = nameIndex >= 0 ? cursor.getString(nameIndex) : "";
                    if (!name.toLowerCase(Locale.US).endsWith(".apk")) continue;
                    String documentId = cursor.getString(idIndex);
                    Uri document = DocumentsContract.buildDocumentUriUsingTree(tree, documentId);
                    copyApk(context, document);
                    imported++;
                }
            }
        } catch (Exception error) {
            appendLog(context, "UNIMPLEMENTED/ERROR: external DonutHLE_apps scan failed: " + error.getMessage());
        }
        if (imported > 0) appendLog(context, "Imported " + imported + " APK(s) from selected DonutHLE_apps folder");
        return imported;
    }

    static Uri savedTree(Context context) {
        String value = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(TREE_URI, null);
        return value == null ? null : Uri.parse(value);
    }

    static void saveTree(Context context, Uri tree) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString(TREE_URI, tree.toString()).apply();
    }

    private static String displayName(Context context, Uri source) {
        try (Cursor cursor = context.getContentResolver().query(source, null, null, null, null)) {
            if (cursor != null) {
                int index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (cursor.moveToFirst() && index >= 0) return cursor.getString(index);
            }
        } catch (Exception ignored) {}
        return "game.apk";
    }

    static void appendLog(Context context, String message) {
        ensure(context);
        try (FileOutputStream stream = new FileOutputStream(logFile(context), true)) {
            String timestamp = new SimpleDateFormat("yyyy-MM-dd HH:mm:ss.SSS", Locale.US).format(new Date());
            stream.write(("[" + timestamp + "] " + message + "\n").getBytes(StandardCharsets.UTF_8));
        } catch (IOException ignored) {}
    }

    static String readLog(Context context) { return readText(logFile(context)); }

    static String readText(File file) {
        if (!file.exists()) return "";
        try (FileInputStream input = new FileInputStream(file); ByteArrayOutputStream output = new ByteArrayOutputStream()) {
            byte[] buffer = new byte[8192];
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
        } catch (IOException ignored) {}
    }
}
