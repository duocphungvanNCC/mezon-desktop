use std::sync::OnceLock;

unsafe extern "C" {
    #[link_name = "_ZN6webrtc18InitializePipeWireEv"]
    fn webrtc_initialize_pipewire() -> bool;
}

pub(crate) fn ensure_pipewire_stubs_armed() -> bool {
    static ARMED: OnceLock<bool> = OnceLock::new();

    *ARMED.get_or_init(|| {
        let armed = unsafe { webrtc_initialize_pipewire() };
        if !armed {
            tracing::error!("webrtc::InitializePipeWire() failed, libpipewire unavailable");
        }
        armed
    })
}
