package org.donuthle.android;

import android.content.Context;

import java.io.File;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Map;
import java.util.Set;

final class CompatFeatures {
    private static final Map<String, String> MARKERS = new HashMap<>();

    static {
        MARKERS.put("android/opengl", "GLES 1.x graphics backend");
        MARKERS.put("javax/microedition/khronos", "OpenGL ES EGL/Khronos bridge");
        MARKERS.put("android/media/AudioTrack", "AudioTrack PCM mixer");
        MARKERS.put("android/media/MediaPlayer", "MediaPlayer backend");
        MARKERS.put("android/media/SoundPool", "SoundPool effects backend");
        MARKERS.put("android/view/SurfaceView", "SurfaceView and framebuffer bridge");
        MARKERS.put("android/view/SurfaceHolder", "Surface lifecycle and buffer queue");
        MARKERS.put("android/database/sqlite", "SQLite compatibility layer");
        MARKERS.put("SharedPreferences", "SharedPreferences persistence");
        MARKERS.put("android/os/Looper", "Looper/Handler message queues");
        MARKERS.put("android/hardware/Sensor", "sensor compatibility layer");
        MARKERS.put("android/hardware/Camera", "camera compatibility layer");
        MARKERS.put("android/location", "location services");
        MARKERS.put("android/bluetooth", "Bluetooth services");
        MARKERS.put("android/webkit", "WebView compatibility layer");
        MARKERS.put("android/net/", "Android network services");
        MARKERS.put("android/view/MotionEvent", "touch and motion input bridge");
        MARKERS.put("System.loadLibrary", "JNI/native library loader");
        MARKERS.put("dalvik/system", "Dalvik system APIs");
    }

    private CompatFeatures() {}

    static Set<String> detect(byte[] dexBytes) {
        String dex = new String(dexBytes, java.nio.charset.StandardCharsets.ISO_8859_1);
        Set<String> result = new HashSet<>();
        for (Map.Entry<String, String> marker : MARKERS.entrySet()) {
            if (dex.contains(marker.getKey())) result.add(marker.getValue());
        }
        return result;
    }

    static void log(Context context, String feature) {
        StorageLayout.appendLog(context, "UNIMPLEMENTED: " + feature);
    }

    static void logAll(Context context, Set<String> features) {
        for (String feature : features) log(context, feature);
    }
}
