use crate::screen_picker::PickedScreen;

#[derive(Clone, Debug)]
pub struct ScreenSharePreview {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
const PREVIEW_MAX_WIDTH: u32 = 420;
#[cfg(any(target_os = "macos", target_os = "linux"))]
const PREVIEW_MAX_HEIGHT: u32 = 236;

pub fn capture_screen_share_preview(pick: &PickedScreen) -> Option<ScreenSharePreview> {
    match pick {
        PickedScreen::Target(target) => {
            #[cfg(target_os = "macos")]
            {
                capture_macos_preview(target)
            }

            #[cfg(target_os = "linux")]
            {
                capture_x11_preview(target)
            }

            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            {
                let _ = target;
                None
            }
        }
        #[cfg(target_os = "linux")]
        PickedScreen::LinuxPortal => None,
    }
}

#[cfg(target_os = "macos")]
fn capture_macos_preview(target: &scap::Target) -> Option<ScreenSharePreview> {
    use core_graphics_helmer_fork::geometry::{CGPoint, CGRect, CGSize};
    use core_graphics_helmer_fork::window::{
        self, kCGWindowImageBoundsIgnoreFraming, kCGWindowImageNominalResolution,
        kCGWindowListOptionIncludingWindow,
    };

    let image = match target {
        scap::Target::Window(win) => {
            let bounds = CGRect::new(&CGPoint::new(0.0, 0.0), &CGSize::new(0.0, 0.0));
            window::create_image(
                bounds,
                kCGWindowListOptionIncludingWindow,
                win.raw_handle,
                kCGWindowImageBoundsIgnoreFraming | kCGWindowImageNominalResolution,
            )?
        }
        scap::Target::Display(display) => display.raw_handle.image()?,
    };

    cg_image_to_preview(&image)
}

#[cfg(target_os = "macos")]
fn cg_image_to_preview(
    image: &core_graphics_helmer_fork::image::CGImageRef,
) -> Option<ScreenSharePreview> {
    let width = image.width() as u32;
    let height = image.height() as u32;
    if width == 0 || height == 0 {
        return None;
    }

    let (thumb_w, thumb_h) = preview_dimensions(width, height);
    let bytes_per_row = image.bytes_per_row();
    if bytes_per_row < (width as usize * 4) {
        return None;
    }

    let data = image.data();
    let bytes = data.bytes();
    let mut rgba = vec![0u8; (thumb_w * thumb_h * 4) as usize];

    for y in 0..thumb_h as usize {
        let src_y = y * height as usize / thumb_h as usize;
        for x in 0..thumb_w as usize {
            let src_x = x * width as usize / thumb_w as usize;
            let src_offset = src_y * bytes_per_row + src_x * 4;
            let dst_offset = (y * thumb_w as usize + x) * 4;
            if src_offset + 3 >= bytes.len() {
                continue;
            }
            rgba[dst_offset] = bytes[src_offset];
            rgba[dst_offset + 1] = bytes[src_offset + 1];
            rgba[dst_offset + 2] = bytes[src_offset + 2];
            rgba[dst_offset + 3] = bytes[src_offset + 3];
        }
    }

    Some(ScreenSharePreview {
        width: thumb_w,
        height: thumb_h,
        rgba,
    })
}

#[cfg(target_os = "linux")]
fn capture_x11_preview(target: &scap::Target) -> Option<ScreenSharePreview> {
    use xcb::x;

    let (conn, _) = xcb::Connection::connect(None).ok()?;

    let (drawable, src_x, src_y, width, height) = match target {
        scap::Target::Window(win) => {
            let drawable = x::Drawable::Window(win.raw_handle);
            let cookie = conn.send_request(&x::GetGeometry { drawable });
            let geometry = conn.wait_for_reply(cookie).ok()?;
            (drawable, 0, 0, geometry.width(), geometry.height())
        }
        scap::Target::Display(display) => (
            x::Drawable::Window(display.raw_handle),
            display.x_offset,
            display.y_offset,
            display.width,
            display.height,
        ),
    };
    if width == 0 || height == 0 {
        return None;
    }

    let cookie = conn.send_request(&x::GetImage {
        format: x::ImageFormat::ZPixmap,
        drawable,
        x: src_x,
        y: src_y,
        width,
        height,
        plane_mask: u32::MAX,
    });
    let image = conn.wait_for_reply(cookie).ok()?;
    if image.depth() != 24 && image.depth() != 32 {
        return None;
    }

    let width = u32::from(width);
    let height = u32::from(height);
    let data = image.data();
    let stride = data.len() / height as usize;
    if stride < width as usize * 4 {
        return None;
    }

    let lsb_first = matches!(conn.get_setup().image_byte_order(), x::ImageOrder::LsbFirst);
    let (thumb_w, thumb_h) = preview_dimensions(width, height);
    let mut rgba = vec![0u8; (thumb_w * thumb_h * 4) as usize];

    for y in 0..thumb_h as usize {
        let src_y = y * height as usize / thumb_h as usize;
        for x in 0..thumb_w as usize {
            let src_x = x * width as usize / thumb_w as usize;
            let src_offset = src_y * stride + src_x * 4;
            let dst_offset = (y * thumb_w as usize + x) * 4;
            if src_offset + 3 >= data.len() {
                continue;
            }
            let (b, g, r) = if lsb_first {
                (data[src_offset], data[src_offset + 1], data[src_offset + 2])
            } else {
                (
                    data[src_offset + 3],
                    data[src_offset + 2],
                    data[src_offset + 1],
                )
            };
            rgba[dst_offset] = b;
            rgba[dst_offset + 1] = g;
            rgba[dst_offset + 2] = r;
            rgba[dst_offset + 3] = 0xff;
        }
    }

    Some(ScreenSharePreview {
        width: thumb_w,
        height: thumb_h,
        rgba,
    })
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn preview_dimensions(width: u32, height: u32) -> (u32, u32) {
    let width = width.max(1);
    let height = height.max(1);
    let scale = (PREVIEW_MAX_WIDTH as f32 / width as f32)
        .min(PREVIEW_MAX_HEIGHT as f32 / height as f32)
        .min(1.0);
    let thumb_w = ((width as f32 * scale).round() as u32).max(1);
    let thumb_h = ((height as f32 * scale).round() as u32).max(1);
    (thumb_w, thumb_h)
}
