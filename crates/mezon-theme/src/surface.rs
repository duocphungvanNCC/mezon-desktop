use gpui::{Background, Fill, Hsla, Rgba, linear_color_stop, linear_gradient};

use crate::surfaces::surface_gradients;
use crate::tokens::ThemeTokens;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    pub color: Rgba,
    pub position: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceGradient {
    pub angle: f32,
    pub stops: &'static [GradientStop],
    pub overlay: Option<Rgba>,
    pub base: Option<Rgba>,
    pub viewport_anchored: bool,
}

impl SurfaceGradient {
    fn composite(&self, color: Rgba) -> Rgba {
        let mut out = color;
        if let Some(base) = self.base {
            out = source_over(out, base);
        }
        if let Some(overlay) = self.overlay {
            out = source_over(overlay, out);
        }
        out
    }
}

pub(crate) struct SurfaceGradients {
    pub(crate) primary: Option<SurfaceGradient>,
    pub(crate) secondary: Option<SurfaceGradient>,
    pub(crate) surface: Option<SurfaceGradient>,
    pub(crate) direct_message: Option<SurfaceGradient>,
    pub(crate) input_primary: Option<SurfaceGradient>,
    pub(crate) active_friend_list: Option<SurfaceGradient>,
    pub(crate) modal_search: Option<SurfaceGradient>,
    pub(crate) outside_footer: Option<SurfaceGradient>,
    pub(crate) footer: Option<SurfaceGradient>,
}

impl SurfaceGradients {
    pub(crate) const NONE: Self = Self {
        primary: None,
        secondary: None,
        surface: None,
        direct_message: None,
        input_primary: None,
        active_friend_list: None,
        modal_search: None,
        outside_footer: None,
        footer: None,
    };
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeSurface {
    pub solid: Rgba,
    pub gradient: Option<SurfaceGradient>,
}

impl ThemeSurface {
    pub const fn from_solid(color: Rgba) -> Self {
        Self {
            solid: color,
            gradient: None,
        }
    }

    pub fn viewport_anchored(&self) -> bool {
        self.gradient
            .is_some_and(|gradient| gradient.viewport_anchored)
    }

    pub fn layer_count(&self) -> usize {
        match self.gradient {
            Some(gradient) if gradient.stops.len() >= 2 => gradient.stops.len() - 1,
            _ => 1,
        }
    }

    pub fn layer(&self, index: usize) -> Option<Background> {
        let solid = || (index == 0).then(|| Background::from(Hsla::from(self.solid)));
        let Some(gradient) = self.gradient else {
            return solid();
        };
        let (Some(from), Some(to)) = (gradient.stops.get(index), gradient.stops.get(index + 1))
        else {
            return solid();
        };
        let to_color = gradient.composite(to.color);
        let from_color = if index == 0 {
            gradient.composite(from.color)
        } else {
            fade_out(to_color)
        };
        Some(linear_gradient(
            gradient.angle,
            linear_color_stop(from_color, from.position),
            linear_color_stop(to_color, to.position),
        ))
    }

