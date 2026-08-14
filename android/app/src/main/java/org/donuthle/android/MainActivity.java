package org.donuthle.android;

import android.app.Activity;
import android.content.Intent;
import android.graphics.Color;
import android.net.Uri;
import android.os.Bundle;
import android.provider.Settings;
import android.view.Gravity;
import android.view.View;
import android.view.Window;
import android.view.WindowManager;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import java.io.File;
import java.util.Locale;

public final class MainActivity extends Activity {
    static {
        System.loadLibrary("donuthle");
    }

    private native String nativeRuntimeInfo();

    private int teal = Color.rgb(128, 203, 196);
    private int text = Color.rgb(232, 240, 242);
    private int muted = Color.rgb(158, 174, 180);
    private int panel = Color.rgb(27, 33, 38);
    private int background = Color.rgb(14, 18, 22);

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        requestWindowFeature(Window.FEATURE_NO_TITLE);
        getWindow().setFlags(WindowManager.LayoutParams.FLAG_FULLSCREEN, WindowManager.LayoutParams.FLAG_FULLSCREEN);
        StorageLayout.ensure(this);
        StorageLayout.appendLog(this, "DonutHLE main menu opened");
        showHome();
    }

    private void showHome() {
        LinearLayout content = column();
        content.setPadding(dp(22), dp(20), dp(22), dp(28));

        TextView brand = label("DONUTHLE", 27, teal);
        brand.setTypeface(null, 1);
        content.addView(brand);
        TextView subtitle = label("Android 1.6 • Donut • HLE prototype", 14, muted);
        content.addView(subtitle, margins(0, 2, 0, 22));

        TextView status = label("READY TO EXPLORE", 12, Color.rgb(112, 214, 164));
        status.setTypeface(null, 1);
        content.addView(status);
        TextView title = label("Your games,\nyour sandbox.", 31, text);
        title.setTypeface(null, 1);
        content.addView(title, margins(0, 5, 0, 8));
        content.addView(label("Place legal APK files in DonutHLE_apps. Each game gets its own writable data folder and log.", 15, muted), margins(0, 0, 0, 22));

        content.addView(card("▣", "APK LIBRARY", "Install and manage Android 1.6 games", "Open file storage", v -> openFiles()), margins(0, 0, 0, 10));
        content.addView(card("⌁", "GAME SANDBOX", "Browse save data and per-game files", "Open sandbox", v -> openFolder(StorageLayout.sandbox(this))), margins(0, 0, 0, 10));
        content.addView(card("≡", "EMULATOR LOG", "Inspect startup and compatibility reports", "View log", v -> showLog()), margins(0, 0, 0, 10));

        TextView paths = label("FILES LIVE IN\n" + StorageLayout.root(this).getAbsolutePath(), 12, muted);
        paths.setPadding(dp(14), dp(13), dp(14), dp(13));
        paths.setBackgroundColor(panel);
        content.addView(paths, margins(0, 12, 0, 18));

        Button options = button("OPTIONS  ›", false);
        options.setOnClickListener(v -> openOptions());
        content.addView(options, margins(0, 0, 0, 8));
        Button about = button("ABOUT DONUTHLE  ›", false);
        about.setOnClickListener(v -> showAbout());
        content.addView(about);
        setContentView(scroll(content));
    }

    private View card(String icon, String heading, String description, String action, View.OnClickListener listener) {
        LinearLayout row = new LinearLayout(this);
        row.setOrientation(LinearLayout.HORIZONTAL);
        row.setGravity(Gravity.CENTER_VERTICAL);
        row.setPadding(dp(14), dp(14), dp(12), dp(14));
        row.setBackgroundColor(panel);
        row.setOnClickListener(listener);

        TextView symbol = label(icon, 28, teal);
        symbol.setGravity(Gravity.CENTER);
        row.addView(symbol, new LinearLayout.LayoutParams(dp(40), dp(60)));
        LinearLayout copy = column();
        copy.setPadding(dp(12), 0, dp(8), 0);
        TextView h = label(heading, 13, teal);
        h.setTypeface(null, 1);
        copy.addView(h);
        copy.addView(label(description, 15, text), margins(0, 3, 0, 0));
        row.addView(copy, new LinearLayout.LayoutParams(0, -2, 1));
        TextView arrow = label("›", 28, muted);
        row.addView(arrow);
        return row;
    }

    private void openFiles() {
        StorageLayout.appendLog(this, "Opened APK library");
        openFolder(StorageLayout.apps(this));
    }

    private void openFolder(File folder) {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("*/*");
        try {
            startActivity(intent);
        } catch (Exception error) {
            startActivity(new Intent(Settings.ACTION_INTERNAL_STORAGE_SETTINGS));
        }
    }

    private void showLog() {
        StorageLayout.appendLog(this, "Opened emulator log");
        LinearLayout content = page("EMULATOR LOG", "A readable trace of the current Android shell.");
        TextView log = label(nativeRuntimeInfo() + "\n\nLog path:\n" + new File(StorageLayout.logs(this), "donuthle.log").getAbsolutePath(), 15, text);
        log.setTextIsSelectable(true);
        log.setPadding(dp(14), dp(14), dp(14), dp(14));
        log.setBackgroundColor(panel);
        content.addView(log, margins(0, 16, 0, 16));
        content.addView(button("‹  BACK", true, v -> showHome()));
        setContentView(scroll(content));
    }

    private void showAbout() {
        LinearLayout content = page("ABOUT DONUTHLE", "A clean-room Android 1.6 HLE project.");
        content.addView(label("The goal is to reproduce the small platform surface used by historical games without distributing Android system images or copyrighted game files.\n\nThis build is an Android shell. Dalvik execution, graphics, audio, input, and APK launch are being developed in stages.", 16, text), margins(0, 18, 0, 20));
        content.addView(button("‹  BACK", true, v -> showHome()));
        setContentView(scroll(content));
    }

    private void openOptions() {
        Intent intent = new Intent(Intent.ACTION_VIEW);
        intent.setData(Uri.fromFile(StorageLayout.options(this)));
        try {
            startActivity(intent);
        } catch (Exception error) {
            showAbout();
        }
    }

    private LinearLayout page(String heading, String description) {
        LinearLayout content = column();
        content.setPadding(dp(22), dp(20), dp(22), dp(28));
        content.addView(label("DONUTHLE", 22, teal));
        TextView h = label(heading, 28, text);
        h.setTypeface(null, 1);
        content.addView(h, margins(0, 28, 0, 4));
        content.addView(label(description, 15, muted));
        return content;
    }

    private LinearLayout column() {
        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        layout.setBackgroundColor(background);
        return layout;
    }

    private ScrollView scroll(View child) {
        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        scroll.setBackgroundColor(background);
        scroll.addView(child);
        return scroll;
    }

    private TextView label(String value, int size, int color) {
        TextView view = new TextView(this);
        view.setText(value);
        view.setTextSize(size);
        view.setTextColor(color);
        return view;
    }

    private Button button(String title, boolean filled) {
        return button(title, filled, null);
    }

    private Button button(String title, boolean filled, View.OnClickListener listener) {
        Button button = new Button(this);
        button.setText(title);
        button.setTextSize(13);
        button.setTextColor(filled ? background : teal);
        button.setAllCaps(false);
        button.setGravity(Gravity.CENTER);
        button.setMinHeight(dp(48));
        button.setBackgroundColor(filled ? teal : panel);
        if (listener != null) button.setOnClickListener(listener);
        return button;
    }

    private LinearLayout.LayoutParams margins(int left, int top, int right, int bottom) {
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, -2);
        params.setMargins(dp(left), dp(top), dp(right), dp(bottom));
        return params;
    }

    private int dp(int value) {
        return Math.round(value * getResources().getDisplayMetrics().density);
    }
}
