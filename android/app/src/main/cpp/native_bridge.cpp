#include <jni.h>
#include <string>

extern "C" const char* donuthle_core_info();
extern "C" char* donuthle_launch_report(const char* path);
extern "C" void donuthle_free_string(char* value);

static jstring makeString(JNIEnv* env, const char* value) {
    return env->NewStringUTF(value == nullptr ? "DonutHLE core unavailable" : value);
}

extern "C" JNIEXPORT jstring JNICALL
Java_org_donuthle_android_MainActivity_nativeRuntimeInfo(JNIEnv* env, jobject) {
#ifdef DONUTHLE_NO_CORE
    return makeString(env, "Android shell is built; Rust core is not linked in this local build.");
#else
    return makeString(env, donuthle_core_info());
#endif
}

extern "C" JNIEXPORT jstring JNICALL
Java_org_donuthle_android_MainActivity_nativeLaunchApk(JNIEnv* env, jobject, jstring path) {
#ifdef DONUTHLE_NO_CORE
    return makeString(env, "Rust core is not linked in this local build.");
#else
    if (path == nullptr) return makeString(env, "Runtime error: APK path is null");
    const char* utfPath = env->GetStringUTFChars(path, nullptr);
    if (utfPath == nullptr) return makeString(env, "Runtime error: cannot read APK path");
    char* report = donuthle_launch_report(utfPath);
    jstring result = makeString(env, report);
    env->ReleaseStringUTFChars(path, utfPath);
    donuthle_free_string(report);
    return result;
#endif
}
