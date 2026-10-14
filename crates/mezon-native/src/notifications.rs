/// Desktop notification support.
///
/// macOS  : `UNUserNotificationCenter` via raw `objc` runtime calls.
///          Authorisation is requested once from `init`, not per notification.
/// Windows: Windows Runtime `ToastNotification` via the `windows` crate.
///          `init` registers the process AUMID and a Start Menu shortcut carrying
///          the same AUMID — without that shortcut the OS drops every toast.
/// Linux  : `notify-rust` (wraps `libnotify` / D-Bus `org.freedesktop.Notifications`).

#[derive(Debug, Clone)]
pub struct Notification {
    pub title: String,
    pub body: String,
    /// Optional channel ID — used as the per-channel replacement key.
    pub channel_id: Option<String>,
    pub clan_id: Option<String>,
    /// Server-rendered navigation path (React `extras.link`); the click target.
    pub link: Option<String>,
    pub topic_reply: Option<TopicReply>,
    /// Local file path to the sender-avatar image, shown as the notification icon.
    pub icon_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicReply {
    pub topic_id: String,
    pub message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationTarget {
    pub clan_id: Option<String>,
    pub channel_id: String,
    pub link: Option<String>,
    pub topic_reply: Option<TopicReply>,
}

type ClickHandler = Box<dyn Fn(NotificationTarget) + Send + Sync + 'static>;

static CLICK_HANDLER: std::sync::OnceLock<ClickHandler> = std::sync::OnceLock::new();

/// Maps a notification's replacement identifier to its server-rendered link, so a
/// click can navigate exactly where React would (`extras.link`) instead of
/// reconstructing a route from ids.
static LINK_BY_ID: std::sync::Mutex<Option<std::collections::HashMap<String, String>>> =
    std::sync::Mutex::new(None);

fn remember_link(identifier: &str, link: &str) {
    if link.is_empty() {
        return;
    }
    if let Ok(mut guard) = LINK_BY_ID.lock() {
        guard
            .get_or_insert_with(std::collections::HashMap::new)
            .insert(identifier.to_owned(), link.to_owned());
    }
}

static TOPIC_REPLY_BY_ID: std::sync::Mutex<Option<std::collections::HashMap<String, TopicReply>>> =
    std::sync::Mutex::new(None);

fn remember_topic_reply(identifier: &str, topic_reply: Option<&TopicReply>) {
    if let Ok(mut guard) = TOPIC_REPLY_BY_ID.lock() {
        let replies = guard.get_or_insert_with(std::collections::HashMap::new);
        match topic_reply {
            Some(reply) => replies.insert(identifier.to_owned(), reply.clone()),
            None => replies.remove(identifier),
        };
    }
}

fn build_target(identifier: &str) -> Option<NotificationTarget> {
    let mut target = decode_identifier(identifier)?;
    target.link = LINK_BY_ID
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|m| m.get(identifier).cloned()));
    target.topic_reply = TOPIC_REPLY_BY_ID
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|m| m.get(identifier).cloned()));
    Some(target)
}

pub fn set_click_handler(handler: ClickHandler) {
    if CLICK_HANDLER.set(handler).is_err() {
        tracing::warn!("notification click handler already registered");
    }
}

fn dispatch_click(target: NotificationTarget) {
    match CLICK_HANDLER.get() {
        Some(handler) => handler(target),
        None => tracing::debug!("notification clicked but no handler registered"),
    }
}

const IDENTIFIER_PREFIX: &str = "mezon-notify";

fn encode_identifier(clan_id: Option<&String>, channel_id: &str) -> String {
    let clan = clan_id
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("0");
    format!("{IDENTIFIER_PREFIX}-{clan}-{channel_id}")
}

fn decode_identifier(identifier: &str) -> Option<NotificationTarget> {
    let rest = identifier
        .strip_prefix(IDENTIFIER_PREFIX)?
        .strip_prefix('-')?;
    let (clan, channel) = rest.split_once('-')?;
    if channel.is_empty() {
        return None;
    }
    Some(NotificationTarget {
        clan_id: Some(clan.to_owned()).filter(|c| c != "0" && !c.is_empty()),
        link: None,
        channel_id: channel.to_owned(),
        topic_reply: None,
    })
}

#[cfg(target_os = "windows")]
const APP_USER_MODEL_ID: &str = "ai.mezon.Mezon";

pub fn init() {
    #[cfg(target_os = "macos")]
    init_macos();

    #[cfg(target_os = "windows")]
    init_windows();
}

/// Show a desktop notification.  Fire-and-forget; errors are logged but not propagated.
pub fn show(notification: &Notification) {
    tracing::debug!(
        channel_id = ?notification.channel_id,
        "Showing desktop notification"
    );

    if let Some(key) = replacement_key(notification) {
        if let Some(link) = notification.link.as_deref() {
            remember_link(&key, link);
        }
        remember_topic_reply(&key, notification.topic_reply.as_ref());
    }

    #[cfg(target_os = "macos")]
    show_macos(notification);

    #[cfg(target_os = "windows")]
    show_windows(notification);

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    show_linux(notification);
}

fn replacement_key(n: &Notification) -> Option<String> {
    let channel = n
        .channel_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty() && *id != "0")?;
    Some(encode_identifier(n.clan_id.as_ref(), channel))
}

