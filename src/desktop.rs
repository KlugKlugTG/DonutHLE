use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::mem::MaybeUninit;
use std::ptr;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::runtime::Runtime;

#[repr(C)]
struct XEvent {
    type_: c_int,
    pad: [c_long; 24],
}

#[repr(C)]
struct XClientMessageEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut c_void,
    window: c_ulong,
    message_type: *mut c_void,
    format: c_int,
    data: [c_long; 5],
}

#[repr(C)]
struct XImage {
    width: c_int,
    height: c_int,
    xoffset: c_int,
    format: c_int,
    data: *mut c_char,
    byte_order: c_int,
    bitmap_unit: c_int,
    bitmap_bit_order: c_int,
    bitmap_pad: c_int,
    depth: c_int,
    bytes_per_line: c_int,
    bits_per_pixel: c_int,
    red_mask: c_ulong,
    green_mask: c_ulong,
    blue_mask: c_ulong,
    obdata: *mut c_char,
    funcs: [*mut c_void; 5],
}

#[link(name = "X11")]
unsafe extern "C" {
    fn XOpenDisplay(name: *const c_char) -> *mut c_void;
    fn XCloseDisplay(display: *mut c_void) -> c_int;
    fn XDefaultScreen(display: *mut c_void) -> c_int;
    fn XRootWindow(display: *mut c_void, screen: c_int) -> c_ulong;
    fn XCreateSimpleWindow(
        display: *mut c_void,
        parent: c_ulong,
        x: c_int,
        y: c_int,
        width: c_uint,
        height: c_uint,
        border_width: c_uint,
        border: c_ulong,
        background: c_ulong,
    ) -> c_ulong;
    fn XSelectInput(display: *mut c_void, window: c_ulong, mask: c_long) -> c_int;
    fn XMapWindow(display: *mut c_void, window: c_ulong) -> c_int;
    fn XStoreName(display: *mut c_void, window: c_ulong, name: *const c_char) -> c_int;
    fn XInternAtom(display: *mut c_void, name: *const c_char, only_if_exists: c_int) -> c_ulong;
    fn XSetWMProtocols(
        display: *mut c_void,
        window: c_ulong,
        protocols: *mut c_ulong,
        count: c_int,
    ) -> c_int;
    fn XPending(display: *mut c_void) -> c_int;
    fn XNextEvent(display: *mut c_void, event: *mut XEvent) -> c_int;
    fn XDestroyWindow(display: *mut c_void, window: c_ulong) -> c_int;
    fn XCreateImage(
        display: *mut c_void,
        visual: *mut c_void,
        depth: c_uint,
        format: c_int,
        offset: c_int,
        data: *mut c_char,
        width: c_uint,
        height: c_uint,
        bitmap_pad: c_int,
        bytes_per_line: c_int,
    ) -> *mut XImage;
    fn XPutImage(
        display: *mut c_void,
        drawable: c_ulong,
        gc: *mut c_void,
        image: *mut XImage,
        src_x: c_int,
        src_y: c_int,
        dest_x: c_int,
        dest_y: c_int,
        width: c_uint,
        height: c_uint,
    ) -> c_int;
    fn XDestroyImage(image: *mut XImage) -> c_int;
    fn XFlush(display: *mut c_void) -> c_int;
    fn XDefaultVisual(display: *mut c_void, screen: c_int) -> *mut c_void;
    fn XDefaultDepth(display: *mut c_void, screen: c_int) -> c_int;
    fn XDefaultGC(display: *mut c_void, screen: c_int) -> *mut c_void;
}

const EXPOSURE_MASK: c_long = 1 << 15;
const KEY_PRESS_MASK: c_long = 1 << 0;
const STRUCTURE_NOTIFY_MASK: c_long = 1 << 17;
const CLIENT_MESSAGE: c_int = 33;
const ZPIXMAP: c_int = 2;

pub fn present(mut runtime: Runtime) -> Result<()> {
    let display = unsafe { XOpenDisplay(ptr::null()) };
    if display.is_null() {
        anyhow::bail!(
            "cannot open X11 display; use `validate` or run under a graphical Linux session"
        );
    }
    let result = present_window(display, &mut runtime);
    unsafe {
        XCloseDisplay(display);
    }
    result
}

fn present_window(display: *mut c_void, runtime: &mut Runtime) -> Result<()> {
    let screen = unsafe { XDefaultScreen(display) };
    let root = unsafe { XRootWindow(display, screen) };
    let width = runtime.config.screen.width;
    let height = runtime.config.screen.height;
    let window = unsafe { XCreateSimpleWindow(display, root, 0, 0, width, height, 0, 0, 0) };
    if window == 0 {
        anyhow::bail!("cannot create X11 window");
    }
    let title = c"DonutHLE - Linux";
    unsafe {
        XStoreName(display, window, title.as_ptr());
        XSelectInput(
            display,
            window,
            EXPOSURE_MASK | KEY_PRESS_MASK | STRUCTURE_NOTIFY_MASK,
        );
        XMapWindow(display, window);
    }
    let wm_delete = c"WM_DELETE_WINDOW";
    let atom = unsafe { XInternAtom(display, wm_delete.as_ptr(), 0) };
    unsafe {
        XSetWMProtocols(display, window, &atom as *const c_ulong as *mut c_ulong, 1);
    }

    let start = Instant::now();
    let mut running = true;
    while running && start.elapsed() < Duration::from_secs(30) {
        while unsafe { XPending(display) } > 0 {
            let mut event = MaybeUninit::<XEvent>::zeroed();
            unsafe { XNextEvent(display, event.as_mut_ptr()) };
            let event = unsafe { event.assume_init() };
            if event.type_ == CLIENT_MESSAGE {
                let client = unsafe { &*((&event as *const XEvent).cast::<XClientMessageEvent>()) };
                if client.data[0] as c_ulong == atom {
                    running = false;
                }
            }
        }
        let session = runtime
            .session
            .as_mut()
            .context("runtime session was not created")?;
        session.render_current_frame()?;
        let framebuffer = session.vm.framework.gles.framebuffer();
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for pixel in framebuffer.pixels() {
            pixels.push(u32::from(pixel.r) << 16 | u32::from(pixel.g) << 8 | u32::from(pixel.b));
        }
        let image_data = pixels.as_mut_ptr().cast::<c_char>();
        let visual = unsafe { XDefaultVisual(display, screen) };
        let depth = unsafe { XDefaultDepth(display, screen) };
        let image = unsafe {
            XCreateImage(
                display,
                visual,
                depth as c_uint,
                ZPIXMAP,
                0,
                image_data,
                width,
                height,
                32,
                0,
            )
        };
        if image.is_null() {
            anyhow::bail!("cannot create X11 image");
        }
        unsafe {
            XPutImage(
                display,
                window,
                XDefaultGC(display, screen),
                image,
                0,
                0,
                0,
                0,
                width,
                height,
            );
            (*image).data = ptr::null_mut();
            XDestroyImage(image);
            XFlush(display);
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    unsafe {
        XDestroyWindow(display, window);
    }
    Ok(())
}
