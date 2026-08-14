#include <jni.h>
#include <string>

extern "C" JNIEXPORT jstring JNICALL
Java_org_donuthle_android_MainActivity_nativeRuntimeInfo(JNIEnv* env, jobject) {
    const std::string text =
        "DonutHLE Android prototype\\n\\n"
        "Target: Android 1.6 Donut / API 4\\n"
        "Host: Android native shell\\n\\n"
        "The Android build is working.\\n"
        "APK loading, Dalvik execution, graphics, audio, and input are next milestones.";
    return env->NewStringUTF(text.c_str());
}
