use gpui::{App, Pixels, px};

use mezon_store::AppConfig;

const ATTACHMENT_MAX_W: f32 = 400.0;
const ATTACHMENT_MAX_H: f32 = 300.0;

pub fn proxied(cx: &App, source_url: &str, width: u32, height: u32, resize_type: &str) -> String {
    if source_url.is_empty() {
        return String::new();
    }
    AppConfig::try_global(cx)
        .map(|cfg| cfg.imgproxy_url(source_url, width, height, resize_type))
        .unwrap_or_else(|| source_url.to_string())
}

pub fn avatar_url(cx: &App, source_url: &str) -> String {
    proxied(cx, source_url, 100, 100, "fit")
}

pub fn profile_url(cx: &App, source_url: &str) -> String {
    proxied(cx, source_url, 300, 300, "fit")
}

pub fn attachment_display_dimensions(real_width: u32, real_height: u32) -> (f32, f32) {
    let w = real_width as f32;
    let h = real_height as f32;
    if w <= 0.0 || h <= 0.0 {
        return (280.0, 150.0);
    }
    let scale = (ATTACHMENT_MAX_W / w).min(ATTACHMENT_MAX_H / h).min(1.0);
    (w * scale, h * scale)
}

pub fn attachment_image(
    cx: &App,
    source_url: &str,
    real_width: u32,
    real_height: u32,
) -> (String, Pixels, Pixels) {
    if source_url.is_empty() {
        let (w, h) = attachment_display_dimensions(real_width, real_height);
        return (String::new(), px(w), px(h));
    }
    let (display_w, display_h) = attachment_display_dimensions(real_width, real_height);
    let proxy_w = (display_w * 2.0).ceil().max(1.0) as u32;
    let proxy_h = (display_h * 2.0).ceil().max(1.0) as u32;
    let resize = if real_width == 0 || real_height == 0 {
        "fill"
    } else if real_width < proxy_w || real_height < proxy_h {
        "fill-down"
    } else {
        "fill"
    };
    let src = proxied(cx, source_url, proxy_w, proxy_h, resize);
    (src, px(display_w), px(display_h))
}
