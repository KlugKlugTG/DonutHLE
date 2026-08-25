package org.donuthle.android;

import android.app.Activity;
import android.content.Intent;
import android.graphics.Color;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.net.Uri;
import android.os.Bundle;
import android.view.Gravity;
import android.view.View;
import android.view.Window;
import android.view.WindowManager;
import android.widget.Button;
import android.widget.FrameLayout;
import android.widget.ImageView;
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
    private native String registerTrace();
    native void nativeRenderFrame(int width, int height);
    native int nativeTouchEvent(int action, float x, float y);

    private static final int PICK_APK = 42;
    private static final int PICK_DONUTHLE_FOLDER = 43;
    private static final int TEAL = Color.rgb(128, 203, 196);
    private static final int TEXT = Color.rgb(239, 246, 247);
    private static final int MUTED = Color.rgb(164, 181, 187);
    private static final int PANEL = Color.rgb(28, 37, 43);
    private static final int PANEL_LIGHT = Color.rgb(36, 48, 55);
    private static final int BACKGROUND = Color.rgb(10, 15, 19);
    private static final int SUCCESS = Color.rgb(112, 214, 164);
    private Gles1SurfaceView gameSurface;

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
        if (gameSurface != null) gameSurface.onResume();
    }

    @Override protected void onPause() {
        if (gameSurface != null) gameSurface.onPause();
        super.onPause();
    }

    @Override public void onBackPressed() {
        if (gameSurface != null) {
            showLibrary();
            return;
        }
        super.onBackPressed();
    }

    private void stopGameSurface() {
        if (gameSurface != null) {
            gameSurface.onPause();
            gameSurface = null;
        }
    }

    private void showHome() {
        stopGameSurface();
        StorageLayout.ensure(this);
        StorageLayout.importFromDefaultFolder(this);
        LinearLayout content = column();
        content.setPadding(dp(20), dp(18), dp(20), dp(28));
        content.addView(header(), margins(0, 0, 0, 22));

        LinearLayout hero = surface(PANEL, 22);
        hero.setPadding(dp(20), dp(21), dp(20), dp(20));
        TextView status = label("●  READY TO EXPLORE", 12, SUCCESS);
        status.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        hero.addView(status);
        TextView title = label("Your games,\nyour sandbox.", 31, TEXT);
        title.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        hero.addView(title, margins(0, 9, 0, 8));
        hero.addView(label("Import an APK, inspect its compatibility, and launch it through the DonutHLE runtime.", 15, MUTED), margins(0, 0, 0, 18));
        hero.addView(button("OPEN GAME LIBRARY  ›", true, v -> showLibrary()));
        content.addView(hero, margins(0, 0, 0, 18));

        content.addView(sectionTitle("QUICK ACCESS", "Everything you need is one tap away."), margins(2, 0, 0, 10));
        content.addView(actionCard("▣", "Game Library", "Import, inspect, and launch APKs", v -> showLibrary()), margins(0, 0, 0, 10));
        content.addView(actionCard("⌂", "Game Sandbox", "Browse writable game data", v -> showFolderInfo(StorageLayout.sandbox(this))), margins(0, 0, 0, 10));
        content.addView(actionCard("≡", "Emulator Log", "See compatibility gaps and runtime events", v -> showLog()), margins(0, 0, 0, 18));

        LinearLayout files = surface(PANEL_LIGHT, 16);
        files.setPadding(dp(16), dp(14), dp(16), dp(14));
        files.addView(label("YOUR DONUTHLE FOLDER", 11, TEAL));
        TextView folderPath = label(StorageLayout.publicRoot(this).getAbsolutePath(), 12, MUTED);
        folderPath.setTextIsSelectable(true);
        files.addView(folderPath, margins(0, 5, 0, 0));
        content.addView(files, margins(0, 0, 0, 16));
        content.addView(button("OPTIONS  ›", false, v -> showOptions()), margins(0, 0, 0, 8));
        content.addView(button("ABOUT DONUTHLE  ›", false, v -> showAbout()));
        setContentView(scroll(content));
    }

    private View header() {
        LinearLayout header = new LinearLayout(this);
        header.setGravity(Gravity.CENTER_VERTICAL);
        ImageView icon = new ImageView(this);
        icon.setImageResource(R.mipmap.ic_launcher);
        icon.setContentDescription("DonutHLE icon");
        header.addView(icon, new LinearLayout.LayoutParams(dp(58), dp(58)));
        LinearLayout names = new LinearLayout(this);
        names.setOrientation(LinearLayout.VERTICAL);
        names.setPadding(dp(13), 0, 0, 0);
        TextView brand = label("DONUTHLE", 25, TEXT);
        brand.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        names.addView(brand);
        names.addView(label("ANDROID 1.x-2.x COMPATIBILITY LAB", 11, TEAL), margins(0, 3, 0, 0));
        header.addView(names, new LinearLayout.LayoutParams(0, -2, 1));
        TextView version = label("0.1.3", 11, MUTED);
        version.setGravity(Gravity.CENTER);
        version.setBackground(round(PANEL_LIGHT, 30));
        version.setPadding(dp(10), dp(7), dp(10), dp(7));
        header.addView(version);
        return header;
    }

    private void showLibrary() {
        stopGameSurface();
        StorageLayout.ensure(this);
        StorageLayout.importFromDefaultFolder(this);
        LinearLayout content = page("GAME LIBRARY", "Your imported APKs appear here. Choose an APK you own to inspect or run it.");
        content.setId(7001);
        content.addView(button("＋  IMPORT APK", true, v -> pickApk()), margins(0, 18, 0, 8));
        content.addView(button("↻  REFRESH LIBRARY", false, v -> showLibrary()), margins(0, 0, 0, 16));
        File[] apks = StorageLayout.apks(this);
        if (apks.length == 0) {
            LinearLayout empty = surface(PANEL, 18);
            empty.setGravity(Gravity.CENTER_HORIZONTAL);
            empty.setPadding(dp(20), dp(26), dp(20), dp(26));
            TextView glyph = label("＋", 34, TEAL);
            glyph.setGravity(Gravity.CENTER);
            empty.addView(glyph, new LinearLayout.LayoutParams(-1, dp(45)));
            TextView title = label("Your library is empty", 18, TEXT);
            title.setGravity(Gravity.CENTER);
            title.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
            empty.addView(title, margins(0, 7, 0, 5));
            TextView hint = label("Import an APK or copy one into DonutHLE_apps, then refresh.", 14, MUTED);
            hint.setGravity(Gravity.CENTER);
            empty.addView(hint);
            content.addView(empty, margins(0, 0, 0, 16));
        } else {
            content.addView(sectionTitle("INSTALLED APKs", apks.length + (apks.length == 1 ? " package" : " packages")), margins(2, 0, 0, 10));
            for (File apk : apks) content.addView(gameRow(apk), margins(0, 0, 0, 10));
        }
        content.addView(button("OPEN DONUTHLE_APPS  ›", false, v -> chooseDonutHleFolder()), margins(0, 8, 0, 8));
        content.addView(button("‹  BACK TO HOME", false, v -> showHome()));
        setContentView(scroll(content));
    }

    private View gameRow(File apk) {
        LinearLayout row = surface(PANEL, 18);
        row.setPadding(dp(16), dp(15), dp(16), dp(15));
        LinearLayout titleLine = new LinearLayout(this);
        titleLine.setGravity(Gravity.CENTER_VERTICAL);
        TextView glyph = label("▣", 23, TEAL);
        titleLine.addView(glyph, new LinearLayout.LayoutParams(dp(34), dp(34)));
        TextView name = label(apk.getName(), 16, TEXT);
        name.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        name.setMaxLines(2);
        titleLine.addView(name, new LinearLayout.LayoutParams(0, -2, 1));
        row.addView(titleLine);
        row.addView(label(formatBytes(apk.length()) + "  •  Android package", 13, MUTED), margins(34, 5, 0, 11));
        row.addView(button("▶  RUN / INSPECT", true, v -> launchApk(apk)));
        return row;
    }

    private void pickApk() {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("*/*");
        intent.putExtra(Intent.EXTRA_MIME_TYPES, new String[]{"application/vnd.android.package-archive", "application/octet-stream", "*/*"});
        startActivityForResult(intent, PICK_APK);
    }

    private void importApk(Uri uri) {
        try {
            File apk = StorageLayout.copyApk(this, uri);
            StorageLayout.appendLog(this, "APK_IMPORTED: " + apk.getName());
            CompatibilityLog.recordApk(this, apk);
            Toast.makeText(this, "Added " + apk.getName(), Toast.LENGTH_SHORT).show();
            showLibrary();
        } catch (IOException error) {
            StorageLayout.appendLog(this, "APK_IMPORT_FAILED: " + error.getMessage());
            Toast.makeText(this, "Could not add APK: " + error.getMessage(), Toast.LENGTH_LONG).show();
        }
    }

    @Override protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == PICK_APK && resultCode == RESULT_OK && data != null && data.getData() != null) importApk(data.getData());
        if (requestCode == PICK_DONUTHLE_FOLDER && resultCode == RESULT_OK && data != null && data.getData() != null) {
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
        message.append("\nRust runtime result:\n").append(nativeLaunchApk(apk.getAbsolutePath()));
        showGameScreen(apk, message.toString());
    }

    private void showGameScreen(File apk, String report) {
        FrameLayout root = new FrameLayout(this);
        root.setBackgroundColor(Color.BLACK);
        gameSurface = new Gles1SurfaceView(this, this);
        root.addView(gameSurface, new FrameLayout.LayoutParams(-1, -1));
        StorageLayout.appendLog(this, "GAME_SURFACE_STARTED: " + apk.getName() + "\n" + report);
        setContentView(root);
        gameSurface.onResume();
    }

    private void showOptions() {
        LinearLayout content = page("OPTIONS", "Portable storage and emulator information.");
        TextView options = label(StorageLayout.readText(StorageLayout.options(this)), 14, TEXT);
        options.setTextIsSelectable(true);
        options.setPadding(dp(16), dp(16), dp(16), dp(16));
        options.setTypeface(Typeface.MONOSPACE);
        options.setBackground(round(PANEL, 16));
        content.addView(options, margins(0, 18, 0, 16));
        content.addView(button("OPEN FOLDER PICKER", false, v -> chooseDonutHleFolder()), margins(0, 0, 0, 8));
        content.addView(button("‹  BACK TO HOME", false, v -> showHome()));
        setContentView(scroll(content));
    }

    private void showLog() {
        StorageLayout.appendLog(this, "LOG_VIEW_OPENED");
        LinearLayout content = page("EMULATOR LOG", "UTF-8 diagnostics for compatibility gaps, warnings, and runtime events.");
        TextView log = label(nativeRuntimeInfo() + "\n\n" + StorageLayout.readText(StorageLayout.logFile(this)), 13, TEXT);
        log.setTextIsSelectable(true);
        log.setPadding(dp(16), dp(16), dp(16), dp(16));
        log.setTypeface(Typeface.MONOSPACE);
        log.setBackground(round(PANEL, 16));
        content.addView(log, margins(0, 16, 0, 16));
        content.addView(button("‹  BACK TO HOME", true, v -> showHome()));
        setContentView(scroll(content));
    }

    private void showAbout() {
        LinearLayout content = page("ABOUT DONUTHLE", "A clean-room Android 1.x-2.x HLE project.");
        content.addView(infoBlock("WHAT IT DOES", "DonutHLE inspects APKs, runs a growing subset of Dalvik and Android 1.x-2.x APIs, loads selected libGDX assets, and presents the emulated framebuffer through GLES 2.0 on Android."), margins(0, 18, 0, 10));
        content.addView(infoBlock("WHAT THE LOG MEANS", "The emulator reports unsupported APIs and incomplete paths instead of hiding them. A visible frame is useful evidence, but does not mean an application is fully playable."), margins(0, 0, 0, 18));
        content.addView(button("‹  BACK TO HOME", true, v -> showHome()));
        setContentView(scroll(content));
    }

    private View infoBlock(String heading, String body) {
        LinearLayout block = surface(PANEL, 16);
        block.setPadding(dp(16), dp(15), dp(16), dp(15));
        TextView h = label(heading, 11, TEAL);
        h.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        block.addView(h);
        block.addView(label(body, 15, TEXT), margins(0, 7, 0, 0));
        return block;
    }

    private void showFolderInfo(File folder) {
        LinearLayout content = page("GAME SANDBOX", "Writable storage for the selected game runtime.");
        LinearLayout block = surface(PANEL, 16);
        block.setPadding(dp(16), dp(16), dp(16), dp(16));
        block.addView(label("FOLDER LOCATION", 11, TEAL));
        block.addView(label(folder.getAbsolutePath(), 14, TEXT), margins(0, 7, 0, 12));
        File[] files = folder.listFiles();
        int count = files == null ? 0 : files.length;
        block.addView(label(count == 0 ? "This sandbox is empty." : count + " item(s) in this sandbox.", 14, MUTED));
        content.addView(block, margins(0, 18, 0, 14));
        content.addView(button("OPEN FOLDER PICKER", false, v -> chooseDonutHleFolder()), margins(0, 0, 0, 8));
        content.addView(button("‹  BACK TO HOME", false, v -> showHome()));
        setContentView(scroll(content));
    }

    private void chooseDonutHleFolder() {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
        startActivityForResult(intent, PICK_DONUTHLE_FOLDER);
    }

    private LinearLayout page(String heading, String description) {
        LinearLayout content = column();
        content.setPadding(dp(20), dp(18), dp(20), dp(28));
        content.addView(header(), margins(0, 0, 0, 23));
        TextView h = label(heading, 28, TEXT);
        h.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        content.addView(h, margins(0, 0, 0, 5));
        content.addView(label(description, 15, MUTED));
        return content;
    }

    private TextView sectionTitle(String heading, String description) {
        TextView title = label(heading + "  ·  " + description, 11, TEAL);
        title.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        return title;
    }

    private LinearLayout actionCard(String icon, String heading, String description, View.OnClickListener listener) {
        LinearLayout card = surface(PANEL, 16);
        card.setOrientation(LinearLayout.HORIZONTAL);
        card.setGravity(Gravity.CENTER_VERTICAL);
        card.setPadding(dp(15), dp(13), dp(13), dp(13));
        TextView glyph = label(icon, 26, TEAL);
        glyph.setGravity(Gravity.CENTER);
        card.addView(glyph, new LinearLayout.LayoutParams(dp(43), dp(43)));
        LinearLayout text = new LinearLayout(this);
        text.setOrientation(LinearLayout.VERTICAL);
        TextView h = label(heading, 16, TEXT);
        h.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        text.addView(h);
        text.addView(label(description, 13, MUTED), margins(0, 4, 0, 0));
        card.addView(text, new LinearLayout.LayoutParams(0, -2, 1));
        TextView arrow = label("›", 28, MUTED);
        card.addView(arrow, new LinearLayout.LayoutParams(dp(24), -2));
        card.setOnClickListener(listener);
        card.setClickable(true);
        card.setContentDescription(heading + ": " + description);
        return card;
    }

    private LinearLayout column() {
        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        layout.setBackgroundColor(BACKGROUND);
        return layout;
    }

    private ScrollView scroll(View child) {
        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        scroll.setBackgroundColor(BACKGROUND);
        scroll.addView(child);
        return scroll;
    }

    private LinearLayout surface(int color, int radius) {
        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        layout.setBackground(round(color, radius));
        layout.setElevation(dp(2));
        return layout;
    }

    private TextView label(String value, int size, int color) {
        TextView view = new TextView(this);
        view.setText(value);
        view.setTextSize(size);
        view.setTextColor(color);
        view.setIncludeFontPadding(true);
        return view;
    }

    private Button button(String title, boolean filled, View.OnClickListener listener) {
        Button button = new Button(this);
        button.setText(title);
        button.setTextSize(13);
        button.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        button.setTextColor(filled ? BACKGROUND : TEAL);
        button.setAllCaps(false);
        button.setGravity(Gravity.CENTER);
        button.setMinHeight(dp(50));
        button.setPadding(dp(12), 0, dp(12), 0);
        button.setBackground(round(filled ? TEAL : PANEL_LIGHT, 14));
        button.setOnClickListener(listener);
        button.setContentDescription(title);
        return button;
    }

    private GradientDrawable round(int color, int radius) {
        GradientDrawable drawable = new GradientDrawable();
        drawable.setColor(color);
        drawable.setCornerRadius(dp(radius));
        return drawable;
    }

    private LinearLayout.LayoutParams margins(int left, int top, int right, int bottom) {
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, -2);
        params.setMargins(dp(left), dp(top), dp(right), dp(bottom));
        return params;
    }

    private String formatBytes(long bytes) {
        if (bytes < 1024) return bytes + " B";
        if (bytes < 1024 * 1024) return (bytes / 1024) + " KB";
        return (bytes / (1024 * 1024)) + " MB";
    }

    private int dp(int value) {
        return Math.round(value * getResources().getDisplayMetrics().density);
    }
}
