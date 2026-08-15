package org.donuthle.android;

import android.app.Activity;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;
import android.provider.DocumentsContract;
import android.view.Gravity;
import android.view.View;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;

import java.io.File;
import java.io.IOException;
import java.util.Locale;

public final class MainActivity extends Activity {
    private static final int PICK_APK = 1001;
    private static final int PICK_DONUTHLE_FOLDER = 1002;
    private final int background = 0xff101416;
    private final int panel = 0xff192125;
    private final int teal = 0xff62d6c4;
    private final int text = 0xfff3f7f6;
    private final int muted = 0xffa5b5b2;

    @Override public void onCreate(Bundle state) { super.onCreate(state); showHome(); }

    private void showHome() {
        LinearLayout content = page("DONUTHLE", "Android 1.6 high-level emulator prototype");
        content.addView(card("▣", "GAME LIBRARY", "Add APKs, scan DonutHLE_apps, and launch compatible games.", "OPEN LIBRARY", v -> showLibrary()), margins(0, 24, 0, 10));
        content.addView(card("▤", "GAME SANDBOX", "Open the real DonutHLE sandbox folder containing APKs and game data.", "OPEN SANDBOX", v -> openFolder(StorageLayout.sandbox(this))), margins(0, 0, 0, 10));
        content.addView(card("⚙", "OPTIONS", "Storage locations, compatibility switches, and emulator settings.", "OPEN OPTIONS", v -> showOptions()), margins(0, 0, 0, 10));
        content.addView(card("?", "ABOUT", "Runtime status and current implementation notes.", "ABOUT DONUTHLE", v -> showAbout()), margins(0, 0, 0, 10));
        content.addView(button("VIEW LOG", false, v -> showLog()), margins(0, 0, 0, 8));
        setContentView(scroll(content));
        StorageLayout.appendLog(this, "MAIN_MENU_OPENED");
        CompatFeatures.recordImplemented(this);
    }

    private void showLibrary() {
        LinearLayout content = page("GAME LIBRARY", "APK files stored in DonutHLE_apps.");
        content.addView(button("ADD APK", true, v -> chooseApk()), margins(0, 18, 0, 8));
        content.addView(button("IMPORT FROM DONUTHLE_APPS", false, v -> chooseDonutHleFolder()), margins(0, 0, 0, 18));
        File[] apks = StorageLayout.apks(this);
        if (apks.length == 0) content.addView(label("No APKs yet. Add an Android 1.6 APK or import a folder.", 15, muted));
        for (File apk : apks) content.addView(card("▸", apk.getName(), formatBytes(apk.length()), "LAUNCH GAME", v -> launchApk(apk)), margins(0, 0, 0, 10));
        content.addView(button("‹ BACK", false, v -> showHome()), margins(0, 12, 0, 0));
        setContentView(scroll(content));
    }

    private void chooseApk() {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("*/*");
        intent.putExtra(Intent.EXTRA_MIME_TYPES, new String[] {"application/vnd.android.package-archive", "application/octet-stream"});
        startActivityForResult(intent, PICK_APK);
    }

    private void importApk(Uri uri) {
        try { File apk = StorageLayout.copyApk(this, uri); CompatibilityLog.recordImport(this, apk); Toast.makeText(this, "Added " + apk.getName(), Toast.LENGTH_SHORT).show(); showLibrary(); }
        catch (IOException error) { StorageLayout.appendLog(this, "APK_IMPORT_FAILED: " + error.getMessage()); Toast.makeText(this, "Could not add APK: " + error.getMessage(), Toast.LENGTH_LONG).show(); }
    }

