#![allow(unsafe_op_in_unsafe_fn)]

use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use crate::types::threat::ThreatState;
use crate::types::overlays::OverlayConfig;

use winapi::shared::windef::{HWND, HBITMAP, POINT, SIZE};
use winapi::shared::minwindef::{UINT, WPARAM, LPARAM, LRESULT};
use winapi::um::winuser::*;
use winapi::um::wingdi::*;
use winapi::um::libloaderapi::GetModuleHandleW;

const IDLE_BOB_MS:    f32 = 1200.0;
const ALERT_SHAKE_MS: f32 = 120.0;

struct OverlayState {
    threat_state: Arc<Mutex<ThreatState>>,
    idle_bmp:     HBITMAP,
    alert_bmp:    HBITMAP,
    width:        i32,
    height:       i32,
    start:        Instant,
}

pub fn run(
    threat_state: Arc<Mutex<ThreatState>>,
    config:       OverlayConfig,
) {
    unsafe {
        let hinstance = GetModuleHandleW(std::ptr::null());

        let class_name = wide("HaroOverlay");
        let wc = WNDCLASSEXW {
            cbSize:        std::mem::size_of::<WNDCLASSEXW>() as u32,
            style:         CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc:   Some(wnd_proc),
            cbClsExtra:    0,
            cbWndExtra:    0,
            hInstance:     hinstance,
            hIcon:         std::ptr::null_mut(),
            hCursor:       LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName:  std::ptr::null_mut(),
            lpszClassName: class_name.as_ptr(),
            hIconSm:       std::ptr::null_mut(),
        };
        RegisterClassExW(&wc);

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            wide("").as_ptr(),
            WS_POPUP | WS_VISIBLE,
            config.x,
            config.y,
            config.width,
            config.height,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null_mut(),
        );

        let idle_bmp  = load_png_as_bitmap("models/character/haro_idle.png",  config.width, config.height);
        let alert_bmp = load_png_as_bitmap("models/character/haro_alert.png", config.width, config.height);

        let state = Box::new(OverlayState {
            threat_state,
            idle_bmp,
            alert_bmp,
            width:  config.width,
            height: config.height,
            start:  Instant::now(),
        });

        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
        SetTimer(hwnd, 1, 16, None);

        let mut msg = std::mem::zeroed::<MSG>();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd:   HWND,
    msg:    UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TIMER => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OverlayState;
            if ptr.is_null() { return 0; }
            let state = &*ptr;

            // hide HARO if Mecha Break is not the focused window
            if !crate::game_load_checker::is_game_focused() {
                ShowWindow(hwnd, SW_HIDE);
                return 0;
            } else {
                // show without stealing focus
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }

            let elapsed_ms = state.start.elapsed().as_millis() as f32;
            let is_alert   = state.threat_state
                .lock()
                .map(|s| s.alert.is_some())
                .unwrap_or(false);

            let offset_y: i32 = if is_alert {
                let phase = (elapsed_ms % ALERT_SHAKE_MS) / ALERT_SHAKE_MS;
                if phase < 0.5 { -8 } else { 8 }
            } else {
                let phase = (elapsed_ms % IDLE_BOB_MS) / IDLE_BOB_MS;
                let sine  = (phase * std::f32::consts::TAU).sin();
                (sine * 5.0) as i32
            };

            let bmp = if is_alert { state.alert_bmp } else { state.idle_bmp };
            draw_layered(hwnd, bmp, state.width, state.height, offset_y);
            0
        }

        WM_KEYDOWN => {
            match wparam as i32 {
                0x1B => PostQuitMessage(0),
                _ => {}
            }
            0
        }

        WM_DESTROY => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OverlayState;
            if !ptr.is_null() {
                let state = Box::from_raw(ptr);
                if !state.idle_bmp.is_null()  { DeleteObject(state.idle_bmp as _); }
                if !state.alert_bmp.is_null() { DeleteObject(state.alert_bmp as _); }
            }
            PostQuitMessage(0);
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn draw_layered(hwnd: HWND, bmp: HBITMAP, w: i32, h: i32, offset_y: i32) {
    let screen_dc = GetDC(std::ptr::null_mut());
    let mem_dc    = CreateCompatibleDC(screen_dc);
    let old_bmp   = SelectObject(mem_dc, bmp as _);

    let mut wnd_pos = POINT { x: 0, y: 0 };
    let mut size    = SIZE { cx: w, cy: h };
    let mut src_pos = POINT { x: 0, y: offset_y.max(0) };

    let mut blend = BLENDFUNCTION {
        BlendOp:             AC_SRC_OVER,
        BlendFlags:          0,
        SourceConstantAlpha: 255,
        AlphaFormat:         AC_SRC_ALPHA,
    };

    let mut rect = std::mem::zeroed::<winapi::shared::windef::RECT>();
    GetWindowRect(hwnd, &mut rect);
    wnd_pos.x = rect.left;
    wnd_pos.y = rect.top;

    UpdateLayeredWindow(
        hwnd,
        screen_dc,
        &mut wnd_pos,
        &mut size,
        mem_dc,
        &mut src_pos,
        0,
        &mut blend,
        ULW_ALPHA,
    );

    SelectObject(mem_dc, old_bmp);
    DeleteDC(mem_dc);
    ReleaseDC(std::ptr::null_mut(), screen_dc);
}

