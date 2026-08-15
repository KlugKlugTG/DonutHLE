package org.donuthle.android;

import android.content.Context;

final class CompatFeatures {
    private CompatFeatures() {}

    static void recordImplemented(Context context) {
        String[] features = {
                "Rust core is linked through the Android native bridge",
                "AXML manifest and resources.arsc decoding",
                "Dalvik DEX 035 VM and Android framework shim",
                "software framebuffer, GLES rasterizer, audio queue, and input queue"
        };
        for (String feature : features) StorageLayout.appendLog(context, "IMPLEMENTED: " + feature);
        StorageLayout.appendLog(context, "STATUS: compatibility is app-dependent; unsupported calls are reported during launch");
    }
}
