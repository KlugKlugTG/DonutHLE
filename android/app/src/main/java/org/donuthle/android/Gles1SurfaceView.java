package org.donuthle.android;

import android.content.Context;
import android.opengl.GLSurfaceView;
import android.view.MotionEvent;

import javax.microedition.khronos.egl.EGLConfig;
import javax.microedition.khronos.opengles.GL10;

final class Gles1SurfaceView extends GLSurfaceView {
    private final MainActivity activity;

    Gles1SurfaceView(Context context, MainActivity activity) {
        super(context);
        this.activity = activity;
        setEGLContextClientVersion(2);
        setEGLConfigChooser(8, 8, 8, 8, 16, 0);
        setRenderer(new Renderer());
        setRenderMode(RENDERMODE_CONTINUOUSLY);
    }

    @Override public boolean onTouchEvent(MotionEvent event) {
        int width = Math.max(1, getWidth());
        int height = Math.max(1, getHeight());
        float x = event.getX() * 320.0f / width;
        float y = event.getY() * 480.0f / height;
        activity.nativeTouchEvent(event.getActionMasked(), x, y);
        return true;
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
