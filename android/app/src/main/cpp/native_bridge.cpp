#include <GLES/gl.h>
#include <jni.h>
#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <string>
#include <vector>

extern "C" const char* donuthle_core_info();
extern "C" char* donuthle_launch_report(const char* path);
extern "C" void donuthle_free_string(char* value);
extern "C" char* donuthle_game_title();
extern "C" uint32_t donuthle_framebuffer_width();
extern "C" uint32_t donuthle_framebuffer_height();
extern "C" int32_t donuthle_touch(int32_t action, float x, float y);
#ifndef DONUTHLE_NO_CORE
extern "C" uint32_t donuthle_render_frame(uint32_t width, uint32_t height);
extern "C" size_t donuthle_framebuffer_copy(uint8_t* output, size_t output_len);
#endif

static uint32_t nextFrame() {
#ifdef DONUTHLE_NO_CORE
    static uint32_t frame = 0;
    return ++frame;
#else
    return 0;
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
Java_org_donuthle_android_MainActivity_nativeGameTitle(JNIEnv* env, jobject) {
#ifdef DONUTHLE_NO_CORE
    return makeString(env, "Unknown game");
#else
    char* title = donuthle_game_title();
    jstring result = makeString(env, title);
    donuthle_free_string(title);
    return result;
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


#ifndef DONUTHLE_NO_CORE
static uint32_t nextPowerOfTwo(uint32_t value) {
    uint32_t result = 1;
    while (result < value && result < 4096) result <<= 1;
    return result;
}

static void drawFallbackFrame(GLfloat width, GLfloat height) {
    glViewport(0, 0, static_cast<GLsizei>(width), static_cast<GLsizei>(height));
    glDisable(GL_TEXTURE_2D);
    glDisable(GL_DEPTH_TEST);
    glDisable(GL_CULL_FACE);
    glDisable(GL_SCISSOR_TEST);
    glDisable(GL_BLEND);
    glClearColor(0.035f, 0.055f, 0.07f, 1.0f);
    glClear(GL_COLOR_BUFFER_BIT);
    glMatrixMode(GL_PROJECTION);
    glLoadIdentity();
    glOrthof(0.0f, width, height, 0.0f, -1.0f, 1.0f);
    glMatrixMode(GL_MODELVIEW);
    glLoadIdentity();
}

static bool drawSoftwareFrame(GLfloat width, GLfloat height) {
    const uint32_t frameWidth = donuthle_framebuffer_width();
    const uint32_t frameHeight = donuthle_framebuffer_height();
    if (frameWidth == 0 || frameHeight == 0) return false;

    const uint32_t textureWidth = nextPowerOfTwo(frameWidth);
    const uint32_t textureHeight = nextPowerOfTwo(frameHeight);
    std::vector<uint8_t> pixels(static_cast<size_t>(textureWidth) * textureHeight * 4u, 0);
    const size_t sourceLength = static_cast<size_t>(frameWidth) * frameHeight * 4u;
    std::vector<uint8_t> source(sourceLength);
    if (donuthle_framebuffer_copy(source.data(), source.size()) != source.size()) return false;
    for (uint32_t row = 0; row < frameHeight; ++row) {
        const size_t sourceOffset = static_cast<size_t>(row) * frameWidth * 4u;
        const size_t destinationOffset = static_cast<size_t>(row) * textureWidth * 4u;
        const size_t rowBytes = static_cast<size_t>(frameWidth) * 4u;
        std::memcpy(pixels.data() + destinationOffset, source.data() + sourceOffset, rowBytes);
    }

    static GLuint texture = 0;
    if (texture == 0) glGenTextures(1, &texture);
    glViewport(0, 0, static_cast<GLsizei>(width), static_cast<GLsizei>(height));
    glDisable(GL_SCISSOR_TEST);
    glDisable(GL_CULL_FACE);
    glDisable(GL_DEPTH_TEST);
    glDisable(GL_BLEND);
    glDisable(GL_LIGHTING);
    glDisable(GL_FOG);
    glDisable(GL_ALPHA_TEST);
    glDisableClientState(GL_COLOR_ARRAY);
    glDisableClientState(GL_NORMAL_ARRAY);
    glBindTexture(GL_TEXTURE_2D, texture);
    glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
    glTexEnvf(GL_TEXTURE_ENV, GL_TEXTURE_ENV_MODE, GL_REPLACE);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, static_cast<GLsizei>(textureWidth), static_cast<GLsizei>(textureHeight), 0, GL_RGBA, GL_UNSIGNED_BYTE, pixels.data());

    const GLfloat vertices[] = {0.0f, 0.0f, width, 0.0f, 0.0f, height, width, height};
    const GLfloat u = static_cast<GLfloat>(frameWidth) / textureWidth;
    const GLfloat v = static_cast<GLfloat>(frameHeight) / textureHeight;
    const GLfloat coordinates[] = {0.0f, 0.0f, u, 0.0f, 0.0f, v, u, v};
    glMatrixMode(GL_PROJECTION);
    glLoadIdentity();
    glOrthof(0.0f, width, height, 0.0f, -1.0f, 1.0f);
    glMatrixMode(GL_MODELVIEW);
    glLoadIdentity();
    glEnable(GL_TEXTURE_2D);
    glColor4f(1.0f, 1.0f, 1.0f, 1.0f);
    glEnableClientState(GL_VERTEX_ARRAY);
    glEnableClientState(GL_TEXTURE_COORD_ARRAY);
    glVertexPointer(2, GL_FLOAT, 0, vertices);
    glTexCoordPointer(2, GL_FLOAT, 0, coordinates);
    glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    glDisableClientState(GL_TEXTURE_COORD_ARRAY);
    glDisableClientState(GL_VERTEX_ARRAY);
    glDisable(GL_TEXTURE_2D);
    return true;
}
#endif

extern "C" JNIEXPORT jint JNICALL
Java_org_donuthle_android_MainActivity_nativeTouchEvent(JNIEnv*, jobject, jint action, jfloat x, jfloat y) {
#ifdef DONUTHLE_NO_CORE
    return 0;
#else
    return static_cast<jint>(donuthle_touch(static_cast<int32_t>(action), x, y));
#endif
}

extern "C" JNIEXPORT void JNICALL
Java_org_donuthle_android_MainActivity_nativeRenderFrame(JNIEnv*, jobject, jint width, jint height) {
    const uint32_t frame = nextFrame();
    const GLfloat safeWidth = width > 0 ? static_cast<GLfloat>(width) : 1.0f;
    const GLfloat safeHeight = height > 0 ? static_cast<GLfloat>(height) : 1.0f;
#ifdef DONUTHLE_NO_CORE
    const GLfloat pulse = 0.5f + 0.5f * std::sin(static_cast<GLfloat>(frame) * 0.04f);
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
    drawRect(20.0f, 28.0f, 20.0f + (safeWidth - 40.0f) * (0.6f + 0.35f * std::sin(static_cast<GLfloat>(frame) * 0.04f)), 30.0f, 0.50f, 0.80f, 0.77f, 1.0f);
    const GLfloat cardLeft = safeWidth * 0.12f;
    const GLfloat cardRight = safeWidth * 0.88f;
    const GLfloat cardTop = safeHeight * 0.28f;
    const GLfloat cardBottom = safeHeight * 0.72f;
    drawRect(cardLeft, cardTop, cardRight, cardBottom, 0.08f, 0.12f, 0.15f, 1.0f);
    drawRect(cardLeft + 18.0f, cardTop + 18.0f, cardRight - 18.0f, cardTop + 21.0f, 0.50f, 0.80f, 0.77f, 1.0f);
    drawRect(cardLeft + 18.0f, cardBottom - 32.0f, cardLeft + 18.0f + (cardRight - cardLeft - 36.0f) * (0.45f + 0.25f * std::sin(static_cast<GLfloat>(frame) * 0.04f)), cardBottom - 26.0f, 0.31f, 0.52f, 0.48f, 1.0f);
    drawRect(safeWidth * 0.40f, safeHeight * 0.48f, safeWidth * 0.60f, safeHeight * 0.56f, 0.50f, 0.80f, 0.77f, 0.92f);
    glDisable(GL_BLEND);
#else
    donuthle_render_frame(static_cast<uint32_t>(safeWidth), static_cast<uint32_t>(safeHeight));
    if (!drawSoftwareFrame(safeWidth, safeHeight)) drawFallbackFrame(safeWidth, safeHeight);
#endif
    (void)frame;
}
