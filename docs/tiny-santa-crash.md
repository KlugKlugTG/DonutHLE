# Tiny Santa runtime crash

The activation descriptor is `VI(I)->V`, and Hashtable dispatch is implemented on `main`.

The remaining first-frame failure is in `Lcom/a/a/f/a;->a(Landroid/graphics/Canvas;)V`: the app reads a 19-entry `[[[Bitmap` resource array with an invalid index (`222397737`). This indicates a VM/resource initialization or numeric-width issue, not a missing activation method.

Reproduction with the supplied APK:

```text
Dalvik VM error at pc 172 opcode 0x46: array index 222397737 out of bounds for length 19
```

The crash must be fixed before calling an APK playable. The emulator should log unsupported framework methods rather than substituting a demo frame.
