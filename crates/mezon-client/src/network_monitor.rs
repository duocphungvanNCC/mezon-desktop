use tokio::sync::watch;

pub struct NetworkMonitor {
    online: watch::Receiver<bool>,
    #[cfg(target_os = "macos")]
    _thread: std::thread::JoinHandle<()>,
    #[cfg(not(target_os = "macos"))]
    _keep_open: watch::Sender<bool>,
}

impl NetworkMonitor {
    pub fn new() -> Self {
        #[cfg(target_os = "macos")]
        {
            let (tx, online) = watch::channel(true);
            let thread = std::thread::Builder::new()
                .name("mezon-netmon".into())
                .spawn(move || macos::run(tx))
                .expect("failed to spawn network monitor thread");
            Self {
                online,
                _thread: thread,
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let (keep_open, online) = watch::channel(true);
            Self {
                online,
                _keep_open: keep_open,
            }
        }
    }

    pub fn is_online(&self) -> bool {
        *self.online.borrow()
    }

    pub fn online(&self) -> watch::Receiver<bool> {
        self.online.clone()
    }
}

impl Default for NetworkMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use core_foundation::runloop::{CFRunLoop, kCFRunLoopCommonModes};
    use std::net::SocketAddr;
    use system_configuration::network_reachability::{ReachabilityFlags, SCNetworkReachability};
    use tokio::sync::watch;

    fn flags_online(flags: ReachabilityFlags) -> bool {
        flags.contains(ReachabilityFlags::REACHABLE)
            && !flags.contains(ReachabilityFlags::CONNECTION_REQUIRED)
    }

    pub fn run(tx: watch::Sender<bool>) {
        let addr = SocketAddr::from(([0, 0, 0, 0], 0));
        let mut reachability = SCNetworkReachability::from(addr);

        if let Ok(flags) = reachability.reachability() {
            let _ = tx.send(flags_online(flags));
        }

        if reachability
            .set_callback(move |flags| {
                let _ = tx.send(flags_online(flags));
            })
            .is_err()
        {
            tracing::warn!("network monitor: failed to set reachability callback");
            return;
        }

        let scheduled = unsafe {
            reachability.schedule_with_runloop(&CFRunLoop::get_current(), kCFRunLoopCommonModes)
        };
        if scheduled.is_err() {
            tracing::warn!("network monitor: failed to schedule reachability on run loop");
            return;
        }

        CFRunLoop::run_current();
    }
}