// ─── macOS ────────────────────────────────────────────────────────────────────
//
// UNUserNotificationCenter is available on macOS 10.14+.
// We drive it through the raw `objc` runtime to avoid linking against the
// UserNotifications framework header directly.

#[cfg(target_os = "macos")]
static NOTIFICATION_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(target_os = "macos")]
const AUTH_UNKNOWN: u8 = 0;
#[cfg(target_os = "macos")]
const AUTH_GRANTED: u8 = 1;
#[cfg(target_os = "macos")]
const AUTH_DENIED: u8 = 2;
#[cfg(target_os = "macos")]
static NOTIFICATION_AUTH: std::sync::atomic::AtomicU8 =
    std::sync::atomic::AtomicU8::new(AUTH_UNKNOWN);

/// Whether the OS permits notifications. Returns `false` only when authorisation was
/// explicitly denied; an as-yet-undetermined state (async grant pending) returns `true`.
pub fn notifications_permitted() -> bool {
    #[cfg(target_os = "macos")]
    {
        NOTIFICATION_AUTH.load(std::sync::atomic::Ordering::Relaxed) != AUTH_DENIED
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

#[cfg(target_os = "macos")]
fn leak_for_async_objc_callback<T>(block: T) {
    std::mem::forget(block);
}

#[cfg(target_os = "macos")]
fn has_bundle_identifier() -> bool {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};

    unsafe {
        let bundle: *mut Object = msg_send![class!(NSBundle), mainBundle];
        if bundle.is_null() {
            return false;
        }
        let bundle_id: *mut Object = msg_send![bundle, bundleIdentifier];
        !bundle_id.is_null()
    }
}

#[cfg(target_os = "macos")]
extern "C" fn did_receive_notification_response(
    _this: &objc::runtime::Object,
    _sel: objc::runtime::Sel,
    _center: *mut objc::runtime::Object,
    response: *mut objc::runtime::Object,
    completion: *mut objc::runtime::Object,
) {
    use objc::runtime::Object;
    use objc::{msg_send, sel, sel_impl};

    unsafe {
        if !response.is_null() {
            let notification: *mut Object = msg_send![response, notification];
            if !notification.is_null() {
                let request: *mut Object = msg_send![notification, request];
                if !request.is_null() {
                    let identifier: *mut Object = msg_send![request, identifier];
                    if let Some(identifier) = nsstring_to_string(identifier)
                        && let Some(target) = build_target(&identifier)
                    {
                        dispatch_click(target);
                    }
                }
            }
        }
        if !completion.is_null() {
            let block = completion as *mut block::Block<(), ()>;
            (*block).call(());
        }
    }
}

#[cfg(target_os = "macos")]
const PRESENT_BANNER_LIST_SOUND: usize = (1 << 4) | (1 << 3) | (1 << 1);

#[cfg(target_os = "macos")]
extern "C" fn will_present_notification(
    _this: &objc::runtime::Object,
    _sel: objc::runtime::Sel,
    _center: *mut objc::runtime::Object,
    _notification: *mut objc::runtime::Object,
    completion: *mut objc::runtime::Object,
) {
    tracing::debug!(target: "noti", "presenting a notification while the app is active");
    if completion.is_null() {
        return;
    }
    unsafe {
        let block = completion as *mut block::Block<(usize,), ()>;
        (*block).call((PRESENT_BANNER_LIST_SOUND,));
    }
}

#[cfg(target_os = "macos")]
unsafe fn nsstring_to_string(s: *mut objc::runtime::Object) -> Option<String> {
    use objc::{msg_send, sel, sel_impl};

    if s.is_null() {
        return None;
    }
    unsafe {
        let bytes: *const std::os::raw::c_char = msg_send![s, UTF8String];
        if bytes.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr(bytes)
            .to_str()
            .ok()
            .map(str::to_owned)
    }
}

#[cfg(target_os = "macos")]
fn install_notification_delegate(center: *mut objc::runtime::Object) {
    use objc::declare::ClassDecl;
    use objc::runtime::{Class, Object, Sel};
    use objc::{class, msg_send, sel, sel_impl};

    static DELEGATE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

    let delegate = *DELEGATE.get_or_init(|| unsafe {
        let superclass = class!(NSObject);
        let Some(mut decl) = ClassDecl::new("MezonNotificationDelegate", superclass) else {
            return 0;
        };
        let callback: extern "C" fn(&Object, Sel, *mut Object, *mut Object, *mut Object) =
            did_receive_notification_response;
        decl.add_method(
            sel!(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:),
            callback,
        );
        let present: extern "C" fn(&Object, Sel, *mut Object, *mut Object, *mut Object) =
            will_present_notification;
        decl.add_method(
            sel!(userNotificationCenter:willPresentNotification:withCompletionHandler:),
            present,
        );
        let cls: &Class = decl.register();
        let instance: *mut Object = msg_send![cls, new];
        instance as usize
    });

    if delegate == 0 {
        tracing::warn!("could not register notification delegate class");
        return;
    }
    unsafe {
        let _: () = msg_send![center, setDelegate: delegate as *mut Object];
    }
}

#[cfg(target_os = "macos")]
fn init_macos() {
    use block::ConcreteBlock;
    use objc::runtime::{BOOL, Object, YES};
    use objc::{class, msg_send, sel, sel_impl};

    if !has_bundle_identifier() {
        tracing::warn!(
            target: "noti",
            "notification setup skipped: not running from an app bundle, so macOS will \
             refuse every notification this process posts"
        );
        return;
    }

    unsafe {
        let center_cls = class!(UNUserNotificationCenter);
        let center: *mut Object = msg_send![center_cls, currentNotificationCenter];

        install_notification_delegate(center);

        const AUTH_OPTIONS_BADGE_SOUND_ALERT: usize = 0b111;
        let options: usize = AUTH_OPTIONS_BADGE_SOUND_ALERT;
        let handler = ConcreteBlock::new(move |granted: BOOL, _error: *mut Object| {
            if granted == YES {
                tracing::info!(target: "noti", "macOS notification authorisation granted");
                NOTIFICATION_AUTH.store(AUTH_GRANTED, std::sync::atomic::Ordering::Relaxed);
            } else {
                tracing::warn!(
                    target: "noti",
                    "macOS notification authorisation DENIED — no notification will appear. \
                     Enable it in System Settings > Notifications > Mezon, then restart the app."
                );
                NOTIFICATION_AUTH.store(AUTH_DENIED, std::sync::atomic::Ordering::Relaxed);
            }
        });
        let handler = handler.copy();
        let _: () = msg_send![
            center,
            requestAuthorizationWithOptions: options
            completionHandler: &*handler
        ];
        leak_for_async_objc_callback(handler);
    }
}

#[cfg(target_os = "macos")]
fn show_macos(n: &Notification) {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};

    if !has_bundle_identifier() {
        tracing::warn!(
            target: "noti",
            title = %n.title,
            "notification dropped: not running from an app bundle"
        );
        return;
    }

    unsafe {
        let center_cls = class!(UNUserNotificationCenter);
        let center: *mut Object = msg_send![center_cls, currentNotificationCenter];

        let content_cls = class!(UNMutableNotificationContent);
        let content: *mut Object = msg_send![content_cls, new];

        let ns_title = nsstring(&n.title);
        let _: () = msg_send![content, setTitle: ns_title];
        let _: () = msg_send![ns_title, release];

        let ns_body = nsstring(&n.body);
        let _: () = msg_send![content, setBody: ns_body];
        let _: () = msg_send![ns_body, release];

        // Default alert sound, matching React's `sound: 'default'`; without this a
        // UNUserNotificationCenter notification is delivered silently.
        let sound_cls = class!(UNNotificationSound);
        let sound: *mut Object = msg_send![sound_cls, defaultSound];
        let _: () = msg_send![content, setSound: sound];

        if let Some(icon_path) = n.icon_path.as_deref() {
            attach_icon_macos(content, icon_path);
        }

        let key = replacement_key(n);
        if let Some(key) = key.as_deref() {
            let ns_thread = nsstring(key);
            let _: () = msg_send![content, setThreadIdentifier: ns_thread];
            let _: () = msg_send![ns_thread, release];
        }

        let identifier = key.unwrap_or_else(|| {
            let counter = NOTIFICATION_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            format!("mezon-{}-{counter}", std::process::id())
        });
        let ns_identifier = nsstring(&identifier);
        let request_cls = class!(UNNotificationRequest);
        let request: *mut Object = msg_send![
            request_cls,
            requestWithIdentifier: ns_identifier
            content: content
            trigger: std::ptr::null::<Object>()
        ];
        let _: () = msg_send![ns_identifier, release];
        let _: () = msg_send![content, release];

        // A null completion handler throws the OS's verdict away; with one we learn
        // whether macOS actually accepted the request, which is otherwise invisible.
        let title = n.title.clone();
        let handler = block::ConcreteBlock::new(move |error: *mut Object| {
            if error.is_null() {
                tracing::info!(target: "noti", title = %title, "macOS accepted the notification");
            } else {
                let description: *mut Object = msg_send![error, localizedDescription];
                let reason = nsstring_to_string(description).unwrap_or_default();
                tracing::warn!(
                    target: "noti",
                    title = %title,
                    "macOS rejected the notification: {reason}"
                );
            }
        });
        let handler = handler.copy();
        let _: () = msg_send![
            center,
            addNotificationRequest: request
            withCompletionHandler: &*handler
        ];
        leak_for_async_objc_callback(handler);
    }
}

