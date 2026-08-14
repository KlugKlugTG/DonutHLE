package org.donuthle.android;

import android.app.Activity;
import android.content.Intent;
import android.graphics.Color;
import android.net.Uri;
import android.os.Bundle;
import android.view.Gravity;
import android.view.View;
import android.view.Window;
import android.view.WindowManager;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;

import java.io.File;
import java.io.IOException;

public final class MainActivity extends Activity {
    static { System.loadLibrary("donuthle"); }
    private native String nativeRuntimeInfo();
    private native String nativeLaunchApk(String path);
    private static final int PICK_APK = 42;
    private final int teal = Color.rgb(128, 203, 196);
    private final int text = Color.rgb(232, 240, 242);
    private final int muted = Color.rgb(158, 174, 180);
    private final int panel = Color.rgb(27, 33, 38);
    private final int background = Color.rgb(14, 18, 22);

    @Override protected void onCreate(Bundle state) {
        super.onCreate(state);
        requestWindowFeature(Window.FEATURE_NO_TITLE);
        getWindow().setFlags(WindowManager.LayoutParams.FLAG_FULLSCREEN, WindowManager.LayoutParams.FLAG_FULLSCREEN);
        StorageLayout.ensure(this);
        StorageLayout.appendLog(this, "MAIN_MENU_OPENED");
        CompatibilityLog.onStartup(this);
        showHome();
    }

    @Override protected void onResume() {
        super.onResume();
        if (findViewById(7001) != null) showLibrary();
    }

    private void showHome() {
        LinearLayout content = column();
        content.setPadding(dp(22), dp(20), dp(22), dp(28));
        TextView brand = label("DONUTHLE", 27, teal); brand.setTypeface(null, 1); content.addView(brand);
        content.addView(label("Android 1.6 • Donut • HLE prototype", 14, muted), margins(0, 2, 0, 22));
        TextView status = label("READY TO EXPLORE", 12, Color.rgb(112, 214, 164)); status.setTypeface(null, 1); content.addView(status);
        TextView title = label("Your games,\nyour sandbox.", 31, text); title.setTypeface(null, 1); content.addView(title, margins(0, 5, 0, 8));
        content.addView(label("Import APKs or place them in DonutHLE_apps.", 15, muted), margins(0, 0, 0, 22));
        content.addView(card("▣", "GAME LIBRARY", "Scan, inspect, and launch an APK", "Open library", v -> showLibrary()), margins(0, 0, 0, 10));
        content.addView(card("⌁", "GAME SANDBOX", "Browse per-game writable data", "Open sandbox", v -> openFolder(StorageLayout.sandbox(this))), margins(0, 0, 0, 10));
        content.addView(card("≡", "EMULATOR LOG", "See every compatibility gap", "View log", v -> showLog()), margins(0, 0, 0, 10));
        TextView paths = label("FILES LIVE IN\n" + StorageLayout.publicRoot(this).getAbsolutePath(), 12, muted); paths.setPadding(dp(14), dp(13), dp(14), dp(13)); paths.setBackgroundColor(panel); content.addView(paths, margins(0, 12, 0, 18));
        content.addView(button("OPTIONS  ›", false, v -> showOptions()), margins(0, 0, 0, 8));
        content.addView(button("ABOUT DONUTHLE  ›", false, v -> showAbout()));
        setContentView(scroll(content));
    }

    private void showLibrary() {
        StorageLayout.ensure(this);
        LinearLayout content = page("GAME LIBRARY", "Import APKs or scan an existing DonutHLE_apps folder.");
        content.setId(7001);
        content.addView(button("＋  IMPORT APK", true, v -> pickApk()), margins(0, 18, 0, 8));
        content.addView(button("↻  REFRESH LIBRARY", false, v -> showLibrary()), margins(0, 0, 0, 14));
        File[] apks = StorageLayout.apks(this);
        if (apks.length == 0) {
            TextView empty = label("No APKs found.\n\nUse IMPORT APK or copy a legal .apk into DonutHLE_apps, then press REFRESH LIBRARY.", 16, muted); empty.setPadding(dp(16), dp(18), dp(16), dp(18)); empty.setBackgroundColor(panel); content.addView(empty, margins(0, 0, 0, 14));
        } else for (File apk : apks) content.addView(gameRow(apk), margins(0, 0, 0, 10));
        content.addView(button("OPEN DONUTHLE_APPS  ›", false, v -> openFolder(StorageLayout.apps(this))), margins(0, 8, 0, 8));
        content.addView(button("‹  BACK", false, v -> showHome()));
        setContentView(scroll(content));
    }

    private View gameRow(File apk) {
        LinearLayout row = new LinearLayout(this); row.setOrientation(LinearLayout.VERTICAL); row.setPadding(dp(16), dp(14), dp(16), dp(14)); row.setBackgroundColor(panel);
        TextView name = label(apk.getName(), 16, text); name.setTypeface(null, 1); row.addView(name);
        row.addView(label(formatBytes(apk.length()) + "  •  Android package", 13, muted), margins(0, 4, 0, 8));
        row.addView(button("▶  RUN / INSPECT", true, v -> launchApk(apk)));
        return row;
    }

