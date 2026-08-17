package org.donuthle.android;

import android.content.Context;
import android.opengl.GLSurfaceView;

import javax.microedition.khronos.egl.EGLConfig;
import javax.microedition.khronos.opengles.GL10;

final class Gles1SurfaceView extends GLSurfaceView {
    private final MainActivity activity;

    Gles1SurfaceView(Context context, MainActivity activity) {
        super(context);
        this.activity = activity;
        setEGLContextClientVersion(1);
        setRenderer(new Renderer());
        setRenderMode(RENDERMODE_CONTINUOUSLY);
    }

    private final class Renderer implements GLSurfaceView.Renderer {
        @Override public void onSurfaceCreated(GL10 gl, EGLConfig config) {
            activity.nativeRenderFrame(1, 1);
        }

        @Override public void onSurfaceChanged(GL10 gl, int width, int height) {
            activity.nativeRenderFrame(width, height);
        }

        @Override public void onDrawFrame(GL10 gl) {
            activity.nativeRenderFrame(getWidth(), getHeight());
        }
    }
}
