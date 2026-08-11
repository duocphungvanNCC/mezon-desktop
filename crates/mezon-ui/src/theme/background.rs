use gpui::{IntoElement, Styled, canvas, fill};
use mezon_theme::surface::ThemeSurface;

pub fn surface_background(surface: ThemeSurface) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            for index in 0..surface.layer_count() {
                if let Some(background) = surface.layer(index) {
                    window.paint_quad(fill(bounds, background));
                }
            }
        },
    )
    .absolute()
    .inset_0()
}