    pub fn fill(&self) -> Background {
        let solid = Background::from(Hsla::from(self.solid));
        let Some(gradient) = self.gradient else {
            return solid;
        };
        if gradient.viewport_anchored {
            return solid;
        }
        let (Some(first), Some(last)) = (gradient.stops.first(), gradient.stops.last()) else {
            return solid;
        };
        if gradient.stops.len() < 2 {
            return Background::from(Hsla::from(gradient.composite(first.color)));
        }
        linear_gradient(
            gradient.angle,
            linear_color_stop(gradient.composite(first.color), first.position),
            linear_color_stop(gradient.composite(last.color), last.position),
        )
    }
}

impl From<ThemeSurface> for Background {
    fn from(surface: ThemeSurface) -> Self {
        surface.fill()
    }
}

impl From<ThemeSurface> for Fill {
    fn from(surface: ThemeSurface) -> Self {
        Self::Color(surface.fill())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeSurfaces {
    pub primary: ThemeSurface,
    pub secondary: ThemeSurface,
    pub surface: ThemeSurface,
    pub direct_message: ThemeSurface,
    pub input_primary: ThemeSurface,
    pub active_friend_list: ThemeSurface,
    pub modal_search: ThemeSurface,
    pub outside_footer: ThemeSurface,
    pub footer: ThemeSurface,
}

impl ThemeSurfaces {
    pub fn for_theme(theme: &str, tokens: &ThemeTokens) -> Self {
        let gradients = surface_gradients(theme);
        Self {
            primary: ThemeSurface {
                solid: tokens.bg_primary,
                gradient: gradients.primary,
            },
            secondary: ThemeSurface {
                solid: tokens.bg_secondary,
                gradient: gradients.secondary,
            },
            surface: ThemeSurface {
                solid: tokens.bg_surface,
                gradient: gradients.surface,
            },
            direct_message: ThemeSurface {
                solid: tokens.bg_theme_direct_message,
                gradient: gradients.direct_message,
            },
            input_primary: ThemeSurface {
                solid: tokens.bg_theme_input_primary,
                gradient: gradients.input_primary,
            },
            active_friend_list: ThemeSurface {
                solid: tokens.bg_active_friend_list,
                gradient: gradients.active_friend_list,
            },
            modal_search: ThemeSurface {
                solid: tokens.bg_modal_theme_search,
                gradient: gradients.modal_search,
            },
            outside_footer: ThemeSurface {
                solid: tokens.bg_outside_footer,
                gradient: gradients.outside_footer,
            },
            footer: ThemeSurface {
                solid: tokens.bg_footer,
                gradient: gradients.footer,
            },
        }
    }
}

fn fade_out(color: Rgba) -> Rgba {
    Rgba { a: 0.0, ..color }
}

fn source_over(top: Rgba, bottom: Rgba) -> Rgba {
    let alpha = top.a + bottom.a * (1.0 - top.a);
    if alpha <= f32::EPSILON {
        return Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        };
    }
    let blend = |top_channel: f32, bottom_channel: f32| {
        (top_channel * top.a + bottom_channel * bottom.a * (1.0 - top.a)) / alpha
    };
    Rgba {
        r: blend(top.r, bottom.r),
        g: blend(top.g, bottom.g),
        b: blend(top.b, bottom.b),
        a: alpha,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Rgba {
        Rgba { r, g, b, a }
    }

    const fn stop(color: Rgba, position: f32) -> GradientStop {
        GradientStop { color, position }
    }

    const RED_TO_BLUE: &[GradientStop] = &[
        stop(rgba(1.0, 0.0, 0.0, 1.0), 0.0),
        stop(rgba(0.0, 0.0, 1.0, 1.0), 1.0),
    ];

    const TRANSLUCENT_WHITE: &[GradientStop] = &[stop(rgba(1.0, 1.0, 1.0, 0.5), 0.0)];

    const THREE_STOPS: &[GradientStop] = &[
        stop(rgba(1.0, 0.0, 0.0, 1.0), 0.0),
        stop(rgba(0.0, 1.0, 0.0, 1.0), 0.5),
        stop(rgba(0.0, 0.0, 1.0, 1.0), 1.0),
    ];

    fn close(actual: f32, expected: f32) -> bool {
        (actual - expected).abs() < 0.0005
    }

    #[test]
    fn overlay_composites_over_gradient_stop() {
        let gradient = SurfaceGradient {
            angle: 0.0,
            stops: RED_TO_BLUE,
            overlay: Some(rgba(0.0, 0.0, 0.0, 0.5)),
            base: None,
            viewport_anchored: false,
        };
        let composited = gradient.composite(RED_TO_BLUE[0].color);
        assert!(close(composited.r, 0.5), "{composited:?}");
        assert!(close(composited.a, 1.0), "{composited:?}");
    }

    #[test]
    fn translucent_stop_composites_over_base() {
        let gradient = SurfaceGradient {
            angle: 0.0,
            stops: TRANSLUCENT_WHITE,
            overlay: None,
            base: Some(rgba(0.0, 0.0, 0.0, 1.0)),
            viewport_anchored: false,
        };
        let composited = gradient.composite(TRANSLUCENT_WHITE[0].color);
        assert!(close(composited.r, 0.5), "{composited:?}");
        assert!(close(composited.a, 1.0), "{composited:?}");
    }

    #[test]
    fn layer_count_matches_stop_segments() {
        let surface = ThemeSurface {
            solid: rgba(0.0, 0.0, 0.0, 1.0),
            gradient: Some(SurfaceGradient {
                angle: 90.0,
                stops: THREE_STOPS,
                overlay: None,
                base: None,
                viewport_anchored: false,
            }),
        };
        assert_eq!(surface.layer_count(), 2);
        assert!(surface.layer(0).is_some());
        assert!(surface.layer(1).is_some());
        assert!(surface.layer(2).is_none());
    }

    #[test]
    fn ramp_layers_fade_in_from_the_previous_stop() {
        let surface = ThemeSurface {
            solid: rgba(0.0, 0.0, 0.0, 1.0),
            gradient: Some(SurfaceGradient {
                angle: 90.0,
                stops: THREE_STOPS,
                overlay: None,
                base: None,
                viewport_anchored: false,
            }),
        };
        let base = surface.layer(0).expect("first segment");
        let ramp = surface.layer(1).expect("second segment");
        assert!(base.as_solid().is_none());
        assert!(ramp.as_solid().is_none());
        assert_ne!(base, ramp);
    }

    #[test]
    fn solid_surface_has_one_layer() {
        let surface = ThemeSurface::from_solid(rgba(0.1, 0.2, 0.3, 1.0));
        assert_eq!(surface.layer_count(), 1);
        assert!(surface.layer(0).is_some());
        assert!(surface.layer(1).is_none());
        assert_eq!(surface.fill().as_solid(), Some(Hsla::from(surface.solid)));
    }

    #[test]
    fn viewport_anchored_surface_falls_back_to_solid_fill() {
        let surface = ThemeSurface {
            solid: rgba(0.1, 0.2, 0.3, 1.0),
            gradient: Some(SurfaceGradient {
                angle: 154.19,
                stops: RED_TO_BLUE,
                overlay: None,
                base: None,
                viewport_anchored: true,
            }),
        };
        assert_eq!(surface.fill().as_solid(), Some(Hsla::from(surface.solid)));
        assert!(surface.layer(0).is_some());
    }

    #[test]
    fn react_themes_expose_gradient_surfaces() {
        for theme in [
            "sunrise",
            "purple_haze",
            "abyss_dark",
            "berrynade",
            "cisher",
            "sunset",
        ] {
            let tokens = ThemeTokens::for_theme(theme);
            let surfaces = ThemeSurfaces::for_theme(theme, &tokens);
            assert!(
                surfaces.primary.gradient.is_some(),
                "{theme} should carry a primary gradient"
            );
            assert!(
                surfaces.secondary.gradient.is_some(),
                "{theme} should carry a secondary gradient"
            );
        }
    }

    #[test]
    fn composited_stops_average_to_the_flattened_token() {
        let mut worst = 0.0f32;
        let mut worst_label = String::new();
        for theme in [
            "sunrise",
            "purple_haze",
            "abyss_dark",
            "berrynade",
            "cisher",
            "sunset",
        ] {
            let tokens = ThemeTokens::for_theme(theme);
            let surfaces = ThemeSurfaces::for_theme(theme, &tokens);
            for (name, surface) in [
                ("primary", surfaces.primary),
                ("secondary", surfaces.secondary),
                ("surface", surfaces.surface),
                ("direct_message", surfaces.direct_message),
                ("input_primary", surfaces.input_primary),
                ("active_friend_list", surfaces.active_friend_list),
                ("modal_search", surfaces.modal_search),
                ("outside_footer", surfaces.outside_footer),
                ("footer", surfaces.footer),
            ] {
                let Some(gradient) = surface.gradient else {
                    continue;
                };
                let count = gradient.stops.len() as f32;
                let mut sum = [0.0f32; 3];
                for stop in gradient.stops {
                    let composited = gradient.composite(stop.color);
                    sum[0] += composited.r;
                    sum[1] += composited.g;
                    sum[2] += composited.b;
                }
                let expected = surface.solid;
                for (channel, total) in [expected.r, expected.g, expected.b].iter().zip(sum) {
                    let delta = (total / count - channel).abs();
                    if delta > worst {
                        worst = delta;
                        worst_label = format!("{theme}.{name}");
                    }
                }
            }
        }
        assert!(worst < 0.02, "{worst_label} drifts by {worst}");
    }

    #[test]
    fn flat_themes_stay_solid() {
        for theme in ["dark", "light"] {
            let tokens = ThemeTokens::for_theme(theme);
            let surfaces = ThemeSurfaces::for_theme(theme, &tokens);
            assert!(surfaces.primary.gradient.is_none());
            assert_eq!(
                surfaces.primary.fill().as_solid(),
                Some(Hsla::from(tokens.bg_primary))
            );
        }
    }
}
