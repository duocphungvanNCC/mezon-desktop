use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveWindowInfo {
    pub os: String,
    #[serde(rename = "windowClass")]
    pub window_class: String,
    #[serde(rename = "windowName")]
    pub window_name: String,
    #[serde(rename = "windowDesktop")]
    pub window_desktop: String,
    #[serde(rename = "windowType")]
    pub window_type: String,
    #[serde(rename = "windowPid")]
    pub window_pid: String,
    #[serde(rename = "idleTime")]
    pub idle_time: String,
}

#[cfg(target_os = "windows")]
mod windows {
    use super::ActiveWindowInfo;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::Foundation::{HWND, MAX_PATH};
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
    };

    pub fn get_active_window() -> anyhow::Result<ActiveWindowInfo> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.0 == 0 {
                return Err(anyhow::anyhow!("No active window"));
            }

            // Get window title
            let mut title_buf = [0u16; 256];
            let len = GetWindowTextW(hwnd, &mut title_buf);
            let window_name = if len > 0 {
                String::from_utf16_lossy(&title_buf[..len as usize])
            } else {
                String::new()
            };

            // Get PID
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));

            // Get process image name (window class)
            let mut window_class = String::new();
            if pid != 0 {
                if let Ok(handle) = OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_QUERY_INFORMATION,
                    false,
                    pid,
                ) {
                    let mut path_buf = [0u16; MAX_PATH as usize];
                    let mut size = path_buf.len() as u32;
                    if QueryFullProcessImageNameW(
                        handle,
                        windows::Win32::System::Threading::PROCESS_NAME_FORMAT(0),
                        windows::core::PWSTR::from_raw(path_buf.as_mut_ptr()),
                        &mut size,
                    )
                    .is_ok()
                    {
                        let full_path = OsString::from_wide(&path_buf[..size as usize])
                            .to_string_lossy()
                            .into_owned();
                        window_class = std::path::Path::new(&full_path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(&full_path)
                            .to_string();
                    }
                }
            }

            // Get Idle Time
            let mut last_input = LASTINPUTINFO::default();
            last_input.cbSize = std::mem::size_of::<LASTINPUTINFO>() as u32;
            let mut idle_time = 0;
            if GetLastInputInfo(&mut last_input).is_ok() {
                let tick_count = windows::Win32::System::SystemInformation::GetTickCount();
                idle_time = (tick_count.saturating_sub(last_input.dwTime)) / 1000;
            }

            Ok(ActiveWindowInfo {
                os: "windows".to_string(),
                window_class,
                window_name,
                window_desktop: "0".to_string(),
                window_type: "0".to_string(),
                window_pid: pid.to_string(),
                idle_time: idle_time.to_string(),
            })
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::ActiveWindowInfo;
    use cocoa::base::{id, nil};
    use cocoa::foundation::NSString;
    use core_foundation::array::{CFArrayGetCount, CFArrayGetValueAtIndex};
    use core_foundation::base::TCFType;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use std::ffi::c_void;

    extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relativeToWindow: u32) -> *const c_void;
        fn CGEventSourceSecondsSinceLastEventType(source_state_id: i32, event_type: u32) -> f64;
    }

    pub fn get_active_window() -> anyhow::Result<ActiveWindowInfo> {
        // CGWindowListOption: kCGWindowListOptionOnScreenOnly = (1 << 0), kCGWindowListExcludeDesktopElements = (1 << 4)
        let list_options = (1 << 0) | (1 << 4);
        let window_list_ptr = unsafe { CGWindowListCopyWindowInfo(list_options, 0) };
        if window_list_ptr.is_null() {
            return Err(anyhow::anyhow!("Failed to copy window info list"));
        }

        let window_list: core_foundation::array::CFArray =
            unsafe { TCFType::wrap_under_create_rule(window_list_ptr as _) };
        let count = window_list.len();

        let mut active_info = ActiveWindowInfo {
            os: "macos".to_string(),
            window_class: String::new(),
            window_name: String::new(),
            window_desktop: "0".to_string(),
            window_type: "0".to_string(),
            window_pid: "0".to_string(),
            idle_time: get_idle_time().to_string(),
        };

        // Standard filter keys matching the C++ counterpart
        let code_editors = [
            "code",
            "sublime",
            "atom",
            "notepad",
            "coffee",
            "textmate",
            "bluefish",
            "vim",
            "netbean",
            "emacs",
            "bbedit",
            "webstorm",
            "ultraedit",
            "nova",
            "unity",
            "figma",
            "spotify",
            "photoshop",
            "chrome",
            "safari",
        ];

        for i in 0..count {
            let dict_ref = unsafe { CFArrayGetValueAtIndex(window_list.as_concrete_TypeRef(), i) };
            if dict_ref.is_null() {
                continue;
            }

            let dict: CFDictionary = unsafe { TCFType::wrap_under_get_rule(dict_ref as _) };

            // OwnerPID
            let pid_key = unsafe {
                core_foundation::string::CFString::from_static_string("kCGWindowOwnerPID")
            };
            let pid_val = dict.find(pid_key.as_void_ptr());
            if let Some(pid_ptr) = pid_val {
                let num = unsafe { CFNumber::wrap_under_get_rule(pid_ptr as _) };
                if let Some(pid_i64) = num.to_i64() {
                    active_info.window_pid = pid_i64.to_string();
                }
            }

            // OwnerName (Class name equivalent)
            let owner_key = unsafe {
                core_foundation::string::CFString::from_static_string("kCGWindowOwnerName")
            };
            let owner_val = dict.find(owner_key.as_void_ptr());
            let mut window_class = String::new();
            if let Some(owner_ptr) = owner_val {
                let cf_str = unsafe {
                    core_foundation::string::CFString::wrap_under_get_rule(owner_ptr as _)
                };
                window_class = cf_str.to_string();
            }

            // WindowName
            let name_key =
                unsafe { core_foundation::string::CFString::from_static_string("kCGWindowName") };
            let name_val = dict.find(name_key.as_void_ptr());
            let mut window_name = String::new();
            if let Some(name_ptr) = name_val {
                let cf_str = unsafe {
                    core_foundation::string::CFString::wrap_under_get_rule(name_ptr as _)
                };
                window_name = cf_str.to_string();
            }

            if window_class.is_empty() && window_name.is_empty() {
                continue;
            }

            let lower_class = window_class.to_lowercase();
            let matched = code_editors
                .iter()
                .any(|&editor| lower_class.contains(editor));
            if matched {
                active_info.window_class = window_class;
                active_info.window_name = window_name;
                break;
            }
        }

        Ok(active_info)
    }

    fn get_idle_time() -> u64 {
        // CGEventSourceSecondsSinceLastEventType(kCGEventSourceStateCombinedSessionState = 0, kCGAnyInputEventType = ~0)
        let idle = unsafe { CGEventSourceSecondsSinceLastEventType(0, !0) };
        if idle < 0.0 { 0 } else { idle as u64 }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod linux {
    use super::ActiveWindowInfo;
    use x11rb::connection::Connection;
    use x11rb::protocol::screensaver::ConnectionExt as _;
    use x11rb::protocol::xproto::{Atom, ConnectionExt, GetPropertyReply, Window};

    pub fn get_active_window() -> anyhow::Result<ActiveWindowInfo> {
        let (conn, screen_num) =
            x11rb::connect(None).map_err(|e| anyhow::anyhow!("X11 connection failed: {}", e))?;
        let setup = conn.setup();
        let screen = &setup.roots[screen_num];
        let root = screen.root;

        let active_window_atom = get_atom(&conn, "_NET_ACTIVE_WINDOW")?;
        let wm_pid_atom = get_atom(&conn, "_NET_WM_PID")?;
        let wm_class_atom = get_atom(&conn, "WM_CLASS")?;
        let wm_name_atom = get_atom(&conn, "_NET_WM_NAME")?;
        let wm_desktop_atom = get_atom(&conn, "_NET_WM_DESKTOP")?;
        let wm_window_type_atom = get_atom(&conn, "_NET_WM_WINDOW_TYPE")?;

        // Get active window property from Root window
        let active_reply = conn
            .get_property(
                false,
                root,
                active_window_atom,
                x11rb::protocol::xproto::AtomEnum::WINDOW,
                0,
                1000,
            )
            .map_err(|e| anyhow::anyhow!("Failed to get active window property: {}", e))?
            .reply()
            .map_err(|e| anyhow::anyhow!("Property reply failed: {}", e))?;

        let active_window = get_long_val(&active_reply)
            .ok_or_else(|| anyhow::anyhow!("_NET_ACTIVE_WINDOW is empty"))?
            as Window;

        if active_window == 0 {
            return Err(anyhow::anyhow!("Active window is 0"));
        }

        // Get window details
        let pid_reply = conn
            .get_property(
                false,
                active_window,
                wm_pid_atom,
                x11rb::protocol::xproto::AtomEnum::CARDINAL,
                0,
                1000,
            )
            .ok();
        let pid = pid_reply
            .and_then(|r| r.reply().ok())
            .and_then(|reply| get_long_val(&reply))
            .unwrap_or(0);

        let class_reply = conn
            .get_property(
                false,
                active_window,
                wm_class_atom,
                x11rb::protocol::xproto::AtomEnum::STRING,
                0,
                1000,
            )
            .ok();
        let window_class = class_reply
            .and_then(|r| r.reply().ok())
            .map(|reply| get_string_val(&reply))
            .unwrap_or_default();

        let name_reply = conn
            .get_property(
                false,
                active_window,
                wm_name_atom,
                get_atom(&conn, "UTF8_STRING").unwrap_or_default(),
                0,
                1000,
            )
            .ok();
        let window_name = name_reply
            .and_then(|r| r.reply().ok())
            .map(|reply| get_string_val(&reply))
            .unwrap_or_default();

        let desktop_reply = conn
            .get_property(
                false,
                active_window,
                wm_desktop_atom,
                x11rb::protocol::xproto::AtomEnum::CARDINAL,
                0,
                1000,
            )
            .ok();
        let desktop = desktop_reply
            .and_then(|r| r.reply().ok())
            .and_then(|reply| get_long_val(&reply))
            .unwrap_or(0);

        let type_reply = conn
            .get_property(
                false,
                active_window,
                wm_window_type_atom,
                x11rb::protocol::xproto::AtomEnum::ATOM,
                0,
                1000,
            )
            .ok();
        let win_type = type_reply
            .and_then(|r| r.reply().ok())
            .and_then(|reply| get_long_val(&reply))
            .unwrap_or(0);

        // Get Idle Time via XScreenSaver
        let mut idle_time = 0;
        if let Some(reply) = conn
            .screensaver_query_info(root)
            .ok()
            .and_then(|info| info.reply().ok())
        {
            idle_time = reply.ms_since_user_input / 1000;
        }

        Ok(ActiveWindowInfo {
            os: "linux".to_string(),
            window_class,
            window_name,
            window_desktop: desktop.to_string(),
            window_type: win_type.to_string(),
            window_pid: pid.to_string(),
            idle_time: idle_time.to_string(),
        })
    }

    fn get_atom<C: Connection>(conn: &C, name: &str) -> anyhow::Result<Atom> {
        let reply = conn
            .intern_atom(false, name.as_bytes())
            .map_err(|e| anyhow::anyhow!("Intern atom failed: {}", e))?
            .reply()
            .map_err(|e| anyhow::anyhow!("Intern atom reply failed: {}", e))?;
        Ok(reply.atom)
    }

    fn get_long_val(reply: &GetPropertyReply) -> Option<u32> {
        if reply.value.len() >= 4 {
            Some(u32::from_ne_bytes([
                reply.value[0],
                reply.value[1],
                reply.value[2],
                reply.value[3],
            ]))
        } else {
            None
        }
    }

    fn get_string_val(reply: &GetPropertyReply) -> String {
        String::from_utf8_lossy(&reply.value).into_owned()
    }
}

pub fn get_active_window() -> anyhow::Result<ActiveWindowInfo> {
    #[cfg(target_os = "windows")]
    return windows::get_active_window();

    #[cfg(target_os = "macos")]
    return macos::get_active_window();

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return linux::get_active_window();
}