unsafe fn load_png_as_bitmap(path: &str, w: i32, h: i32) -> HBITMAP {
    let img = image::open(path)
        .map(|i| i.resize_exact(w as u32, h as u32, image::imageops::FilterType::Lanczos3))
        .map(|i| i.into_rgba8());

    let rgba = match img {
        Ok(r)  => r,
        Err(_) => return make_placeholder_bitmap(w, h, false),
    };

    let screen_dc = GetDC(std::ptr::null_mut());
    let mem_dc    = CreateCompatibleDC(screen_dc);

    let mut bmi: BITMAPINFO = std::mem::zeroed();
    bmi.bmiHeader.biSize        = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth       = w;
    bmi.bmiHeader.biHeight      = -h;
    bmi.bmiHeader.biPlanes      = 1;
    bmi.bmiHeader.biBitCount    = 32;
    bmi.bmiHeader.biCompression = BI_RGB;

    let mut bits: *mut winapi::ctypes::c_void = std::ptr::null_mut();
    let bmp = CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);

    let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
    for (i, pixel) in rgba.pixels().enumerate() {
        let base  = i * 4;
        let alpha = pixel[3] as u32;
        dst[base]     = (pixel[2] as u32 * alpha / 255) as u8;  // B
        dst[base + 1] = (pixel[1] as u32 * alpha / 255) as u8;  // G
        dst[base + 2] = (pixel[0] as u32 * alpha / 255) as u8;  // R
        dst[base + 3] = pixel[3];                                 // A
    }

    DeleteDC(mem_dc);
    ReleaseDC(std::ptr::null_mut(), screen_dc);
    bmp
}

unsafe fn make_placeholder_bitmap(w: i32, h: i32, alert: bool) -> HBITMAP {
    let screen_dc = GetDC(std::ptr::null_mut());
    let mem_dc    = CreateCompatibleDC(screen_dc);

    let mut bmi: BITMAPINFO = std::mem::zeroed();
    bmi.bmiHeader.biSize        = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth       = w;
    bmi.bmiHeader.biHeight      = -h;
    bmi.bmiHeader.biPlanes      = 1;
    bmi.bmiHeader.biBitCount    = 32;
    bmi.bmiHeader.biCompression = BI_RGB;

    let mut bits: *mut winapi::ctypes::c_void = std::ptr::null_mut();
    let bmp = CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);

    let dst       = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
    let alpha     = 200u8;
    let (r, g, b) = if alert { (239u8, 68u8, 68u8) } else { (34u8, 197u8, 94u8) };

    for i in 0..(w * h) as usize {
        let base      = i * 4;
        dst[base]     = (b as u32 * alpha as u32 / 255) as u8;
        dst[base + 1] = (g as u32 * alpha as u32 / 255) as u8;
        dst[base + 2] = (r as u32 * alpha as u32 / 255) as u8;
        dst[base + 3] = alpha;
    }

    DeleteDC(mem_dc);
    ReleaseDC(std::ptr::null_mut(), screen_dc);
    bmp
}

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}