use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaDevice {
    Microphone,
    Camera,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MediaPermission {
    #[default]
    Granted,
    Undetermined,
    Denied,
}

type ChangeChannel = (flume::Sender<MediaDevice>, flume::Receiver<MediaDevice>);

static CHANGES: LazyLock<ChangeChannel> = LazyLock::new(flume::unbounded);

pub fn media_permission(device: MediaDevice) -> MediaPermission {
    platform::status(device)
}

pub fn recheck_media_permission(device: MediaDevice) -> MediaPermission {
    platform::recheck(device)
}

pub fn media_permission_changes() -> flume::Receiver<MediaDevice> {
    CHANGES.1.clone()
}

pub fn request_media_permission(device: MediaDevice) {
    platform::request(device, |_| {});
}

pub fn open_media_privacy_settings(device: MediaDevice) {
    let Some(mut command) = platform::settings_command(device) else {
        return;
    };
    let spawned = std::thread::Builder::new()
        .name("mezon-privacy-settings".into())
        .spawn(move || {
            if let Err(e) = command.status() {
                tracing::warn!("open privacy settings failed: {e}");
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("open privacy settings failed: {e}");
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn request_media_permission_blocking(
    device: MediaDevice,
    timeout: std::time::Duration,
) -> bool {
    match media_permission(device) {
        MediaPermission::Granted => return true,
        MediaPermission::Denied => return false,
        MediaPermission::Undetermined => {}
    }
    let (tx, rx) = flume::bounded(1);
    platform::request(device, move |granted| {
        let _ = tx.send(granted);
    });
    rx.recv_timeout(timeout).unwrap_or(false)
}

fn publish_change(device: MediaDevice) {
    let _ = CHANGES.0.send(device);
}

#[cfg(target_os = "macos")]
mod platform {
    use block::ConcreteBlock;
    use cocoa::base::{BOOL, NO, id, nil};
    use cocoa::foundation::NSString;
    use objc::runtime::Class;
    use objc::{msg_send, sel, sel_impl};

    use super::{MediaDevice, MediaPermission, publish_change};

    const NOT_DETERMINED: i64 = 0;
    const AUTHORIZED: i64 = 3;
    const RECHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

    fn media_type_name(device: MediaDevice) -> &'static str {
        match device {
            MediaDevice::Microphone => "soun",
            MediaDevice::Camera => "vide",
        }
    }

    fn capture_device_class() -> Option<&'static Class> {
        Class::get("AVCaptureDevice")
    }

    pub(super) fn status(device: MediaDevice) -> MediaPermission {
        let Some(cls) = capture_device_class() else {
            return MediaPermission::Granted;
        };
        let status: i64 = unsafe {
            let media_type: id = NSString::alloc(nil).init_str(media_type_name(device));
            let status: i64 = msg_send![cls, authorizationStatusForMediaType: media_type];
            let _: () = msg_send![media_type, release];
            status
        };
        match status {
            AUTHORIZED => MediaPermission::Granted,
            NOT_DETERMINED => MediaPermission::Undetermined,
            _ => MediaPermission::Denied,
        }
    }

    pub(super) fn recheck(device: MediaDevice) -> MediaPermission {
        let cached = status(device);
        if cached != MediaPermission::Denied {
            return cached;
        }
        let (tx, rx) = flume::bounded(1);
        ask(device, move |granted| {
            let _ = tx.send(granted);
        });
        match rx.recv_timeout(RECHECK_TIMEOUT) {
            Ok(true) => MediaPermission::Granted,
            Ok(false) => MediaPermission::Denied,
            Err(_) => cached,
        }
    }

    pub(super) fn request(device: MediaDevice, on_done: impl Fn(bool) + Send + 'static) {
        ask(device, move |granted| {
            on_done(granted);
            publish_change(device);
        });
    }

    fn ask(device: MediaDevice, on_done: impl Fn(bool) + Send + 'static) {
        let Some(cls) = capture_device_class() else {
            on_done(true);
            return;
        };
        let handler = ConcreteBlock::new(move |granted: BOOL| on_done(granted != NO)).copy();
        unsafe {
            let media_type: id = NSString::alloc(nil).init_str(media_type_name(device));
            let _: () =
                msg_send![cls, requestAccessForMediaType: media_type completionHandler: &*handler];
            let _: () = msg_send![media_type, release];
        }
    }

    pub(super) fn settings_command(device: MediaDevice) -> Option<std::process::Command> {
        let pane = match device {
            MediaDevice::Microphone => "Privacy_Microphone",
            MediaDevice::Camera => "Privacy_Camera",
        };
        let mut command = std::process::Command::new("open");
        command.arg(format!(
            "x-apple.systempreferences:com.apple.preference.security?{pane}"
        ));
        Some(command)
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use windows::Win32::Foundation::NO_ERROR;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
    };
    use windows::core::{PCWSTR, w};

    use super::{MediaDevice, MediaPermission, publish_change};

    fn consent_keys(device: MediaDevice) -> [(HKEY, PCWSTR); 3] {
        match device {
            MediaDevice::Microphone => [
                (
                    HKEY_LOCAL_MACHINE,
                    w!(
                        r"SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone"
                    ),
                ),
                (
                    HKEY_CURRENT_USER,
                    w!(
                        r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone"
                    ),
                ),
                (
                    HKEY_CURRENT_USER,
                    w!(
                        r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone\NonPackaged"
                    ),
                ),
            ],
            MediaDevice::Camera => [
                (
                    HKEY_LOCAL_MACHINE,
                    w!(
                        r"SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\webcam"
                    ),
                ),
                (
                    HKEY_CURRENT_USER,
                    w!(
                        r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\webcam"
                    ),
                ),
                (
                    HKEY_CURRENT_USER,
                    w!(
                        r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\webcam\NonPackaged"
                    ),
                ),
            ],
        }
    }

    fn consent_denied(root: HKEY, key: PCWSTR) -> bool {
        let mut buffer = [0u16; 16];
        let mut size = std::mem::size_of_val(&buffer) as u32;
        let result = unsafe {
            RegGetValueW(
                root,
                key,
                w!("Value"),
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if result != NO_ERROR {
            return false;
        }
        let len = (size as usize / 2).saturating_sub(1).min(buffer.len());
        String::from_utf16_lossy(&buffer[..len]).eq_ignore_ascii_case("Deny")
    }

    pub(super) fn status(device: MediaDevice) -> MediaPermission {
        if consent_keys(device)
            .into_iter()
            .any(|(root, key)| consent_denied(root, key))
        {
            MediaPermission::Denied
        } else {
            MediaPermission::Granted
        }
    }

    pub(super) fn recheck(device: MediaDevice) -> MediaPermission {
        status(device)
    }

    pub(super) fn request(device: MediaDevice, on_done: impl Fn(bool) + Send + 'static) {
        on_done(status(device) == MediaPermission::Granted);
        publish_change(device);
    }

    pub(super) fn settings_command(device: MediaDevice) -> Option<std::process::Command> {
        use std::os::windows::process::CommandExt as _;

        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let page = match device {
            MediaDevice::Microphone => "ms-settings:privacy-microphone",
            MediaDevice::Camera => "ms-settings:privacy-webcam",
        };
        let mut command = std::process::Command::new("cmd");
        command
            .args(["/C", "start", "", page])
            .creation_flags(CREATE_NO_WINDOW);
        Some(command)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::{MediaDevice, MediaPermission, publish_change};

    pub(super) fn status(_device: MediaDevice) -> MediaPermission {
        MediaPermission::Granted
    }

    pub(super) fn recheck(_device: MediaDevice) -> MediaPermission {
        MediaPermission::Granted
    }

    pub(super) fn request(device: MediaDevice, on_done: impl Fn(bool) + Send + 'static) {
        on_done(true);
        publish_change(device);
    }

    pub(super) fn settings_command(_device: MediaDevice) -> Option<std::process::Command> {
        None
    }
}
