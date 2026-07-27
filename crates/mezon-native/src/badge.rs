//! Set the app badge count on the dock icon (macOS) or taskbar overlay (Windows).
//!
//! macOS  : `NSDockTile.setBadgeLabel` via the `objc` runtime.
//! Windows: `ITaskbarList3::SetOverlayIcon` — renders a small count bitmap
//!          as an overlay on the taskbar button.

pub fn set_badge_count(count: u32) {
    tracing::debug!("set_badge_count({})", count);

    #[cfg(target_os = "macos")]
    set_badge_macos(count);

    #[cfg(target_os = "windows")]
    set_badge_windows(count);

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    set_badge_linux(count);
}

// ─── Linux ──────────────────────────────────────────────────────────────────────
//
// The Unity LauncherEntry D-Bus API (`com.canonical.Unity.LauncherEntry.Update`)
// is honoured by GNOME (Dash-to-Dock), KDE Plasma, Unity and others.

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn set_badge_linux(count: u32) {
    std::thread::spawn(move || {
        if let Err(e) = try_set_badge_linux(count) {
            tracing::warn!("Linux badge update failed: {e}");
        }
    });
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn try_set_badge_linux(count: u32) -> Result<(), Box<dyn std::error::Error>> {
    use std::collections::HashMap;
    use zbus::zvariant::Value;

    let connection = zbus::blocking::Connection::session()?;
    let mut props: HashMap<&str, Value> = HashMap::new();
    props.insert("count", Value::I64(i64::from(count)));
    props.insert("count-visible", Value::Bool(count > 0));

    connection.emit_signal(
        None::<&str>,
        "/com/canonical/unity/launcherentry/mezon",
        "com.canonical.Unity.LauncherEntry",
        "Update",
        &("application://mezon.desktop", props),
    )?;
    Ok(())
}

#[cfg(target_os = "windows")]
static MAIN_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

#[cfg(target_os = "windows")]
pub fn set_main_window_hwnd(hwnd: isize) {
    MAIN_HWND.store(hwnd, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(not(target_os = "windows"))]
pub fn set_main_window_hwnd(_hwnd: isize) {}

// ─── macOS ────────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn set_badge_macos(count: u32) {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};

    // Safety: must be called from main thread (guaranteed — GPUI runs UI on main).
    unsafe {
        let cls = class!(NSApplication);
        let app: *mut Object = msg_send![cls, sharedApplication];
        let dock_tile: *mut Object = msg_send![app, dockTile];

        let label: *mut Object = if count == 0 {
            std::ptr::null_mut()
        } else {
            let label_str = count.to_string();
            let cls = class!(NSString);
            let s: *mut Object = msg_send![cls, alloc];
            let s: *mut Object = msg_send![s,
                initWithBytes: label_str.as_ptr()
                length: label_str.len()
                encoding: 4u64 // NSUTF8StringEncoding
            ];
            s
        };

        let _: () = msg_send![dock_tile, setBadgeLabel: label];
        if !label.is_null() {
            let _: () = msg_send![label, release];
        }
    }
}

// ─── Windows ──────────────────────────────────────────────────────────────────
//
// `ITaskbarList3::SetOverlayIcon` places a small icon in the bottom-right
// corner of the app's taskbar button.  We generate a 16×16 HICON on the fly
// that contains the count text, or pass NULL to clear it.

#[cfg(target_os = "windows")]
fn set_badge_windows(count: u32) {
    if let Err(e) = try_set_overlay_icon(count) {
        tracing::warn!("Windows badge count failed: {e}");
    }
}

#[cfg(target_os = "windows")]
fn try_set_overlay_icon(count: u32) -> windows::core::Result<()> {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
    use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList};
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};

    use windows::Win32::Foundation::HWND;

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }

    let stored = MAIN_HWND.load(std::sync::atomic::Ordering::Relaxed);
    let hwnd = if stored != 0 {
        HWND(stored as *mut core::ffi::c_void)
    } else {
        unsafe { GetActiveWindow() }
    };

    let taskbar: ITaskbarList3 = unsafe {
        windows::Win32::System::Com::CoCreateInstance(
            &TaskbarList,
            None,
            windows::Win32::System::Com::CLSCTX_INPROC_SERVER,
        )?
    };
    unsafe { taskbar.HrInit()? };

    if count == 0 {
        // Pass null icon to clear the overlay.
        unsafe {
            taskbar.SetOverlayIcon(
                hwnd,
                HICON(std::ptr::null_mut()),
                &windows::core::HSTRING::new(),
            )?
        };
        return Ok(());
    }

    // Build a 16×16 badge bitmap with the count text.
    let hicon = build_count_icon(count)?;
    let description = windows::core::HSTRING::from(format!("{count} unread"));

    unsafe {
        taskbar.SetOverlayIcon(hwnd, hicon, &description)?;
        let _ = DestroyIcon(hicon);
    }
    Ok(())
}

