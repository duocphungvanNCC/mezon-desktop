pub use imp::*;

#[cfg(target_os = "windows")]
mod imp {
    use std::sync::atomic::{AtomicIsize, Ordering};

    use anyhow::{Context as _, Result, anyhow, bail};
    use windows::Services::Store::{
        StoreContext, StorePackageUpdate, StorePackageUpdateResult, StorePackageUpdateState,
        StorePackageUpdateStatus,
    };
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Recovery::{
        REGISTER_APPLICATION_RESTART_FLAGS, RegisterApplicationRestart,
    };
    use windows::Win32::UI::Shell::IInitializeWithWindow;
    use windows::core::{Interface, PCWSTR};
    use windows_collections::{IIterable, IVectorView};
    use windows_future::AsyncOperationProgressHandler;

    static WINDOW_HANDLE: AtomicIsize = AtomicIsize::new(0);

    pub fn set_window_handle(hwnd: isize) {
        WINDOW_HANDLE.store(hwnd, Ordering::Relaxed);
    }

    fn store_context() -> Result<StoreContext> {
        let context = StoreContext::GetDefault()
            .context("StoreContext unavailable (app not installed from the Microsoft Store?)")?;
        let hwnd = WINDOW_HANDLE.load(Ordering::Relaxed);
        if hwnd != 0
            && let Ok(init) = context.cast::<IInitializeWithWindow>()
        {
            unsafe {
                let _ = init.Initialize(HWND(hwnd as *mut core::ffi::c_void));
            }
        }
        Ok(context)
    }

    async fn pending_updates(context: &StoreContext) -> Result<IVectorView<StorePackageUpdate>> {
        context
            .GetAppAndOptionalStorePackageUpdatesAsync()
            .context("GetAppAndOptionalStorePackageUpdatesAsync failed")?
            .await
            .context("Microsoft Store update check failed")
    }

    fn updates_iterable(
        updates: &IVectorView<StorePackageUpdate>,
    ) -> Result<IIterable<StorePackageUpdate>> {
        updates.cast().context("store update list is not iterable")
    }

    fn update_failure_reason(state: StorePackageUpdateState) -> String {
        if state == StorePackageUpdateState::Canceled {
            "it was canceled".to_owned()
        } else if state == StorePackageUpdateState::ErrorLowBattery {
            "the device battery is too low; plug in and retry".to_owned()
        } else if state == StorePackageUpdateState::ErrorWiFiRequired
            || state == StorePackageUpdateState::ErrorWiFiRecommended
        {
            "a Wi-Fi or unmetered connection is required; connect and retry".to_owned()
        } else if state == StorePackageUpdateState::OtherError {
            "open the Microsoft Store (Library > Get updates) or run wsreset, then retry".to_owned()
        } else {
            format!("unexpected state {state:?}")
        }
    }

    fn ensure_completed(state: StorePackageUpdateState, action: &str) -> Result<()> {
        if state == StorePackageUpdateState::Completed {
            Ok(())
        } else {
            Err(anyhow!(
                "Microsoft Store {action} couldn't complete - {}",
                update_failure_reason(state)
            ))
        }
    }

    pub async fn download_updates(on_progress: Box<dyn Fn(Option<f32>) + Send>) -> Result<()> {
        let context = store_context()?;
        let updates = pending_updates(&context).await?;
        if updates.Size().unwrap_or(0) == 0 {
            bail!("no pending Microsoft Store updates");
        }
        let operation = context
            .RequestDownloadStorePackageUpdatesAsync(&updates_iterable(&updates)?)
            .context("RequestDownloadStorePackageUpdatesAsync failed")?;
        let handler = AsyncOperationProgressHandler::<
            StorePackageUpdateResult,
            StorePackageUpdateStatus,
        >::new(move |_, status| {
            if let Some(status) = status.as_ref() {
                on_progress(Some(
                    (status.PackageDownloadProgress as f32).clamp(0.0, 1.0),
                ));
            }
            Ok(())
        });
        let _ = operation.SetProgress(&handler);
        let result = operation.await.context("Microsoft Store download failed")?;
        ensure_completed(
            result
                .OverallState()
                .context("download result state unavailable")?,
            "download",
        )
    }

    pub async fn install_updates() -> Result<()> {
        let context = store_context()?;
        let updates = pending_updates(&context).await?;
        if updates.Size().unwrap_or(0) == 0 {
            bail!("no pending Microsoft Store updates");
        }
        unsafe {
            let _ =
                RegisterApplicationRestart(PCWSTR::null(), REGISTER_APPLICATION_RESTART_FLAGS(0));
        }
        let operation = context
            .RequestDownloadAndInstallStorePackageUpdatesAsync(&updates_iterable(&updates)?)
            .context("RequestDownloadAndInstallStorePackageUpdatesAsync failed")?;
        let result = operation.await.context("Microsoft Store install failed")?;
        ensure_completed(
            result
                .OverallState()
                .context("install result state unavailable")?,
            "install",
        )
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    use anyhow::{Result, bail};

    pub fn set_window_handle(_hwnd: isize) {}

    pub async fn download_updates(_on_progress: Box<dyn Fn(Option<f32>) + Send>) -> Result<()> {
        bail!("Microsoft Store updates are only available on Windows")
    }

    pub async fn install_updates() -> Result<()> {
        bail!("Microsoft Store updates are only available on Windows")
    }
}