    private void pickApk() {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT); intent.addCategory(Intent.CATEGORY_OPENABLE); intent.setType("*/*"); intent.putExtra(Intent.EXTRA_MIME_TYPES, new String[]{"application/vnd.android.package-archive", "application/octet-stream", "*/*"}); startActivityForResult(intent, PICK_APK);
    }

    private void importApk(Uri uri) {
        try { File apk = StorageLayout.copyApk(this, uri); StorageLayout.appendLog(this, "APK_IMPORTED: " + apk.getName()); CompatibilityLog.recordApk(this, apk); Toast.makeText(this, "Added " + apk.getName(), Toast.LENGTH_SHORT).show(); showLibrary(); }
        catch (IOException error) { StorageLayout.appendLog(this, "APK_IMPORT_FAILED: " + error.getMessage()); Toast.makeText(this, "Could not add APK: " + error.getMessage(), Toast.LENGTH_LONG).show(); }
    }

    @Override protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == PICK_APK && resultCode == RESULT_OK && data != null && data.getData() != null) importApk(data.getData());
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
        message.append("\nRust runtime result:\n").append(nativeLaunchApk(apk.getAbsolutePath()));
        showMessage("LAUNCH REPORT", message.toString());
    }

    private void showOptions() { LinearLayout content = page("OPTIONS", "Portable settings and file locations."); TextView options = label(StorageLayout.readText(StorageLayout.options(this)), 15, text); options.setTextIsSelectable(true); options.setPadding(dp(14), dp(14), dp(14), dp(14)); options.setBackgroundColor(panel); content.addView(options, margins(0, 18, 0, 16)); content.addView(button("OPEN DONUTHLE FOLDER", false, v -> openFolder(StorageLayout.publicRoot(this))), margins(0, 0, 0, 8)); content.addView(button("‹  BACK", false, v -> showHome())); setContentView(scroll(content)); }
    private void showLog() { StorageLayout.appendLog(this, "LOG_VIEW_OPENED"); LinearLayout content = page("EMULATOR LOG", "UTF-8 compatibility log with unimplemented features."); TextView log = label(nativeRuntimeInfo() + "\n\n" + StorageLayout.readText(StorageLayout.logFile(this)), 14, text); log.setTextIsSelectable(true); log.setPadding(dp(14), dp(14), dp(14), dp(14)); log.setTypeface(android.graphics.Typeface.MONOSPACE); log.setBackgroundColor(panel); content.addView(log, margins(0, 16, 0, 16)); content.addView(button("‹  BACK", true, v -> showHome())); setContentView(scroll(content)); }
    private void showAbout() { LinearLayout content = page("ABOUT DONUTHLE", "A clean-room Android 1.6 HLE project."); content.addView(label("The current APK pipeline scans packages, reports their requested APIs, and prepares the compatibility log. Dalvik execution, Android framework shims, graphics, audio, input, and activity launching remain active development milestones.", 16, text), margins(0, 18, 0, 20)); content.addView(button("‹  BACK", true, v -> showHome())); setContentView(scroll(content)); }
    private void showMessage(String heading, String message) { LinearLayout content = page(heading, "Compatibility inspection"); content.addView(label(message, 16, text), margins(0, 18, 0, 20)); content.addView(button("‹  BACK TO LIBRARY", false, v -> showLibrary())); setContentView(scroll(content)); }
    private void openFolder(File folder) { Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT); intent.addCategory(Intent.CATEGORY_OPENABLE); intent.setType("*/*"); try { startActivity(intent); } catch (Exception error) { Toast.makeText(this, folder.getAbsolutePath(), Toast.LENGTH_LONG).show(); } }
    private LinearLayout page(String heading, String description) { LinearLayout content = column(); content.setPadding(dp(22), dp(20), dp(22), dp(28)); content.addView(label("DONUTHLE", 22, teal)); TextView h = label(heading, 28, text); h.setTypeface(null, 1); content.addView(h, margins(0, 28, 0, 4)); content.addView(label(description, 15, muted)); return content; }
    private LinearLayout card(String icon, String heading, String description, String action, View.OnClickListener listener) { LinearLayout card = new LinearLayout(this); card.setOrientation(LinearLayout.VERTICAL); card.setPadding(dp(16), dp(14), dp(16), dp(14)); card.setBackgroundColor(panel); card.addView(label(icon, 26, teal)); TextView h = label(heading, 15, text); h.setTypeface(null, 1); card.addView(h, margins(0, 6, 0, 3)); card.addView(label(description, 14, muted), margins(0, 0, 0, 10)); card.addView(button(action, false, listener)); return card; }
    private LinearLayout column() { LinearLayout layout = new LinearLayout(this); layout.setOrientation(LinearLayout.VERTICAL); layout.setBackgroundColor(background); return layout; }
    private ScrollView scroll(View child) { ScrollView scroll = new ScrollView(this); scroll.setFillViewport(true); scroll.setBackgroundColor(background); scroll.addView(child); return scroll; }
    private TextView label(String value, int size, int color) { TextView view = new TextView(this); view.setText(value); view.setTextSize(size); view.setTextColor(color); return view; }
    private Button button(String title, boolean filled, View.OnClickListener listener) { Button button = new Button(this); button.setText(title); button.setTextSize(13); button.setTextColor(filled ? background : teal); button.setAllCaps(false); button.setGravity(Gravity.CENTER); button.setMinHeight(dp(48)); button.setBackgroundColor(filled ? teal : panel); if (listener != null) button.setOnClickListener(listener); return button; }
    private LinearLayout.LayoutParams margins(int left, int top, int right, int bottom) { LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, -2); params.setMargins(dp(left), dp(top), dp(right), dp(bottom)); return params; }
    private String formatBytes(long bytes) { if (bytes < 1024) return bytes + " B"; if (bytes < 1024 * 1024) return (bytes / 1024) + " KB"; return (bytes / (1024 * 1024)) + " MB"; }
    private int dp(int value) { return Math.round(value * getResources().getDisplayMetrics().density); }
}