/// Generate a 16×16 `HICON` with the badge count centred on a red circle
/// (matching the in-app `#DA373C` unread badges). A monochrome AND-mask makes
/// the pixels outside the circle transparent so the overlay reads as a dot, not
/// a square.
#[cfg(target_os = "windows")]
fn build_count_icon(
    count: u32,
) -> windows::core::Result<windows::Win32::UI::WindowsAndMessaging::HICON> {
    use windows::Win32::Foundation::{COLORREF, RECT};
    use windows::Win32::Graphics::Gdi::{
        CreateBitmap, CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush,
        DEFAULT_GUI_FONT, DeleteDC, DeleteObject, Ellipse, FillRect, GetDC, GetStockObject,
        NULL_PEN, ReleaseDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT, TextOutW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, ICONINFO};

    const SIZE: i32 = 16;
    // #DA373C danger red (matches every in-app badge); white text. COLORREF is 0x00BBGGRR.
    const BG_COLOR: u32 = 0x003C_37DA;
    const TEXT_COLOR: u32 = 0x00FF_FFFF;
    const WHITE: u32 = 0x00FF_FFFF;
    const BLACK: u32 = 0x0000_0000;

    let label = if count > 99 {
        "99+".to_owned()
    } else {
        count.to_string()
    };
    let text: Vec<u16> = label.encode_utf16().collect();
    let text_x = match text.len() {
        1 => 5,
        2 => 2,
        _ => 0,
    };

    let full = RECT {
        left: 0,
        top: 0,
        right: SIZE,
        bottom: SIZE,
    };

    let hdc_screen = unsafe { GetDC(None) };
    let hdc = unsafe { CreateCompatibleDC(Some(hdc_screen)) };
    let hbmp_color = unsafe { CreateCompatibleBitmap(hdc_screen, SIZE, SIZE) };
    // The AND-mask must be a 1bpp monochrome bitmap: white = transparent, black = opaque.
    let hbmp_mask = unsafe { CreateBitmap(SIZE, SIZE, 1, 1, None) };
    unsafe { ReleaseDC(None, hdc_screen) };

    unsafe {
        let old_bmp = SelectObject(hdc, hbmp_color.into());
        let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));

        // ── Colour bitmap: transparent (black) background + red filled circle + text ──
        let black_brush = CreateSolidBrush(COLORREF(BLACK));
        FillRect(hdc, &full, black_brush);
        let _ = DeleteObject(black_brush.into());

        let red_brush = CreateSolidBrush(COLORREF(BG_COLOR));
        let old_brush = SelectObject(hdc, red_brush.into());
        let _ = Ellipse(hdc, 0, 0, SIZE, SIZE);
        SelectObject(hdc, old_brush);
        let _ = DeleteObject(red_brush.into());

        let old_font = SelectObject(hdc, GetStockObject(DEFAULT_GUI_FONT));
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, COLORREF(TEXT_COLOR));
        let _ = TextOutW(hdc, text_x, 2, &text);
        SelectObject(hdc, old_font);

        // ── Monochrome mask: white (transparent) everywhere, black (opaque) circle ──
        SelectObject(hdc, hbmp_mask.into());
        let white_brush = CreateSolidBrush(COLORREF(WHITE));
        FillRect(hdc, &full, white_brush);
        let _ = DeleteObject(white_brush.into());
        let mask_black = CreateSolidBrush(COLORREF(BLACK));
        let mask_old_brush = SelectObject(hdc, mask_black.into());
        let _ = Ellipse(hdc, 0, 0, SIZE, SIZE);
        SelectObject(hdc, mask_old_brush);
        let _ = DeleteObject(mask_black.into());

        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_bmp);
        let _ = DeleteDC(hdc);
    }

    let icon_info = ICONINFO {
        fIcon: true.into(),
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: hbmp_mask,
        hbmColor: hbmp_color,
    };

    let hicon = unsafe { CreateIconIndirect(&icon_info)? };

    unsafe {
        let _ = DeleteObject(hbmp_color.into());
        let _ = DeleteObject(hbmp_mask.into());
    }

    Ok(hicon)
}