/// Create an Objective-C NSString from a Rust &str.
#[cfg(target_os = "macos")]
unsafe fn nsstring(s: &str) -> *mut objc::runtime::Object {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};

    let cls = class!(NSString);
    let obj: *mut Object = msg_send![cls, alloc];
    msg_send![
        obj,
        initWithBytes: s.as_ptr()
        length: s.len()
        encoding: 4u64 // NSUTF8StringEncoding
    ]
}

/// Attach a local image file to the notification content as its icon. The system
/// copies the file into its own store, so the caller's temp file may be deleted.
#[cfg(target_os = "macos")]
unsafe fn attach_icon_macos(content: *mut objc::runtime::Object, path: &str) {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};

    unsafe {
        let ns_path = nsstring(path);
        let url_cls = class!(NSURL);
        let file_url: *mut Object = msg_send![url_cls, fileURLWithPath: ns_path];
        let _: () = msg_send![ns_path, release];
        if file_url.is_null() {
            return;
        }

        let ns_id = nsstring("");
        let attachment_cls = class!(UNNotificationAttachment);
        let mut error: *mut Object = std::ptr::null_mut();
        let attachment: *mut Object = msg_send![
            attachment_cls,
            attachmentWithIdentifier: ns_id
            URL: file_url
            options: std::ptr::null::<Object>()
            error: &mut error
        ];
        let _: () = msg_send![ns_id, release];
        if attachment.is_null() {
            return;
        }

        let array_cls = class!(NSArray);
        let array: *mut Object = msg_send![array_cls, arrayWithObject: attachment];
        let _: () = msg_send![content, setAttachments: array];
    }
}

