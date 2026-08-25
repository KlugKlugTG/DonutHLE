# Architecture notes

## HLE layers

```text
APK / resources.arsc
        |
        v
loader -> manifest/package model -> launch plan
        |
        v
Dalvik 035 interpreter <-> heap / class registry
        |
        v
Android 1.x-2.x API shims (API 1–8)
  Activity + Context + View + Looper + services
        |
        +--> GLES 1.x API
                |
                v
          GLES1 compatibility adapter
          fixed-point conversion / client arrays / palette skinning
                |
                v
          software rasterizer + GLES 2.0 Android presentation framebuffer
        +--> input
        +--> audio
        +--> host filesystem and network policy
```

The prototype keeps host integration behind small interfaces so that the core can be tested without a window or an Android system image.

## First compatibility target

The first game target should be selected from a legally obtained APK and should be simple enough to expose missing APIs without needing a full Play Services stack. The compatibility harness should record lifecycle calls, DEX methods invoked, resource lookups, GL calls, audio requests, and input events.

## Non-goals

- Booting a full Android kernel or system image.
- Shipping proprietary Android framework files.
- Bypassing licensing, DRM, signature checks, or online services.
- Claiming broad game compatibility before repeatable tests exist.
