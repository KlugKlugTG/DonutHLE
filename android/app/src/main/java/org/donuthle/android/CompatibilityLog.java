package org.donuthle.android;

import android.content.Context;

import java.util.LinkedHashSet;
import java.util.Set;

final class CompatibilityLog {
    private CompatibilityLog() {}

    static void sessionStart(Context context, String fileName) {
        StorageLayout.appendLog(context, "SESSION_START: " + fileName);
        StorageLayout.appendLog(context, "UNIMPLEMENTED: Android framework API bridge (Context/Activity lifecycle)");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: Dalvik 035 interpreter and Java method execution");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: Android binary XML resource decoding");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: resources.arsc and drawable/resource table loading");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: GLES 1.x graphics backend");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: audio mixer/backend");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: Android input/event bridge");
        StorageLayout.appendLog(context, "UNIMPLEMENTED: Activity launch and package class loader");
    }

    static void apkInventory(Context context, String fileName) {
        StorageLayout.appendLog(context, "APK_SCAN: " + fileName);
        StorageLayout.appendLog(context, "REQUESTED: package manifest, classes.dex, resources.arsc and native libraries");
    }

    static void record(Context context, String category, String detail) {
        StorageLayout.appendLog(context, "REQUESTED[" + category + "]: " + detail);
        StorageLayout.appendLog(context, "UNIMPLEMENTED[" + category + "]: compatibility handler is not implemented");
    }

    static void unsupported(Context context, String feature) {
        StorageLayout.appendLog(context, "UNIMPLEMENTED: " + feature);
    }
}