    @Override protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (resultCode != RESULT_OK || data == null || data.getData() == null) return;
        if (requestCode == PICK_APK) { importApk(data.getData()); return; }
        if (requestCode == PICK_DONUTHLE_FOLDER) {
            Uri tree = data.getData();
            try { getContentResolver().takePersistableUriPermission(tree, Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION); } catch (Exception ignored) {}
            StorageLayout.saveTree(this, tree);
            int imported = StorageLayout.importFromTree(this, tree);
            Toast.makeText(this, imported + " APK(s) found in selected folder", Toast.LENGTH_SHORT).show();
            showLibrary();
        }
    }

    private void launchApk(File apk) {
        StorageLayout.appendLog(this, "LAUNCH_REQUESTED: " + apk.getName());
        CompatibilityLog.recordLaunchAttempt(this, apk);
        ApkCompatibility.Report report;
        try { report = ApkCompatibility.inspect(apk); }
        catch (IOException error) { Toast.makeText(this, "Cannot read APK: " + error.getMessage(), Toast.LENGTH_LONG).show(); return; }
        StringBuilder message = new StringBuilder();
        message.append("APK inspection complete\n\n");
        message.append("File: ").append(report.fileName).append("\n");
        message.append("Size: ").append(formatBytes(report.fileSize)).append("\n");
        message.append("Manifest: ").append(report.hasManifest ? "found" : "missing").append("\n");
        message.append("Dalvik classes.dex: ").append(report.hasDex ? "found" : "missing").append("\n");
        message.append("Resources: ").append(report.hasResources ? "found" : "missing").append("\n\n");
        message.append("Rust runtime result:\n").append(nativeLaunchApk(apk.getAbsolutePath()));
        showMessage("LAUNCH REPORT", message.toString());
    }

    private void showOptions() { LinearLayout content = page("OPTIONS", "Portable settings and file locations."); TextView options = label(StorageLayout.readText(StorageLayout.options(this)), 15, text); options.setTextIsSelectable(true); options.setPadding(dp(14), dp(14), dp(14), dp(14)); options.setBackgroundColor(panel); content.addView(options, margins(0, 18, 0, 16)); content.addView(button("OPEN DONUTHLE FOLDER", false, v -> openFolder(StorageLayout.root(this))), margins(0, 0, 0, 8)); content.addView(button("‹ BACK", false, v -> showHome())); setContentView(scroll(content)); }
    private void showLog() { StorageLayout.appendLog(this, "LOG_VIEW_OPENED"); TextView log = label(nativeRuntimeInfo() + "\n\n" + StorageLayout.readText(StorageLayout.logFile(this)), 14, text); log.setTextIsSelectable(true); log.setPadding(dp(14), dp(14), dp(14), dp(14)); log.setTypeface(android.graphics.Typeface.MONOSPACE); log.setBackgroundColor(panel); LinearLayout content = page("EMULATOR LOG", "UTF-8 compatibility log with unimplemented features."); content.addView(log, margins(0, 16, 0, 16)); content.addView(button("‹ BACK", true, v -> showHome())); setContentView(scroll(content)); }
    private void showAbout() { LinearLayout content = page("ABOUT DONUTHLE", "A clean-room Android 1.6 HLE project."); content.addView(label("The APK pipeline scans packages, reports requested APIs, and launches the Rust runtime bridge. Dalvik execution and Android framework shims are active development milestones.", 16, text), margins(0, 18, 0, 20)); content.addView(button("‹ BACK", true, v -> showHome())); setContentView(scroll(content)); }
    private void showMessage(String heading, String message) { LinearLayout content = page(heading, "Compatibility inspection"); content.addView(label(message, 16, text), margins(0, 18, 0, 20)); content.addView(button("‹ BACK TO LIBRARY", false, v -> showLibrary())); setContentView(scroll(content)); }

    private void chooseDonutHleFolder() { Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE); intent.putExtra("android.content.extra.SHOW_ADVANCED", true); startActivityForResult(intent, PICK_DONUTHLE_FOLDER); }
    private void openFolder(File folder) {
        if (!folder.exists()) folder.mkdirs();
        Uri saved = StorageLayout.savedTree(this);
        if (saved != null) {
            try { startActivityForResult(new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).setData(saved).putExtra("android.content.extra.SHOW_ADVANCED", true), PICK_DONUTHLE_FOLDER); return; } catch (Exception ignored) {}
        }
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
        intent.putExtra("android.content.extra.SHOW_ADVANCED", true);
        intent.putExtra("android.provider.extra.SHOW_ADVANCED", true);
        intent.putExtra("android.content.extra.INITIAL_URI", DocumentsContract.buildRootUri("org.donuthle.android.documents", "donuthle"));
        try { startActivityForResult(intent, PICK_DONUTHLE_FOLDER); } catch (Exception error) { chooseDonutHleFolder(); }
    }
    private LinearLayout page(String heading, String description) { LinearLayout content = column(); content.setPadding(dp(22), dp(20), dp(22), dp(28)); content.addView(label("DONUTHLE", 22, teal)); TextView h = label(heading, 28, text); h.setTypeface(null, 1); content.addView(h, margins(0, 28, 0, 4)); content.addView(label(description, 15, muted)); return content; }
    private LinearLayout card(String icon, String heading, String description, String action, View.OnClickListener listener) { LinearLayout card = new LinearLayout(this); card.setOrientation(LinearLayout.VERTICAL); card.setPadding(dp(16), dp(14), dp(16), dp(14)); card.setBackgroundColor(panel); card.addView(label(icon, 26, teal)); TextView h = label(heading, 15, text); h.setTypeface(null, 1); card.addView(h, margins(0, 6, 0, 3)); card.addView(label(description, 14, muted), margins(0, 0, 0, 10)); card.addView(button(action, false, listener)); return card; }
    private LinearLayout column() { LinearLayout layout = new LinearLayout(this); layout.setOrientation(LinearLayout.VERTICAL); layout.setBackgroundColor(background); return layout; }
    private ScrollView scroll(View child) { ScrollView scroll = new ScrollView(this); scroll.setFillViewport(true); scroll.setBackgroundColor(background); scroll.addView(child); return scroll; }
    private TextView label(String value, int size, int color) { TextView view = new TextView(this); view.setText(value); view.setTextSize(size); view.setTextColor(color); return view; }
    private Button button(String title, boolean filled, View.OnClickListener listener) { Button button = new Button(this); button.setText(title); button.setTextSize(13); button.setTextColor(filled ? background : teal); button.setAllCaps(false); button.setGravity(Gravity.CENTER); button.setMinHeight(dp(48)); button.setBackgroundColor(filled ? teal : panel); if (listener != null) button.setOnClickListener(listener); return button; }
    private LinearLayout.LayoutParams margins(int left, int top, int right, int bottom) { LinearLayout.LayoutParams p = new LinearLayout.LayoutParams(-1, -2); p.setMargins(dp(left), dp(top), dp(right), dp(bottom)); return p; }
    private int dp(int value) { return Math.round(value * getResources().getDisplayMetrics().density); }
    private String formatBytes(long bytes) { if (bytes > 1024 * 1024) return String.format(Locale.US, "%.1f MB", bytes / 1024f / 1024f); return (bytes / 1024) + " KB"; }
    private native String nativeLaunchApk(String path);
    private native String nativeRuntimeInfo();
    static { System.loadLibrary("donuthle"); }
}