// ─── Windows ──────────────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn running_packaged() -> bool {
    use windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
    use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;

    let mut len = 0u32;
    unsafe { GetCurrentPackageFullName(&mut len, None) == ERROR_INSUFFICIENT_BUFFER }
}

#[cfg(target_os = "windows")]
fn init_windows() {
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    use windows::core::HSTRING;

    if running_packaged() {
        tracing::debug!("packaged install detected; toasts use the package identity");
        return;
    }

    unsafe {
        if let Err(e) = SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(APP_USER_MODEL_ID)) {
            tracing::warn!("failed to set AppUserModelID: {e}");
        }
    }

    if let Err(e) = ensure_start_menu_shortcut() {
        tracing::warn!("failed to create Start Menu shortcut; toasts may not appear: {e}");
    }
}

#[cfg(target_os = "windows")]
fn ensure_start_menu_shortcut() -> anyhow::Result<()> {
    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
    use windows::Win32::System::Com::StructuredStorage::{
        InitPropVariantFromStringAsVector, PROPVARIANT,
    };
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        IPersistFile,
    };
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::{HSTRING, Interface};

    let Some(appdata) = dirs::data_dir() else {
        anyhow::bail!("no APPDATA directory");
    };
    let shortcut = appdata
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Mezon.lnk");
    if let Some(parent) = shortcut.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let exe = std::env::current_exe()?;

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&HSTRING::from(exe.as_os_str()))?;
        if let Some(dir) = exe.parent() {
            link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
        }

        let store: IPropertyStore = link.cast()?;
        let value: PROPVARIANT =
            InitPropVariantFromStringAsVector(&HSTRING::from(APP_USER_MODEL_ID))?;
        store.SetValue(&PKEY_AppUserModel_ID, &value)?;
        store.Commit()?;

        let file: IPersistFile = link.cast()?;
        file.Save(&HSTRING::from(shortcut.as_os_str()), true)?;
    }

    tracing::info!("wrote Start Menu shortcut with AppUserModelID for toast notifications");
    Ok(())
}

/// `ToastNotification::Activated` is an in-process COM callback, so it can only
/// arrive while the apartment that raised the toast is still alive.
///
/// Raising a toast from a thread that calls `CoInitializeEx(APARTMENTTHREADED)`
/// and then `CoUninitialize` before exiting tears that apartment down
/// immediately, which is why the click only dismissed the toast. Leaving the
/// thread apartment-less instead makes `windows-rs` fall back to
/// `CoIncrementMTAUsage` when it resolves the activation factory, and that
/// keeps an implicit MTA alive for the rest of the process, so the callback
/// still has somewhere to land after the thread is gone.
#[cfg(target_os = "windows")]
fn ensure_toast_apartment() {
    use windows::Win32::System::Com::CoIncrementMTAUsage;

    static MTA: std::sync::OnceLock<()> = std::sync::OnceLock::new();

    MTA.get_or_init(|| unsafe {
        if let Err(e) = CoIncrementMTAUsage() {
            tracing::warn!("could not join the process MTA for toasts: {e}");
        }
    });
}

#[cfg(target_os = "windows")]
fn retain_toast(toast: windows::UI::Notifications::ToastNotification) {
    use windows::UI::Notifications::ToastNotification;

    const LIVE_TOAST_LIMIT: usize = 32;

    static LIVE: std::sync::Mutex<Vec<ToastNotification>> = std::sync::Mutex::new(Vec::new());

    let Ok(mut live) = LIVE.lock() else {
        return;
    };
    if live.len() >= LIVE_TOAST_LIMIT {
        live.remove(0);
    }
    live.push(toast);
}

#[cfg(target_os = "windows")]
fn show_windows(n: &Notification) {
    let title = n.title.clone();
    let body = n.body.clone();
    let tag = replacement_key(n);
    let icon_path = n.icon_path.clone();

    std::thread::spawn(move || {
        ensure_toast_apartment();
        match try_show_toast(&title, &body, tag.as_deref(), icon_path.as_deref()) {
            Ok(toast) => retain_toast(toast),
            Err(e) => tracing::warn!("Windows toast notification failed: {e}"),
        }
    });
}

