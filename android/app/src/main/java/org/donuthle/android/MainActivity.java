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

public final class MainActivity extends Activity {
    private static final int REQUEST_IMPORT_APK = 42;
    private static final int REQUEST_APPS_TREE = 43;
    static { System.loadLibrary("donuthle"); }
    private native String nativeRuntimeInfo();
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
        StorageLayout.appendLog(this, "SESSION_START: DonutHLE main menu opened");
        CompatibilityLog.recordStartup(this);
        showHome();
    }

    @Override protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (resultCode != RESULT_OK || data == null || data.getData() == null) return;
        Uri source = data.getData();
        if (requestCode == REQUEST_IMPORT_APK) {
            try { getContentResolver().takePersistableUriPermission(source, Intent.FLAG_GRANT_READ_URI_PERMISSION); } catch (SecurityException ignored) {}
            try {
                File imported = StorageLayout.copyApk(this, source);
                StorageLayout.appendLog(this, "APK_IMPORTED: " + imported.getName());
                Toast.makeText(this, "Imported " + imported.getName(), Toast.LENGTH_SHORT).show();
            } catch (Exception error) {
                StorageLayout.appendLog(this, "ERROR[APK_IMPORT]: " + error.getMessage());
                Toast.makeText(this, "APK import failed. Open Log.", Toast.LENGTH_LONG).show();
            }
            showLibrary();
        } else if (requestCode == REQUEST_APPS_TREE) {
            try { getContentResolver().takePersistableUriPermission(source, Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION); } catch (SecurityException ignored) {}
            StorageLayout.saveTree(this, source);
            int count = StorageLayout.importFromTree(this, source);
            Toast.makeText(this, count == 0 ? "No APK files found" : "Imported " + count + " APK(s)", Toast.LENGTH_SHORT).show();
            showLibrary();
        }
    }

    private void showHome() {
        StorageLayout.ensure(this);
        LinearLayout content = column();
        content.setPadding(dp(22), dp(20), dp(22), dp(28));
        TextView brand = label("DONUTHLE", 27, teal); brand.setTypeface(null, 1); content.addView(brand);
        content.addView(label("Android 1.6 • Donut • HLE prototype", 14, muted), margins(0, 2, 0, 22));
        TextView status = label("READY TO EXPLORE", 12, Color.rgb(112, 214, 164)); status.setTypeface(null, 1); content.addView(status);
        TextView title = label("Your games,\nyour sandbox.", 31, text); title.setTypeface(null, 1); content.addView(title, margins(0, 5, 0, 8));
        content.addView(label("Import legal APK files and keep each game's files isolated in its own sandbox.", 15, muted), margins(0, 0, 0, 22));
        content.addView(card("▣", "GAME LIBRARY", "Import APK files into DonutHLE_apps", "IMPORT APK", v -> showLibrary()), margins(0, 0, 0, 10));
        content.addView(card("⌁", "GAME SANDBOX", "Browse save data and per-game files", "OPEN FOLDER", v -> openFolder(StorageLayout.sandbox(this))), margins(0, 0, 0, 10));
        content.addView(card("≡", "EMULATOR LOG", "Read compatibility and unimplemented-feature reports", "VIEW LOG", v -> showLog()), margins(0, 0, 0, 10));
        File[] apks = StorageLayout.apks(this);
        TextView library = label(apks.length == 0 ? "APK LIBRARY IS EMPTY\nUse IMPORT APK to add a game." : apks.length + " APK" + (apks.length == 1 ? "" : "S") + " READY\n" + apkNames(apks), 13, apks.length == 0 ? muted : text);
        library.setPadding(dp(14), dp(13), dp(14), dp(13)); library.setBackgroundColor(panel); content.addView(library, margins(0, 12, 0, 18));
        Button options = button("OPTIONS  ›", false); options.setOnClickListener(v -> showOptions()); content.addView(options, margins(0, 0, 0, 8));
        Button about = button("ABOUT DONUTHLE  ›", false); about.setOnClickListener(v -> showAbout()); content.addView(about);
        setContentView(scroll(content));
    }

    private String apkNames(File[] apks) { StringBuilder names = new StringBuilder(); for (File apk : apks) { if (names.length() > 0) names.append("\n"); names.append("• ").append(apk.getName()); } return names.toString(); }

    private View card(String icon, String heading, String description, String action, View.OnClickListener listener) {
        LinearLayout row = new LinearLayout(this); row.setOrientation(LinearLayout.HORIZONTAL); row.setGravity(Gravity.CENTER_VERTICAL); row.setPadding(dp(14), dp(14), dp(12), dp(14)); row.setBackgroundColor(panel); row.setOnClickListener(listener);
        TextView symbol = label(icon, 28, teal); symbol.setGravity(Gravity.CENTER); row.addView(symbol, new LinearLayout.LayoutParams(dp(40), dp(60)));
        LinearLayout copy = column(); copy.setPadding(dp(12), 0, dp(8), 0); TextView headingView = label(heading, 13, teal); headingView.setTypeface(null, 1); copy.addView(headingView); copy.addView(label(description, 15, text), margins(0, 3, 0, 0)); copy.addView(label(action, 11, muted), margins(0, 7, 0, 0)); row.addView(copy, new LinearLayout.LayoutParams(0, -2, 1)); row.addView(label("›", 28, muted)); return row;
    }

    private void showLibrary() {
        LinearLayout content = page("GAME LIBRARY", "Import APKs or scan an existing DonutHLE_apps folder.");
        content.addView(button("＋  IMPORT APK", true, v -> importApk()), margins(0, 18, 0, 10));
        content.addView(button("SCAN DONUTHLE_APPS FOLDER", false, v -> chooseAppsFolder()), margins(0, 0, 0, 14));
        File[] apks = StorageLayout.apks(this);
        if (apks.length == 0) content.addView(label("No APKs found yet.\n\nUse IMPORT APK, or choose a folder containing .apk files with SCAN DONUTHLE_APPS FOLDER.", 16, muted), margins(0, 0, 0, 14));
        else for (File apk : apks) content.addView(gameRow(apk), margins(0, 0, 0, 10));
        content.addView(button("OPEN DONUTHLE_APPS  ›", false, v -> openFolder(StorageLayout.apps(this))), margins(0, 8, 0, 8));
        content.addView(button("‹  BACK", false, v -> showHome()));
        setContentView(scroll(content));
    }

    private View gameRow(File apk) {
        LinearLayout row = new LinearLayout(this); row.setOrientation(LinearLayout.VERTICAL); row.setPadding(dp(16), dp(14), dp(16), dp(14)); row.setBackgroundColor(panel);
        TextView name = label(apk.getName(), 16, text); name.setTypeface(null, 1); row.addView(name); row.addView(label(formatBytes(apk.length()) + " • Android package", 13, muted), margins(0, 4, 0, 10)); row.addView(button("▶  RUN GAME", true, v -> launchApk(apk))); return row;
    }

    private void importApk() { StorageLayout.appendLog(this, "Opening APK picker"); Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT); intent.addCategory(Intent.CATEGORY_OPENABLE); intent.setType("application/vnd.android.package-archive"); intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, false); startActivityForResult(intent, REQUEST_IMPORT_APK); }
    private void chooseAppsFolder() { StorageLayout.appendLog(this, "Opening external folder picker for APK scan"); Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE); intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION); startActivityForResult(intent, REQUEST_APPS_TREE); }
    private void openFolder(File folder) { StorageLayout.appendLog(this, "Opening file browser: " + folder.getName()); Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE); intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION); try { startActivity(intent); } catch (Exception error) { StorageLayout.appendLog(this, "ERROR[FILE_BROWSER]: " + error.getMessage()); Toast.makeText(this, "File browser unavailable; use IMPORT APK.", Toast.LENGTH_LONG).show(); } }

    private void launchApk(File apk) { StorageLayout.appendLog(this, "LAUNCH_REQUEST: " + apk.getName()); CompatibilityLog.inspectApk(this, apk); Toast.makeText(this, "Compatibility scan saved to Log. Runtime launch is not implemented yet.", Toast.LENGTH_LONG).show(); showLog(); }
    private void showLog() { StorageLayout.appendLog(this, "Opened emulator log"); LinearLayout content = page("EMULATOR LOG", "UTF-8 trace • missing features are marked UNIMPLEMENTED"); TextView log = label(StorageLayout.readLog(this) + "\n" + CompatibilityLog.statusText() + "\n" + nativeRuntimeInfo(), 14, text); log.setTextIsSelectable(true); log.setTypeface(android.graphics.Typeface.MONOSPACE); log.setPadding(dp(14), dp(14), dp(14), dp(14)); log.setBackgroundColor(panel); content.addView(log, margins(0, 16, 0, 16)); content.addView(button("‹  BACK", true, v -> showHome())); setContentView(scroll(content)); }

    private void showOptions() { LinearLayout content = page("OPTIONS", "Settings and file locations"); content.addView(label("APK library\n" + StorageLayout.apps(this).getAbsolutePath() + "\n\nGame sandbox\n" + StorageLayout.sandbox(this).getAbsolutePath() + "\n\nLog file\n" + StorageLayout.logFile(this).getAbsolutePath(), 14, text), margins(0, 18, 0, 20)); content.addView(button("OPEN APK FOLDER", false, v -> openFolder(StorageLayout.apps(this))), margins(0, 0, 0, 8)); content.addView(button("OPEN SANDBOX FOLDER", false, v -> openFolder(StorageLayout.sandbox(this)), margins(0, 0, 0, 8)); content.addView(button("OPEN LOG", false, v -> showLog()), margins(0, 0, 0, 8)); content.addView(button("‹  BACK", true, v -> showHome())); setContentView(scroll(content)); }
    private void showAbout() { LinearLayout content = page("ABOUT DONUTHLE", "A clean-room Android 1.6 HLE project."); content.addView(label("The goal is to reproduce the small platform surface used by historical games without distributing Android system images or copyrighted game files.\n\nThe APK library, sandbox folders, compatibility scanner, DEX header/table parser, UTF-8 log, and Android shell are implemented. Full Dalvik instruction execution, binary AXML decoding, Android framework HLE, graphics, audio, input, JNI, and actual game launch remain explicit next milestones.", 16, text), margins(0, 18, 0, 20)); content.addView(button("‹  BACK", true, v -> showHome())); setContentView(scroll(content)); }

    private LinearLayout page(String heading, String description) { LinearLayout content = column(); content.setPadding(dp(22), dp(20), dp(22), dp(28)); content.addView(label("DONUTHLE", 22, teal)); TextView headingView = label(heading, 28, text); headingView.setTypeface(null, 1); content.addView(headingView, margins(0, 28, 0, 4)); content.addView(label(description, 15, muted)); return content; }
    private LinearLayout column() { LinearLayout layout = new LinearLayout(this); layout.setOrientation(LinearLayout.VERTICAL); layout.setBackgroundColor(background); return layout; }
    private ScrollView scroll(View child) { ScrollView scroll = new ScrollView(this); scroll.setFillViewport(true); scroll.setBackgroundColor(background); scroll.addView(child); return scroll; }
    private TextView label(String value, int size, int color) { TextView view = new TextView(this); view.setText(value); view.setTextSize(size); view.setTextColor(color); return view; }
    private Button button(String title, boolean filled) { return button(title, filled, null); }
    private Button button(String title, boolean filled, View.OnClickListener listener) { Button button = new Button(this); button.setText(title); button.setTextSize(13); button.setTextColor(filled ? background : teal); button.setAllCaps(false); button.setGravity(Gravity.CENTER); button.setMinHeight(dp(48)); button.setBackgroundColor(filled ? teal : panel); if (listener != null) button.setOnClickListener(listener); return button; }
    private LinearLayout.LayoutParams margins(int left, int top, int right, int bottom) { LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, -2); params.setMargins(dp(left), dp(top), dp(right), dp(bottom)); return params; }
    private int dp(int value) { return Math.round(value * getResources().getDisplayMetrics().density); }
    private String formatBytes(long bytes) { if (bytes < 1024) return bytes + " B"; if (bytes < 1024 * 1024) return (bytes / 1024) + " KB"; return (bytes / (1024 * 1024)) + " MB"; }
}
