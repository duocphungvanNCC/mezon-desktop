use std::path::{Path, PathBuf};

#[allow(dead_code)]
const CHROMIUM_STEMS: &[&str] = &[
    "google-chrome",
    "google-chrome-stable",
    "google-chrome-beta",
    "google-chrome-unstable",
    "chrome",
    "chromium",
    "chromium-browser",
    "microsoft-edge",
    "microsoft-edge-stable",
    "msedge",
    "brave",
    "brave-browser",
    "vivaldi",
    "vivaldi-stable",
];

#[allow(dead_code)]
const CHROMIUM_MAC_APPS: &[&str] = &[
    "google chrome",
    "google chrome beta",
    "google chrome canary",
    "chromium",
    "microsoft edge",
    "microsoft edge beta",
    "brave browser",
    "brave browser beta",
    "vivaldi",
];

#[allow(dead_code)]
fn is_chromium_stem(stem: &str) -> bool {
    let stem = stem.trim().to_ascii_lowercase();
    CHROMIUM_STEMS.iter().any(|known| stem == *known)
}

#[allow(dead_code)]
fn is_chromium_mac_app(app_name: &str) -> bool {
    let name = app_name
        .trim()
        .trim_end_matches(".app")
        .to_ascii_lowercase();
    CHROMIUM_MAC_APPS.iter().any(|known| name == *known)
}

#[allow(dead_code)]
fn desktop_entry_stem(desktop_file: &str) -> &str {
    desktop_file
        .trim()
        .rsplit('/')
        .next()
        .unwrap_or("")
        .trim_end_matches(".desktop")
}

#[allow(dead_code)]
fn executable_stem(path: &Path) -> Option<String> {
    Some(path.file_stem()?.to_string_lossy().to_ascii_lowercase())
}

/// Open `url` in a chromeless browser window when the user's default browser can
/// do it, otherwise fall back to a normal tab.
///
/// Chromium `--app=` is the only cross-platform way to get a window with no
/// toolbar without embedding a webview, which this project deliberately does not
/// do (the wry/WebKitGTK dependency was removed on purpose).
///
/// The window runs in the user's own browser profile, so it outlives the app the
/// same way a normal browser tab does. Placing it over the app window would mean
/// a dedicated profile — Chromium drops `--window-position`/`--window-size` when
/// it hands the command line to a running instance — and a dedicated profile
/// leaves instances behind that the app cannot reliably find and close again.
pub fn open_url_app_window(url: &str) -> anyhow::Result<()> {
    crate::ensure_http_url(url)?;
    match default_chromium_browser() {
        Some(browser) => launch_app_window(&browser, url).or_else(|error| {
            tracing::warn!("app-window launch failed, falling back to a browser tab: {error:#}");
            crate::open_url(url)
        }),
        None => crate::open_url(url),
    }
}

