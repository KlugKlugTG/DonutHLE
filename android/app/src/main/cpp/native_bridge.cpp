#include <GLES/gl.h>
#include <jni.h>
#include <cmath>
#include <cstdint>
#include <string>

extern "C" const char* donuthle_core_info();
extern "C" char* donuthle_launch_report(const char* path);
extern "C" void donuthle_free_string(char* value);
#ifndef DONUTHLE_NO_CORE
extern "C" uint32_t donuthle_gles1_next_frame();
#endif

static uint32_t nextFrame() {
#ifdef DONUTHLE_NO_CORE
    static uint32_t frame = 0;
    return ++frame;
#else
    return donuthle_gles1_next_frame();
#endif
}


static void drawRect(GLfloat left, GLfloat top, GLfloat right, GLfloat bottom, GLfloat red, GLfloat green, GLfloat blue, GLfloat alpha) {
    if (right <= left || bottom <= top) return;
    const GLfloat vertices[] = {left, top, right, top, left, bottom, right, bottom};
    glColor4f(red, green, blue, alpha);
    glVertexPointer(2, GL_FLOAT, 0, vertices);
    glEnableClientState(GL_VERTEX_ARRAY);
    glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    glDisableClientState(GL_VERTEX_ARRAY);
}

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


extern "C" JNIEXPORT void JNICALL
Java_org_donuthle_android_MainActivity_nativeRenderFrame(JNIEnv*, jobject, jint width, jint height) {
    const uint32_t frame = nextFrame();
    const GLfloat pulse = 0.5f + 0.5f * std::sin(static_cast<GLfloat>(frame) * 0.04f);
    const GLfloat safeWidth = width > 0 ? static_cast<GLfloat>(width) : 1.0f;
    const GLfloat safeHeight = height > 0 ? static_cast<GLfloat>(height) : 1.0f;
    glViewport(0, 0, static_cast<GLsizei>(safeWidth), static_cast<GLsizei>(safeHeight));
    glClearColor(0.035f, 0.055f, 0.07f, 1.0f);
    glClear(GL_COLOR_BUFFER_BIT);
    glMatrixMode(GL_PROJECTION);
    glLoadIdentity();
    glOrthof(0.0f, safeWidth, safeHeight, 0.0f, -1.0f, 1.0f);
    glMatrixMode(GL_MODELVIEW);
    glLoadIdentity();
    glDisable(GL_DEPTH_TEST);
    glEnable(GL_BLEND);
    glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
    drawRect(20.0f, 28.0f, safeWidth - 20.0f, 30.0f, 0.12f, 0.19f, 0.23f, 1.0f);
    drawRect(20.0f, 28.0f, 20.0f + (safeWidth - 40.0f) * (0.6f + pulse * 0.35f), 30.0f, 0.50f, 0.80f, 0.77f, 1.0f);
    const GLfloat cardLeft = safeWidth * 0.12f;
    const GLfloat cardRight = safeWidth * 0.88f;
    const GLfloat cardTop = safeHeight * 0.28f;
    const GLfloat cardBottom = safeHeight * 0.72f;
    drawRect(cardLeft, cardTop, cardRight, cardBottom, 0.08f, 0.12f, 0.15f, 1.0f);
    drawRect(cardLeft + 18.0f, cardTop + 18.0f, cardRight - 18.0f, cardTop + 21.0f, 0.50f, 0.80f, 0.77f, 1.0f);
    drawRect(cardLeft + 18.0f, cardBottom - 32.0f, cardLeft + 18.0f + (cardRight - cardLeft - 36.0f) * (0.45f + pulse * 0.25f), cardBottom - 26.0f, 0.31f, 0.52f, 0.48f, 1.0f);
    drawRect(safeWidth * 0.40f, safeHeight * 0.48f, safeWidth * 0.60f, safeHeight * 0.56f, 0.50f, 0.80f, 0.77f, 0.92f);
    glDisable(GL_BLEND);
}
