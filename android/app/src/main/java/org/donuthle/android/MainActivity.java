package org.donuthle.android;

import android.app.Activity;
import android.os.Bundle;
import android.view.Window;
import android.view.WindowManager;
import android.widget.TextView;

public final class MainActivity extends Activity {
    static {
        System.loadLibrary("donuthle");
    }

    private native String nativeRuntimeInfo();

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        requestWindowFeature(Window.FEATURE_NO_TITLE);
        getWindow().setFlags(
                WindowManager.LayoutParams.FLAG_FULLSCREEN,
                WindowManager.LayoutParams.FLAG_FULLSCREEN);

        TextView view = new TextView(this);
        view.setText(nativeRuntimeInfo());
        view.setTextColor(0xffe8f0f2);
        view.setTextSize(18);
        view.setPadding(32, 32, 32, 32);
        view.setBackgroundColor(0xff101418);
        setContentView(view);
    }
}