#[cfg(target_os = "windows")]
fn try_show_toast(
    title: &str,
    body: &str,
    tag: Option<&str>,
    icon_path: Option<&str>,
) -> windows::core::Result<windows::UI::Notifications::ToastNotification> {
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::core::HSTRING;

    let image_xml = match icon_path {
        Some(path) => format!(
            r#"<image placement="appLogoOverride" hint-crop="circle" src="file:///{}"/>"#,
            escape_xml(&path.replace('\\', "/"))
        ),
        None => String::new(),
    };
    let xml_str = format!(
        r#"<toast>
  <visual>
    <binding template="ToastGeneric">
      {image_xml}
      <text>{}</text>
      <text>{}</text>
    </binding>
  </visual>
</toast>"#,
        escape_xml(title),
        escape_xml(body)
    );

    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(xml_str))?;

    let notifier = if running_packaged() {
        ToastNotificationManager::CreateToastNotifier()?
    } else {
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_USER_MODEL_ID))?
    };
    let toast = ToastNotification::CreateToastNotification(&doc)?;
    if let Some(tag) = tag {
        let tag: String = tag.chars().take(64).collect();
        toast.SetTag(&HSTRING::from(tag.as_str()))?;
        toast.SetGroup(&HSTRING::from("mezon"))?;
    }

    let activation_target = tag.and_then(|t| build_target(&t));
    toast.Activated(&windows::Foundation::TypedEventHandler::<
        ToastNotification,
        windows::core::IInspectable,
    >::new(move |_, _| {
        if let Some(target) = activation_target.clone() {
            dispatch_click(target);
        }
        Ok(())
    }))?;

    notifier.Show(&toast)?;
    Ok(toast)
}

#[cfg(target_os = "windows")]
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