#[cfg(target_os = "macos")]
fn launch_app_window(browser: &Path, url: &str) -> anyhow::Result<()> {
    let child = std::process::Command::new("/usr/bin/open")
        .arg("-n")
        .arg("-a")
        .arg(browser)
        .arg("--args")
        .arg(format!("--app={url}"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to open {}: {e}", browser.display()))?;
    reap(child);
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn launch_app_window(browser: &Path, url: &str) -> anyhow::Result<()> {
    let child = std::process::Command::new(browser)
        .arg(format!("--app={url}"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to open {}: {e}", browser.display()))?;
    reap(child);
    Ok(())
}

/// The browser is not ours to manage — it is launched by the OS and outlives the
/// app — but the spawned handle still has to be waited on, or every launch leaves
/// a zombie behind until the app exits.
fn reap(mut child: std::process::Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

#[cfg(target_os = "macos")]
fn default_chromium_browser() -> Option<PathBuf> {
    let app = macos::default_http_handler()?;
    let name = app.file_name()?.to_string_lossy().to_string();
    is_chromium_mac_app(&name).then_some(app)
}

#[cfg(target_os = "macos")]
mod macos {
    use std::path::PathBuf;

    use core_foundation::base::TCFType;
    use core_foundation::error::{CFError, CFErrorRef};
    use core_foundation::string::CFString;
    use core_foundation::url::{CFURL, CFURLRef};

    const K_LS_ROLES_ALL: u32 = 0xFFFF_FFFF;

    unsafe extern "C" {
        fn LSCopyDefaultApplicationURLForURL(
            in_url: CFURLRef,
            in_role_mask: u32,
            out_error: *mut CFErrorRef,
        ) -> CFURLRef;

        fn CFURLCreateWithString(
            allocator: *const std::ffi::c_void,
            url_string: core_foundation::string::CFStringRef,
            base_url: CFURLRef,
        ) -> CFURLRef;
    }

    pub(super) fn default_http_handler() -> Option<PathBuf> {
        let probe = CFString::new("https://example.com");
        let probe_url = unsafe {
            let raw = CFURLCreateWithString(
                std::ptr::null(),
                probe.as_concrete_TypeRef(),
                std::ptr::null(),
            );
            if raw.is_null() {
                return None;
            }
            CFURL::wrap_under_create_rule(raw)
        };

        let mut error: CFErrorRef = std::ptr::null_mut();
        let raw = unsafe {
            LSCopyDefaultApplicationURLForURL(
                probe_url.as_concrete_TypeRef(),
                K_LS_ROLES_ALL,
                &mut error,
            )
        };
        if !error.is_null() {
            unsafe { drop(CFError::wrap_under_create_rule(error)) };
        }
        if raw.is_null() {
            return None;
        }
        let handler = unsafe { CFURL::wrap_under_create_rule(raw) };
        handler.to_path()
    }
}

#[cfg(target_os = "windows")]
fn default_chromium_browser() -> Option<PathBuf> {
    let exe = windows_impl::default_http_handler()?;
    let stem = executable_stem(&exe)?;
    is_chromium_stem(&stem).then_some(exe)
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use std::path::PathBuf;

    use windows::Win32::UI::Shell::{ASSOCF_IS_PROTOCOL, ASSOCSTR_EXECUTABLE, AssocQueryStringW};
    use windows::core::{PCWSTR, w};

    pub(super) fn default_http_handler() -> Option<PathBuf> {
        let mut len: u32 = 0;
        unsafe {
            let _ = AssocQueryStringW(
                ASSOCF_IS_PROTOCOL,
                ASSOCSTR_EXECUTABLE,
                w!("http"),
                PCWSTR::null(),
                None,
                &mut len,
            );
        }
        if len == 0 {
            return None;
        }
        let mut buffer = vec![0u16; len as usize];
        let queried = unsafe {
            AssocQueryStringW(
                ASSOCF_IS_PROTOCOL,
                ASSOCSTR_EXECUTABLE,
                w!("http"),
                PCWSTR::null(),
                Some(windows::core::PWSTR(buffer.as_mut_ptr())),
                &mut len,
            )
        };
        if queried.is_err() {
            return None;
        }
        let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        let path = String::from_utf16(&buffer[..end]).ok()?;
        (!path.trim().is_empty()).then(|| PathBuf::from(path))
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn default_chromium_browser() -> Option<PathBuf> {
    let output = std::process::Command::new("xdg-settings")
        .args(["get", "default-web-browser"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let desktop = String::from_utf8(output.stdout).ok()?;
    let stem = desktop_entry_stem(&desktop);
    if !is_chromium_stem(stem) {
        return None;
    }
    which_binary(stem)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn which_binary(stem: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(stem))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_chromium_executables() {
        assert!(is_chromium_stem("chrome"));
        assert!(is_chromium_stem("MSEdge"));
        assert!(is_chromium_stem(" google-chrome-stable "));
        assert!(is_chromium_stem("brave-browser"));
    }

    #[test]
    fn rejects_non_chromium_executables() {
        assert!(!is_chromium_stem("firefox"));
        assert!(!is_chromium_stem("safari"));
        assert!(!is_chromium_stem("librewolf"));
        assert!(!is_chromium_stem(""));
        assert!(!is_chromium_stem("chrome-remote-desktop"));
    }

    #[test]
    fn recognises_chromium_mac_apps() {
        assert!(is_chromium_mac_app("Google Chrome.app"));
        assert!(is_chromium_mac_app("Microsoft Edge"));
        assert!(is_chromium_mac_app("Brave Browser.app"));
    }

    #[test]
    fn rejects_non_chromium_mac_apps() {
        assert!(!is_chromium_mac_app("Safari.app"));
        assert!(!is_chromium_mac_app("Firefox.app"));
        assert!(!is_chromium_mac_app("Google Chrome Helper.app"));
    }

    #[test]
    fn reads_the_stem_of_a_desktop_entry() {
        assert_eq!(
            desktop_entry_stem("google-chrome.desktop\n"),
            "google-chrome"
        );
        assert_eq!(
            desktop_entry_stem("/usr/share/applications/firefox.desktop"),
            "firefox"
        );
        assert_eq!(desktop_entry_stem(""), "");
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "environment dependent: reports the default browser of the machine it runs on"]
    fn reports_the_default_browser() {
        println!("default http handler: {:?}", macos::default_http_handler());
        println!("chromium app mode target: {:?}", default_chromium_browser());
    }

    #[test]
    fn app_window_rejects_non_http_schemes() {
        assert!(open_url_app_window("file:///etc/passwd").is_err());
        assert!(open_url_app_window("javascript:alert(1)").is_err());
        assert!(open_url_app_window("--app=evil").is_err());
        assert!(open_url_app_window("").is_err());
    }
}
