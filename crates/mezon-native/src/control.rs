use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::Arc;

pub const APP_NOT_RUNNING_MSG: &str =
    "Mezon app is not running. Open the Mezon desktop app, then retry this command.";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlRequest {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlResponse {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ControlResponse {
    pub fn ok(id: u64, result: Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: u64, error: impl Into<String>) -> Self {
        Self {
            id,
            result: None,
            error: Some(error.into()),
        }
    }
}

pub type ControlHandler = Arc<dyn Fn(ControlRequest) -> ControlResponse + Send + Sync>;

pub struct ControlClient;

impl ControlClient {
    pub fn request(method: &str, params: Value) -> anyhow::Result<Value> {
        let request = ControlRequest {
            id: 1,
            method: method.to_string(),
            params,
        };
        let response = Self::send(request)?;
        if let Some(error) = response.error {
            anyhow::bail!(error);
        }
        response
            .result
            .ok_or_else(|| anyhow::anyhow!("Control response missing result"))
    }

    fn send(request: ControlRequest) -> anyhow::Result<ControlResponse> {
        #[cfg(unix)]
        return Self::send_unix(request);

        #[cfg(windows)]
        return Self::send_windows(request);

        #[cfg(not(any(unix, windows)))]
        {
            let _ = request;
            anyhow::bail!(APP_NOT_RUNNING_MSG)
        }
    }

    #[cfg(unix)]
    fn send_unix(request: ControlRequest) -> anyhow::Result<ControlResponse> {
        use std::os::unix::net::UnixStream;

        let mut last_error = None;
        for path in control_socket_paths() {
            match UnixStream::connect(&path) {
                Ok(stream) => return Self::exchange(stream, request),
                Err(e) => last_error = Some(e),
            }
        }
        let error = last_error
            .map(|e| e.to_string())
            .unwrap_or_else(|| "No control socket path".to_string());
        anyhow::bail!("{APP_NOT_RUNNING_MSG} ({error})")
    }

    #[cfg(windows)]
    fn send_windows(request: ControlRequest) -> anyhow::Result<ControlResponse> {
        use std::fs::OpenOptions;

        let mut pipe = OpenOptions::new()
            .read(true)
            .write(true)
            .open(PIPE_NAME)
            .map_err(|_| anyhow::anyhow!(APP_NOT_RUNNING_MSG))?;
        Self::exchange(&mut pipe, request)
    }

    fn exchange(
        mut stream: impl Read + Write,
        request: ControlRequest,
    ) -> anyhow::Result<ControlResponse> {
        let payload = serde_json::to_string(&request)?;
        writeln!(stream, "{payload}")?;
        stream.flush()?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line.trim().is_empty() {
            anyhow::bail!("Empty control response");
        }
        Ok(serde_json::from_str(line.trim())?)
    }
}

pub struct ControlServer {
    handler: ControlHandler,
    #[cfg(unix)]
    _socket_path: Option<std::path::PathBuf>,
    #[cfg(unix)]
    listener: Option<std::os::unix::net::UnixListener>,
    #[cfg(windows)]
    stop_tx: Option<std::sync::mpsc::Sender<()>>,
}

impl ControlServer {
    pub fn bind(handler: ControlHandler) -> anyhow::Result<Self> {
        #[cfg(unix)]
        return Self::bind_unix(handler);

        #[cfg(windows)]
        return Self::bind_windows(handler);

        #[cfg(not(any(unix, windows)))]
        {
            let _ = handler;
            Ok(Self {})
        }
    }

    #[cfg(unix)]
    fn bind_unix(handler: ControlHandler) -> anyhow::Result<Self> {
        use std::os::unix::net::UnixStream;

        let mut last_error = None;
        for path in control_socket_paths() {
            if let Some(parent) = path.parent()
                && let Err(e) = std::fs::create_dir_all(parent)
            {
                last_error = Some(e);
                continue;
            }
            if path.exists() {
                match UnixStream::connect(&path) {
                    Ok(_) => {
                        tracing::debug!("Control socket already in use at {}", path.display());
                        last_error = Some(std::io::Error::new(
                            std::io::ErrorKind::AddrInUse,
                            format!("Control socket already in use at {}", path.display()),
                        ));
                        continue;
                    }
                    Err(_) => {
                        // Stale socket file — remove before rebinding.
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
            match std::os::unix::net::UnixListener::bind(&path) {
                Ok(listener) => {
                    tracing::debug!("Control server listening at {}", path.display());
                    return Ok(Self {
                        handler,
                        _socket_path: Some(path),
                        listener: Some(listener),
                    });
                }
                Err(e) => last_error = Some(e),
            }
        }
        Err(last_error
            .unwrap_or_else(|| std::io::Error::other("No control socket path"))
            .into())
    }

    #[cfg(windows)]
    const PIPE_NAME: &'static str = r"\\.\pipe\mezon-control";

    #[cfg(windows)]
    fn bind_windows(handler: ControlHandler) -> anyhow::Result<Self> {
        let (stop_tx, stop_rx) = std::sync::mpsc::channel();
        let handler_for_thread = handler.clone();
        std::thread::Builder::new()
            .name("mezon-control-server".into())
            .spawn(move || Self::windows_server_loop(handler_for_thread, stop_rx))
            .map_err(|e| anyhow::anyhow!("Failed to spawn control server thread: {e}"))?;
        Ok(Self {
            handler,
            stop_tx: Some(stop_tx),
        })
    }

    pub fn run_in_background(&self) {
        #[cfg(unix)]
        self.run_unix_background();

        #[cfg(windows)]
        {
            let _ = self;
        }

        #[cfg(not(any(unix, windows)))]
        {
            let _ = self;
        }
    }

    #[cfg(unix)]
    fn run_unix_background(&self) {
        let Some(listener) = self
            .listener
            .as_ref()
            .and_then(|listener| listener.try_clone().ok())
        else {
            return;
        };
        let handler = self.handler.clone();
        std::thread::Builder::new()
            .name("mezon-control-server".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    match stream {
                        Ok(stream) => {
                            if let Err(e) = Self::serve_connection(stream, handler.clone()) {
                                tracing::debug!("Control connection error: {e}");
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Control accept error: {e}");
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }
                    }
                }
            })
            .map_err(|e| tracing::error!("Failed to spawn control server thread: {e}"))
            .ok();
    }

    #[cfg(unix)]
    fn serve_connection(
        stream: std::os::unix::net::UnixStream,
        handler: ControlHandler,
    ) -> anyhow::Result<()> {
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut writer = stream;
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line.trim().is_empty() {
            return Ok(());
        }
        let request: ControlRequest = serde_json::from_str(line.trim())?;
        let response = handler(request);
        let payload = serde_json::to_string(&response)?;
        writeln!(writer, "{payload}")?;
        writer.flush()?;
        Ok(())
    }

    #[cfg(windows)]
    fn windows_server_loop(handler: ControlHandler, stop_rx: std::sync::mpsc::Receiver<()>) {
        use std::time::Duration;
        use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE};
        use windows::Win32::Storage::FileSystem::{
            FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX,
        };
        use windows::Win32::System::Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
            PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        };

        let pipe_name: Vec<u16> = Self::PIPE_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        loop {
            if stop_rx.try_recv().is_ok() {
                break;
            }

            let handle = unsafe {
                CreateNamedPipeW(
                    windows::core::PCWSTR(pipe_name.as_ptr()),
                    PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                    PIPE_UNLIMITED_INSTANCES,
                    65536,
                    65536,
                    0,
                    None,
                )
            };

            if handle == INVALID_HANDLE_VALUE {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }

            let connected = match unsafe { ConnectNamedPipe(handle, None) } {
                Ok(()) => true,
                Err(e) => e.code() == ERROR_PIPE_CONNECTED.to_hresult(),
            };

            if connected {
                let raw = handle.0 as usize;
                let handler = handler.clone();
                std::thread::spawn(move || {
                    let h = windows::Win32::Foundation::HANDLE(raw as *mut std::ffi::c_void);
                    if let Err(e) = Self::serve_windows_handle(h, handler) {
                        tracing::debug!("Windows control connection error: {e}");
                    }
                    unsafe {
                        let _ = DisconnectNamedPipe(h);
                        let _ = windows::Win32::Foundation::CloseHandle(h);
                    }
                });
            } else {
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(handle);
                }
            }
        }
    }

    #[cfg(windows)]
    fn serve_windows_handle(
        handle: windows::Win32::Foundation::HANDLE,
        handler: ControlHandler,
    ) -> anyhow::Result<()> {
        use windows::Win32::Storage::FileSystem::ReadFile;
        use windows::Win32::Storage::FileSystem::WriteFile;

        let mut buf = [0u8; 65536];
        let mut bytes_read = 0u32;
        let ok = unsafe { ReadFile(handle, Some(&mut buf), Some(&mut bytes_read), None).is_ok() };
        if !ok || bytes_read == 0 {
            return Ok(());
        }
        let line = std::str::from_utf8(&buf[..bytes_read as usize])?.trim();
        if line.is_empty() {
            return Ok(());
        }
        let request: ControlRequest = serde_json::from_str(line)?;
        let response = handler(request);
        let payload = format!("{}\n", serde_json::to_string(&response)?);
        let payload = payload.as_bytes();
        let mut bytes_written = 0u32;
        unsafe {
            WriteFile(handle, Some(payload), Some(&mut bytes_written), None)?;
        }
        Ok(())
    }
}

#[cfg(windows)]
const PIPE_NAME: &str = r"\\.\pipe\mezon-control";

#[cfg(unix)]
pub fn control_socket_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if let Ok(override_path) = std::env::var("MEZON_CONTROL_SOCKET")
        && !override_path.is_empty()
    {
        paths.push(std::path::PathBuf::from(override_path));
        return paths;
    }
    if let Some(runtime_dir) = dirs::runtime_dir() {
        paths.push(runtime_dir.join("mezon-ctl.sock"));
    }
    let user = std::env::var("USER")
        .ok()
        .filter(|user| !user.is_empty())
        .unwrap_or_else(|| "user".to_owned());
    paths.push(
        std::env::temp_dir()
            .join(format!("mezon-desktop-{user}"))
            .join("mezon-ctl.sock"),
    );
    paths
}