// ─── Linux ────────────────────────────────────────────────────────────────────
//
// Uses a single persistent zbus session connection for all notifications so that
// GNOME's FdoNotificationDaemonSource watches one stable D-Bus unique name that
// lives for the whole process, eliminating the per-app-Source destruction that
// occurred when individual per-toast connections were dropped.

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const LINUX_DESKTOP_ENTRY: &str = "mezon";

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const GATE_WAITING: u8 = 0;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const GATE_CLOSE: u8 = 1;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const GATE_SUPERSEDED: u8 = 2;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
struct CloseGate {
    action: std::sync::atomic::AtomicU8,
    waker: std::sync::Mutex<Option<std::task::Waker>>,
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl CloseGate {
    fn new() -> Self {
        Self {
            action: std::sync::atomic::AtomicU8::new(GATE_WAITING),
            waker: std::sync::Mutex::new(None),
        }
    }

    fn request(&self, action: u8) {
        self.action
            .store(action, std::sync::atomic::Ordering::Release);
        self.wake();
    }

    fn supersede(&self) {
        if self
            .action
            .compare_exchange(
                GATE_WAITING,
                GATE_SUPERSEDED,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .is_ok()
        {
            self.wake();
        }
    }

    fn wake(&self) {
        if let Some(waker) = self
            .waker
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
        {
            waker.wake();
        }
    }

    fn action(&self) -> u8 {
        self.action.load(std::sync::atomic::Ordering::Acquire)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
struct CloseWait {
    gate: std::sync::Arc<CloseGate>,
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl std::future::Future for CloseWait {
    type Output = u8;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<u8> {
        let action = self.gate.action();
        if action != GATE_WAITING {
            return std::task::Poll::Ready(action);
        }
        *self
            .gate
            .waker
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(cx.waker().clone());
        let action = self.gate.action();
        if action != GATE_WAITING {
            std::task::Poll::Ready(action)
        } else {
            std::task::Poll::Pending
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn live_gates()
-> &'static std::sync::Mutex<std::collections::HashMap<String, Vec<std::sync::Arc<CloseGate>>>> {
    static GATES: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, Vec<std::sync::Arc<CloseGate>>>>,
    > = std::sync::OnceLock::new();
    GATES.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn lock_gates()
-> std::sync::MutexGuard<'static, std::collections::HashMap<String, Vec<std::sync::Arc<CloseGate>>>>
{
    live_gates()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn push_gate(key: &str, gate: std::sync::Arc<CloseGate>) {
    lock_gates().entry(key.to_owned()).or_default().push(gate);
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn take_gates(key: &str) -> Vec<std::sync::Arc<CloseGate>> {
    lock_gates().remove(key).unwrap_or_default()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn release_gate(key: &str, gate: &std::sync::Arc<CloseGate>) {
    let mut gates = lock_gates();
    let Some(live) = gates.get_mut(key) else {
        return;
    };
    live.retain(|current| !std::sync::Arc::ptr_eq(current, gate));
    if live.is_empty() {
        gates.remove(key);
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn gate_channel_key(channel_id: &str) -> Option<String> {
    let id = channel_id.trim();
    if id.is_empty() || id == "0" {
        None
    } else {
        Some(id.to_owned())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
struct ChannelToastSlot {
    generation: u64,
    id: u32,
    gate: std::sync::Arc<CloseGate>,
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
struct ChannelToastBook {
    slots: std::collections::HashMap<String, ChannelToastSlot>,
    turns: std::collections::HashMap<String, std::sync::Arc<std::sync::Mutex<()>>>,
    next_generation: u64,
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl ChannelToastBook {
    fn new() -> Self {
        Self {
            slots: std::collections::HashMap::new(),
            turns: std::collections::HashMap::new(),
            next_generation: 0,
        }
    }

    fn replaces_id(&self, key: &str) -> u32 {
        self.slots.get(key).map(|slot| slot.id).unwrap_or(0)
    }

    fn turn(&mut self, key: &str) -> std::sync::Arc<std::sync::Mutex<()>> {
        self.turns
            .entry(key.to_owned())
            .or_insert_with(|| std::sync::Arc::new(std::sync::Mutex::new(())))
            .clone()
    }

    fn publish(&mut self, key: &str, id: u32, gate: std::sync::Arc<CloseGate>) -> u64 {
        self.next_generation += 1;
        let generation = self.next_generation;
        if let Some(previous) = self.slots.insert(
            key.to_owned(),
            ChannelToastSlot {
                generation,
                id,
                gate,
            },
        ) {
            previous.gate.supersede();
        }
        generation
    }

    fn retire(&mut self, key: &str, generation: u64) {
        if self
            .slots
            .get(key)
            .is_some_and(|slot| slot.generation == generation)
        {
            self.slots.remove(key);
        }
    }

    fn owns(&self, key: &str, generation: u64) -> bool {
        self.slots
            .get(key)
            .is_some_and(|slot| slot.generation == generation)
    }

    fn clear(&mut self) {
        self.slots.clear();
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn toast_book() -> std::sync::MutexGuard<'static, ChannelToastBook> {
    static BOOK: std::sync::OnceLock<std::sync::Mutex<ChannelToastBook>> =
        std::sync::OnceLock::new();
    BOOK.get_or_init(|| std::sync::Mutex::new(ChannelToastBook::new()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn session_connection() -> Option<&'static zbus::Connection> {
    static CONN: std::sync::OnceLock<Option<zbus::Connection>> = std::sync::OnceLock::new();
    CONN.get_or_init(|| {
        pollster::block_on(async {
            let conn = zbus::Connection::session().await.ok()?;
            let proxy = zbus::fdo::DBusProxy::new(&conn).await.ok()?;
            for member in ["ActionInvoked", "NotificationClosed"] {
                let rule = zbus::MatchRule::builder()
                    .msg_type(zbus::message::Type::Signal)
                    .interface("org.freedesktop.Notifications")
                    .ok()?
                    .member(member)
                    .ok()?
                    .build();
                proxy.add_match_rule(rule).await.ok()?;
            }
            Some(conn)
        })
    })
    .as_ref()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
async fn dbus_notify(
    conn: &zbus::Connection,
    summary: &str,
    body: &str,
    icon: &str,
    has_action: bool,
    replaces_id: u32,
    with_sound: bool,
) -> Result<u32, zbus::Error> {
    let actions: &[&str] = if has_action {
        &["default", "Open"]
    } else {
        &[]
    };
    let mut hints = std::collections::HashMap::<&str, zbus::zvariant::Value<'_>>::new();
    hints.insert("desktop-entry", zbus::zvariant::Value::Str("mezon".into()));
    if with_sound {
        hints.insert(
            "sound-name",
            zbus::zvariant::Value::Str("message-new-instant".into()),
        );
    }
    conn.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &(
            "Mezon",
            replaces_id,
            icon,
            summary,
            body,
            actions,
            &hints,
            -1i32,
        ),
    )
    .await?
    .body()
    .deserialize()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
async fn dbus_close_id(conn: &zbus::Connection, id: u32) {
    let _ = conn
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "CloseNotification",
            &(id,),
        )
        .await;
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
async fn wait_for_notification(conn: &zbus::Connection, id: u32, on_action: impl FnOnce(&str)) {
    use futures_lite::StreamExt as _;
    let mut stream = zbus::MessageStream::from(conn);
    while let Some(Ok(msg)) = stream.next().await {
        let header = msg.header();
        if !matches!(header.message_type(), zbus::message::Type::Signal) {
            continue;
        }
        match header.member() {
            Some(m) if m == "ActionInvoked" => {
                if let Ok((nid, action)) = msg.body().deserialize::<(u32, String)>()
                    && nid == id
                {
                    on_action(&action);
                    return;
                }
            }
            Some(m) if m == "NotificationClosed" => {
                if let Ok((nid, _)) = msg.body().deserialize::<(u32, u32)>()
                    && nid == id
                {
                    return;
                }
            }
            _ => {}
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn notification_clicked_action(action: &str) -> bool {
    action.is_empty() || action == "default"
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn drive_shown(
    conn: &zbus::Connection,
    id: u32,
    gate: std::sync::Arc<CloseGate>,
    target: Option<NotificationTarget>,
    channel_key: Option<String>,
    generation: u64,
) {
    if gate.action() == GATE_SUPERSEDED {
        return;
    }
    if gate.action() == GATE_CLOSE {
        pollster::block_on(dbus_close_id(conn, id));
        return;
    }
    let action = pollster::block_on(async {
        let gate_ref = std::sync::Arc::clone(&gate);
        let mut wait = std::pin::pin!(wait_for_notification(conn, id, |action| {
            let current = channel_key
                .as_deref()
                .is_none_or(|key| toast_book().owns(key, generation));
            if current
                && notification_clicked_action(action)
                && let Some(t) = target
            {
                dispatch_click(t);
            }
        }));
        let mut close = std::pin::pin!(CloseWait { gate: gate_ref });
        std::future::poll_fn(|cx| match close.as_mut().poll(cx) {
            std::task::Poll::Ready(v) => std::task::Poll::Ready(v),
            std::task::Poll::Pending => {
                if wait.as_mut().poll(cx).is_ready() {
                    std::task::Poll::Ready(GATE_WAITING)
                } else {
                    std::task::Poll::Pending
                }
            }
        })
        .await
    });
    if action == GATE_CLOSE {
        pollster::block_on(dbus_close_id(conn, id));
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
struct GateGuard {
    key: Option<String>,
    gate: std::sync::Arc<CloseGate>,
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl Drop for GateGuard {
    fn drop(&mut self) {
        if let Some(key) = self.key.as_deref() {
            release_gate(key, &self.gate);
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn show_linux(n: &Notification) {
    let Some(conn) = session_connection() else {
        tracing::warn!("Linux notification skipped: no D-Bus session connection");
        return;
    };
    let title = n.title.clone();
    let body = n.body.clone();
    let channel_key = n.channel_id.as_deref().and_then(gate_channel_key);
    let activation_target = replacement_key(n).as_deref().and_then(build_target);
    let icon = n
        .icon_path
        .clone()
        .unwrap_or_else(|| LINUX_DESKTOP_ENTRY.to_owned());
    let gate = std::sync::Arc::new(CloseGate::new());
    if let Some(key) = channel_key.clone() {
        push_gate(&key, std::sync::Arc::clone(&gate));
    }
    let conn = conn.clone();

    std::thread::spawn(move || {
        let _guard = GateGuard {
            key: channel_key.clone(),
            gate: std::sync::Arc::clone(&gate),
        };
        let shown = {
            let turn = channel_key.as_deref().map(|key| toast_book().turn(key));
            let _turn = turn
                .as_ref()
                .map(|turn| turn.lock().unwrap_or_else(|poison| poison.into_inner()));
            if gate.action() == GATE_CLOSE {
                None
            } else {
                let replaces_id = channel_key
                    .as_deref()
                    .map(|key| toast_book().replaces_id(key))
                    .unwrap_or(0);
                match pollster::block_on(dbus_notify(
                    &conn,
                    &title,
                    &body,
                    &icon,
                    activation_target.is_some(),
                    replaces_id,
                    replaces_id == 0,
                )) {
                    Ok(id) if id > 0 => {
                        let generation = channel_key
                            .as_deref()
                            .map(|key| toast_book().publish(key, id, std::sync::Arc::clone(&gate)));
                        Some((id, generation.unwrap_or(0)))
                    }
                    Ok(_) => {
                        tracing::warn!("Linux notification: daemon returned id=0");
                        None
                    }
                    Err(e) => {
                        tracing::warn!("Linux notification failed: {e}");
                        None
                    }
                }
            }
        };
        let Some((id, generation)) = shown else {
            return;
        };
        drive_shown(
            &conn,
            id,
            std::sync::Arc::clone(&gate),
            activation_target,
            channel_key.clone(),
            generation,
        );
        if let Some(key) = channel_key.as_deref() {
            let turn = toast_book().turn(key);
            let _turn = turn.lock().unwrap_or_else(|poison| poison.into_inner());
            toast_book().retire(key, generation);
        }
    });
}

pub fn close_channel(clan_id: Option<&str>, channel_id: &str) {
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    close_linux_channel(clan_id, channel_id);
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let _ = (clan_id, channel_id);
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn close_linux_channel(clan_id: Option<&str>, channel_id: &str) {
    let _ = clan_id;
    let Some(key) = gate_channel_key(channel_id) else {
        return;
    };
    for gate in take_gates(&key) {
        gate.request(GATE_CLOSE);
    }
}

pub fn close_all() {
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let all: Vec<std::sync::Arc<CloseGate>> = {
            let mut gates = lock_gates();
            let collected: Vec<_> = gates.values().flatten().cloned().collect();
            gates.clear();
            collected
        };
        for gate in all {
            gate.request(GATE_CLOSE);
        }
        toast_book().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(channel_id: Option<&str>, clan_id: Option<&str>) -> Notification {
        Notification {
            title: "t".into(),
            body: "b".into(),
            channel_id: channel_id.map(str::to_owned),
            clan_id: clan_id.map(str::to_owned),
            link: None,
            topic_reply: None,
            icon_path: None,
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn dock_gate_key_ignores_the_clan_resolved_at_show_time() {
        assert_eq!(gate_channel_key("12345").as_deref(), Some("12345"));
        assert_eq!(gate_channel_key(" 12345 ").as_deref(), Some("12345"));
        assert_eq!(gate_channel_key("0"), None);
        assert_eq!(gate_channel_key(""), None);
        assert_eq!(gate_channel_key("12345"), gate_channel_key("12345"));
        assert_ne!(
            replacement_key(&notification(Some("12345"), None)),
            replacement_key(&notification(Some("12345"), Some("77")))
        );
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn gate_guard_releases_gate_on_drop() {
        let key = "test-gate-guard-releases";
        let gate = std::sync::Arc::new(CloseGate::new());
        push_gate(key, std::sync::Arc::clone(&gate));
        assert!(!lock_gates().get(key).map(|v| v.is_empty()).unwrap_or(true));
        let guard = GateGuard {
            key: Some(key.to_owned()),
            gate: std::sync::Arc::clone(&gate),
        };
        drop(guard);
        assert!(lock_gates().get(key).is_none());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn a_channel_replaces_its_live_toast_until_that_toast_ends() {
        let mut book = ChannelToastBook::new();
        let first = std::sync::Arc::new(CloseGate::new());
        let second = std::sync::Arc::new(CloseGate::new());
        let other = std::sync::Arc::new(CloseGate::new());

        assert_eq!(book.replaces_id("channel-a"), 0);
        let first_generation = book.publish("channel-a", 41, std::sync::Arc::clone(&first));
        assert_eq!(book.replaces_id("channel-a"), 41);

        let second_generation = book.publish("channel-a", 41, std::sync::Arc::clone(&second));
        assert_eq!(first.action(), GATE_SUPERSEDED);
        assert_eq!(book.replaces_id("channel-a"), 41);
        assert!(book.owns("channel-a", second_generation));
        assert!(!book.owns("channel-a", first_generation));

        book.retire("channel-a", first_generation);
        assert_eq!(book.replaces_id("channel-a"), 41);

        let other_generation = book.publish("channel-b", 77, other);
        assert_eq!(second.action(), GATE_WAITING);
        assert_eq!(book.replaces_id("channel-a"), 41);
        assert_eq!(book.replaces_id("channel-b"), 77);

        book.retire("channel-a", second_generation);
        book.retire("channel-b", other_generation);
        assert_eq!(book.replaces_id("channel-a"), 0);
        assert_eq!(book.replaces_id("channel-b"), 0);
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn supersede_leaves_a_toast_that_is_already_closing() {
        let gate = std::sync::Arc::new(CloseGate::new());
        gate.request(GATE_CLOSE);
        gate.supersede();
        assert_eq!(gate.action(), GATE_CLOSE);
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn close_all_clears_every_gate() {
        let key_a = "test-close-all-a";
        let key_b = "test-close-all-b";
        let gate_a = std::sync::Arc::new(CloseGate::new());
        let gate_b = std::sync::Arc::new(CloseGate::new());
        push_gate(key_a, std::sync::Arc::clone(&gate_a));
        push_gate(key_b, std::sync::Arc::clone(&gate_b));
        close_all();
        assert_eq!(gate_a.action(), GATE_CLOSE);
        assert_eq!(gate_b.action(), GATE_CLOSE);
        assert!(lock_gates().get(key_a).is_none());
        assert!(lock_gates().get(key_b).is_none());
    }

    #[test]
    fn replacement_key_is_stable_per_channel() {
        let a = replacement_key(&notification(Some("12345"), Some("77")));
        assert_eq!(a.as_deref(), Some("mezon-notify-77-12345"));
        assert_eq!(a, replacement_key(&notification(Some("12345"), Some("77"))));
        assert_ne!(a, replacement_key(&notification(Some("12346"), Some("77"))));
    }

    #[test]
    fn replacement_key_rejects_absent_blank_and_zero_channel_ids() {
        assert_eq!(replacement_key(&notification(None, Some("77"))), None);
        assert_eq!(replacement_key(&notification(Some(""), Some("77"))), None);
        assert_eq!(
            replacement_key(&notification(Some("   "), Some("77"))),
            None
        );
        assert_eq!(replacement_key(&notification(Some("0"), Some("77"))), None);
    }

    #[test]
    fn identifier_round_trips_clan_and_channel() {
        let key = replacement_key(&notification(Some("12345"), Some("77"))).unwrap();
        assert_eq!(
            decode_identifier(&key),
            Some(NotificationTarget {
                clan_id: Some("77".into()),
                channel_id: "12345".into(),
                link: None,
                topic_reply: None,
            })
        );
    }

    #[test]
    fn identifier_round_trips_dm_without_clan() {
        let key = replacement_key(&notification(Some("12345"), None)).unwrap();
        assert_eq!(key, "mezon-notify-0-12345");
        assert_eq!(
            decode_identifier(&key),
            Some(NotificationTarget {
                clan_id: None,
                channel_id: "12345".into(),
                link: None,
                topic_reply: None,
            })
        );
    }

    #[test]
    fn a_click_reopens_the_topic_reply_until_the_channel_posts_outside_it() {
        let reply = TopicReply {
            topic_id: "2088098374732484608".into(),
            message_id: "1840651254099873792".into(),
        };
        let key = "mezon-notify-77-424242";
        remember_topic_reply(key, Some(&reply));
        assert_eq!(build_target(key).unwrap().topic_reply, Some(reply));
        remember_topic_reply(key, None);
        assert_eq!(build_target(key).unwrap().topic_reply, None);
    }

    #[test]
    fn decode_identifier_rejects_foreign_and_malformed_ids() {
        assert_eq!(decode_identifier("some-other-app-1-2"), None);
        assert_eq!(decode_identifier("mezon-notify"), None);
        assert_eq!(decode_identifier("mezon-notify-77"), None);
        assert_eq!(decode_identifier("mezon-notify-77-"), None);
        assert_eq!(decode_identifier(""), None);
    }
}
