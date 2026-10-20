use std::sync::{Arc, Mutex};

use gpui::{
    AnyElement, App, Bounds, CursorStyle, Entity, FontWeight, Hsla, IntoElement, KeyDownEvent,
    MouseButton, PathBuilder, Pixels, SharedString, TextAlign, Window, canvas, div, fill, point,
    prelude::*, px, size,
};
use mezon_store::{
    ContentViewport, DrawAnnotation, DrawTool, PRESET_COLORS, PRESET_WIDTHS, STROKE_WIDTH_MEDIUM,
    STROKE_WIDTH_THIN, TextAnnotation, VoiceStore,
};
use mezon_theme::Theme;
use ui::Tooltip;

use crate::components::primitives::{Icon, IconName, h_flex};

const ACCENT_BLUE: u32 = 0x5865f2;

pub(crate) fn render_draw_toolbox(
    voice: &Entity<VoiceStore>,
    _theme: &Theme,
    cx: &App,
) -> AnyElement {
    let store = voice.read(cx);
    let draw_state = store.screen_draw();
    let active_tool = draw_state.active_tool;
    let stroke_color = draw_state.stroke_color;
    let stroke_width = draw_state.stroke_width;
    let can_undo = draw_state.can_undo();
    let can_redo = draw_state.can_redo();

    let pen_btn = {
        let voice = voice.clone();
        let is_selected = active_tool == DrawTool::Pen;
        tool_button(
            "draw-tool-pen",
            IconName::PenEdit,
            "Pen",
            is_selected,
            move |_, _, cx| {
                voice.update(cx, |store, _| {
                    store.screen_draw_mut().set_tool(DrawTool::Pen);
                });
            },
        )
    };

    let text_btn = {
        let voice = voice.clone();
        let is_selected = active_tool == DrawTool::Text;
        tool_button(
            "draw-tool-text",
            IconName::TextTool,
            "Text",
            is_selected,
            move |_, _, cx| {
                voice.update(cx, |store, _| {
                    store.screen_draw_mut().set_tool(DrawTool::Text);
                });
            },
        )
    };

    let eraser_btn = {
        let voice = voice.clone();
        let is_selected = active_tool == DrawTool::Eraser;
        tool_button(
            "draw-tool-eraser",
            IconName::Eraser,
            "Eraser",
            is_selected,
            move |_, _, cx| {
                voice.update(cx, |store, _| {
                    store.screen_draw_mut().set_tool(DrawTool::Eraser);
                });
            },
        )
    };

    let colors_row = {
        let voice = voice.clone();
        h_flex()
            .items_center()
            .gap_1()
            .children(PRESET_COLORS.iter().map(|&color| {
                let voice = voice.clone();
                let is_active = stroke_color == color;
                div()
                    .id(SharedString::from(format!("draw-color-{:x}", color)))
                    .size(px(18.))
                    .rounded_full()
                    .bg(gpui::rgba(color))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(is_active, |s| {
                        s.border_2()
                            .border_color(gpui::rgb(0xffffff))
                            .shadow_sm()
                    })
                    .hover(|s| s.opacity(0.85))
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        voice.update(cx, |store, _| {
                            store.screen_draw_mut().set_color(color);
                        });
                    })
            }))
    };

    let width_row = (active_tool == DrawTool::Pen).then(|| {
        let voice = voice.clone();
        h_flex()
            .items_center()
            .gap_1()
            .children(PRESET_WIDTHS.iter().map(|&w| {
                let voice = voice.clone();
                let is_active = (stroke_width - w).abs() < 0.5;
                let dot_size = if (w - STROKE_WIDTH_THIN).abs() < 0.5 {
                    px(4.)
                } else if (w - STROKE_WIDTH_MEDIUM).abs() < 0.5 {
                    px(7.)
                } else {
                    px(10.)
                };
                div()
                    .id(SharedString::from(format!("draw-width-{:.0}", w)))
                    .size(px(24.))
                    .rounded_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .bg(if is_active {
                        gpui::rgba(0xffffff30).into()
                    } else {
                        gpui::transparent_black()
                    })
                    .hover(|s| s.bg(gpui::rgba(0xffffff20)))
                    .child(
                        div()
                            .size(dot_size)
                            .rounded_full()
                            .bg(gpui::rgba(stroke_color)),
                    )
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        voice.update(cx, |store, _| {
                            store.screen_draw_mut().set_width(w);
                        });
                    })
            }))
    });

    let undo_btn = {
        let voice = voice.clone();
        action_button(
            "draw-action-undo",
            IconName::RotateLeftIcon,
            "Undo",
            can_undo,
            move |_, _, cx| {
                voice.update(cx, |store, _| {
                    store.screen_draw_mut().undo();
                });
            },
        )
    };

    let redo_btn = {
        let voice = voice.clone();
        action_button(
            "draw-action-redo",
            IconName::RotateRightIcon,
            "Redo",
            can_redo,
            move |_, _, cx| {
                voice.update(cx, |store, _| {
                    store.screen_draw_mut().redo();
                });
            },
        )
    };

    let clear_btn = {
        let voice = voice.clone();
        action_button(
            "draw-action-clear",
            IconName::TrashIcon,
            "Clear All",
            true,
            move |_, _, cx| {
                voice.update(cx, |store, _| {
                    store.screen_draw_mut().clear();
                });
            },
        )
    };

    let close_btn = {
        let voice = voice.clone();
        action_button(
            "draw-action-close",
            IconName::Close,
            "Close Drawing",
            true,
            move |_, _, cx| {
                voice.update(cx, |store, cx| {
                    store.set_screen_draw_active(false, cx);
                });
            },
        )
    };

    div()
        .id("screen-draw-toolbox-container")
        .absolute()
        .top(px(16.))
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(
            h_flex()
                .id("screen-draw-toolbox")
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .rounded_xl()
                .bg(gpui::rgba(0x18181be6))
                .border_1()
                .border_color(gpui::rgba(0xffffff26))
                .shadow_xl()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(pen_btn)
                .child(text_btn)
                .child(eraser_btn)
                .child(vertical_divider())
                .child(colors_row)
                .children(width_row.map(|row| h_flex().items_center().gap_2().child(vertical_divider()).child(row)))
                .child(vertical_divider())
                .child(undo_btn)
                .child(redo_btn)
                .child(clear_btn)
                .child(vertical_divider())
                .child(close_btn),
        )
        .into_any_element()
}

fn vertical_divider() -> AnyElement {
    div()
        .w(px(1.))
        .h(px(18.))
        .bg(gpui::rgba(0xffffff26))
        .into_any_element()
}

fn tool_button(
    id: &'static str,
    icon: IconName,
    tooltip_text: &'static str,
    selected: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .size(px(32.))
        .rounded_lg()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .bg(if selected {
            gpui::rgba(0x3b82f640).into()
        } else {
            gpui::transparent_black()
        })
        .when(selected, |s| {
            s.border_1().border_color(gpui::rgb(ACCENT_BLUE))
        })
        .hover(|s| s.bg(gpui::rgba(0xffffff1a)))
        .tooltip(Tooltip::text(tooltip_text))
        .child(
            Icon::new(icon)
                .size(px(18.))
                .text_color(if selected {
                    gpui::rgb(0xffffff)
                } else {
                    gpui::rgba(0xffffffb3)
                }),
        )
        .on_click(move |e, window, cx| {
            cx.stop_propagation();
            on_click(e, window, cx);
        })
        .into_any_element()
}

fn action_button(
    id: &'static str,
    icon: IconName,
    tooltip_text: &'static str,
    enabled: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .size(px(32.))
        .rounded_lg()
        .flex()
        .items_center()
        .justify_center()
        .cursor(if enabled {
            CursorStyle::PointingHand
        } else {
            CursorStyle::Arrow
        })
        .opacity(if enabled { 1.0 } else { 0.35 })
        .bg(gpui::transparent_black())
        .when(enabled, |s| s.hover(|h| h.bg(gpui::rgba(0xffffff1a))))
        .tooltip(Tooltip::text(tooltip_text))
        .child(
            Icon::new(icon)
                .size(px(18.))
                .text_color(gpui::rgba(0xffffffb3)),
        )
        .on_click(move |e, window, cx| {
            cx.stop_propagation();
            if enabled {
                on_click(e, window, cx);
            }
        })
        .into_any_element()
}

pub(crate) fn render_screen_draw_overlay(
    voice: &Entity<VoiceStore>,
    _theme: &Theme,
    frame_size: Option<(u32, u32)>,
    cx: &App,
) -> AnyElement {
    let store = voice.read(cx);
    let draw_state = store.screen_draw().clone();
    let is_active = draw_state.is_active;

    let cursor = match draw_state.active_tool {
        DrawTool::Pen => CursorStyle::Crosshair,
        DrawTool::Text => CursorStyle::IBeam,
        DrawTool::Eraser => CursorStyle::Crosshair,
    };

    let bounds_ref = Arc::new(Mutex::new(None::<Bounds<Pixels>>));
    let bounds_for_canvas = bounds_ref.clone();
    let bounds_for_down = bounds_ref.clone();
    let bounds_for_move = bounds_ref.clone();
    let bounds_for_up = bounds_ref;

    let (content_w, content_h) = frame_size.unwrap_or((1920, 1080));
    let vp_w = content_w as f32;
    let vp_h = content_h as f32;

    let draw_state_for_canvas = draw_state.clone();
    let draw_canvas = canvas(
        move |bounds, _, _| {
            if let Ok(mut g) = bounds_for_canvas.lock() {
                *g = Some(bounds);
            }
            bounds
        },
        move |bounds, _, window, cx| {
            let vp = ContentViewport::compute(
                f32::from(bounds.size.width),
                f32::from(bounds.size.height),
                vp_w,
                vp_h,
            );

            // Paint committed strokes
            for ann in &draw_state_for_canvas.annotations {
                match ann {
                    DrawAnnotation::Stroke(s) => {
                        paint_stroke(window, &vp, bounds, s);
                    }
                    DrawAnnotation::Text(t) => {
                        if draw_state_for_canvas.editing_text_id == Some(t.id) {
                            continue;
                        }
                        paint_text_annotation(window, cx, &vp, bounds, t);
                    }
                }
            }

            // Paint in-progress pen stroke
            if let Some(s) = &draw_state_for_canvas.current_stroke {
                paint_stroke(window, &vp, bounds, s);
            }
        },
    )
    .absolute()
    .size_full();

    let mut overlay = div()
        .id("screen-draw-overlay")
        .absolute()
        .inset_0()
        .child(draw_canvas);

    if is_active {
        let voice_down = voice.clone();
        let voice_move = voice.clone();
        let voice_up = voice.clone();
        let voice_key = voice.clone();

        overlay = overlay
            .cursor(cursor)
            .on_mouse_down(MouseButton::Left, move |e, _, cx| {
                cx.stop_propagation();
                let Some(bounds) = bounds_for_down.lock().ok().and_then(|g| *g) else {
                    return;
                };
                let rel_x = f32::from(e.position.x - bounds.origin.x);
                let rel_y = f32::from(e.position.y - bounds.origin.y);
                let vp = ContentViewport::compute(
                    f32::from(bounds.size.width),
                    f32::from(bounds.size.height),
                    vp_w,
                    vp_h,
                );
                let norm = vp.to_normalized(rel_x, rel_y);

                voice_down.update(cx, |store, cx| {
                    let tool = store.screen_draw().active_tool;
                    match tool {
                        DrawTool::Pen => {
                            if store.screen_draw().editing_text_id.is_some() {
                                store.screen_draw_mut().commit_editing_text();
                            }
                            store.screen_draw_mut().start_stroke(norm.x, norm.y);
                        }
                        DrawTool::Eraser => {
                            if store.screen_draw().editing_text_id.is_some() {
                                store.screen_draw_mut().commit_editing_text();
                            }
                            store.screen_draw_mut().erase_at(norm.x, norm.y, 0.02);
                        }
                        DrawTool::Text => {
                            // If user clicked near an existing text annotation, edit it
                            let clicked_existing = store.screen_draw().annotations.iter().find_map(|ann| match ann {
                                DrawAnnotation::Text(t) => {
                                    if t.position.distance(&norm) < 0.04 {
                                        Some(t.id)
                                    } else {
                                        None
                                    }
                                }
                                _ => None,
                            });

                            if let Some(existing_id) = clicked_existing {
                                store.screen_draw_mut().start_editing_text(existing_id);
                            } else {
                                if store.screen_draw().editing_text_id.is_some() {
                                    store.screen_draw_mut().commit_editing_text();
                                }
                                let id = store.screen_draw_mut().add_text(norm.x, norm.y, String::new());
                                store.screen_draw_mut().start_editing_text(id);
                            }
                        }
                    }
                    cx.notify();
                });
            })
            .on_mouse_move(move |e, _, cx| {
                let Some(bounds) = bounds_for_move.lock().ok().and_then(|g| *g) else {
                    return;
                };
                let rel_x = f32::from(e.position.x - bounds.origin.x);
                let rel_y = f32::from(e.position.y - bounds.origin.y);
                let vp = ContentViewport::compute(
                    f32::from(bounds.size.width),
                    f32::from(bounds.size.height),
                    vp_w,
                    vp_h,
                );
                let norm = vp.to_normalized(rel_x, rel_y);

                voice_move.update(cx, |store, cx| {
                    let tool = store.screen_draw().active_tool;
                    if tool == DrawTool::Pen && store.screen_draw().current_stroke.is_some() {
                        store.screen_draw_mut().add_stroke_point(norm.x, norm.y);
                        cx.notify();
                    } else if tool == DrawTool::Eraser && e.pressed_button == Some(MouseButton::Left) {
                        if store.screen_draw_mut().erase_at(norm.x, norm.y, 0.02) {
                            cx.notify();
                        }
                    }
                });
            })
            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                let Some(_) = bounds_for_up.lock().ok().and_then(|g| *g) else {
                    return;
                };
                voice_up.update(cx, |store, cx| {
                    if store.screen_draw().active_tool == DrawTool::Pen {
                        store.screen_draw_mut().finish_stroke();
                        cx.notify();
                    }
                });
            })
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                voice_key.update(cx, |store, cx| {
                    if let Some(id) = store.screen_draw().editing_text_id {
                        let key = &event.keystroke.key;
                        if key == "enter" {
                            store.screen_draw_mut().commit_editing_text();
                            cx.notify();
                        } else if key == "escape" {
                            store.screen_draw_mut().cancel_editing_text();
                            cx.notify();
                        } else if key == "backspace" {
                            store.screen_draw_mut().editing_text_content.pop();
                            let content = store.screen_draw().editing_text_content.clone();
                            store.screen_draw_mut().update_text(id, content);
                            cx.notify();
                        } else if !event.keystroke.modifiers.control
                            && !event.keystroke.modifiers.alt
                            && !event.keystroke.modifiers.platform
                        {
                            if key == "space" {
                                store.screen_draw_mut().editing_text_content.push(' ');
                                let content = store.screen_draw().editing_text_content.clone();
                                store.screen_draw_mut().update_text(id, content);
                                cx.notify();
                            } else {
                                let ch = event.keystroke.key_char.as_deref().unwrap_or(key.as_str());
                                if ch.chars().count() == 1 {
                                    store.screen_draw_mut().editing_text_content.push_str(ch);
                                    let content = store.screen_draw().editing_text_content.clone();
                                    store.screen_draw_mut().update_text(id, content);
                                    cx.notify();
                                }
                            }
                        }
                    }
                });
            });

        // Inline text editor card if editing text
        if let Some(editing_id) = draw_state.editing_text_id {
            if let Some(ann) = draw_state.annotations.iter().find_map(|a| match a {
                DrawAnnotation::Text(t) if t.id == editing_id => Some(t),
                _ => None,
            }) {
                overlay = overlay.child(render_inline_text_editor(
                    voice,
                    ann,
                    &draw_state.editing_text_content,
                    draw_state.stroke_color,
                ));
            }
        }
    }

    overlay.into_any_element()
}

fn render_inline_text_editor(
    voice: &Entity<VoiceStore>,
    ann: &TextAnnotation,
    content: &str,
    color: u32,
) -> AnyElement {
    let voice_commit = voice.clone();
    let voice_cancel = voice.clone();
    let content_text = SharedString::from(content.to_string());

    // Map normalized (u, v) percentage to CSS position percentages
    let left_pct = ann.position.x * 100.0;
    let top_pct = ann.position.y * 100.0;

    div()
        .id("screen-draw-text-editor-card")
        .absolute()
        .left(gpui::relative(left_pct / 100.0))
        .top(gpui::relative(top_pct / 100.0))
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .rounded_lg()
        .bg(gpui::rgba(0x18181bf5))
        .border_2()
        .border_color(gpui::rgba(color))
        .shadow_xl()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .min_w(px(160.))
                .max_w(px(320.))
                .flex()
                .items_center()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .text_color(if content.is_empty() {
                    Hsla::from(gpui::rgba(0xffffff66))
                } else {
                    Hsla::from(gpui::rgba(color))
                })
                .child(if content.is_empty() {
                    SharedString::from("Type note... (Enter to save)")
                } else {
                    content_text
                }),
        )
        .child(
            div()
                .id("screen-draw-text-commit")
                .size(px(24.))
                .rounded_md()
                .bg(gpui::rgb(0x10b981))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.opacity(0.85))
                .tooltip(Tooltip::text("Commit (Enter)"))
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .text_color(gpui::rgb(0xffffff))
                        .child("✓"),
                )
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    voice_commit.update(cx, |store, cx| {
                        store.screen_draw_mut().commit_editing_text();
                        cx.notify();
                    });
                }),
        )
        .child(
            div()
                .id("screen-draw-text-cancel")
                .size(px(24.))
                .rounded_md()
                .bg(gpui::rgb(0xef4444))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.opacity(0.85))
                .tooltip(Tooltip::text("Cancel (Esc)"))
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .text_color(gpui::rgb(0xffffff))
                        .child("✕"),
                )
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    voice_cancel.update(cx, |store, cx| {
                        store.screen_draw_mut().cancel_editing_text();
                        cx.notify();
                    });
                }),
        )
        .into_any_element()
}

fn paint_stroke(
    window: &mut Window,
    vp: &ContentViewport,
    bounds: Bounds<Pixels>,
    stroke: &mezon_store::PenStroke,
) {
    if stroke.points.is_empty() {
        return;
    }

    let color = gpui::rgba(stroke.color);
    let radius = px((stroke.width * 0.5).max(1.5));

    if stroke.points.len() == 1 {
        let (x, y) = vp.to_pixels(&stroke.points[0]);
        let origin = point(
            bounds.origin.x + px(x) - radius,
            bounds.origin.y + px(y) - radius,
        );
        let quad_size = size(radius * 2.0, radius * 2.0);
        window.paint_quad(fill(Bounds { origin, size: quad_size }, color).corner_radii(radius));
        return;
    }

    let mut builder = PathBuilder::stroke(px(stroke.width));
    let (first_x, first_y) = vp.to_pixels(&stroke.points[0]);
    builder.move_to(point(
        bounds.origin.x + px(first_x),
        bounds.origin.y + px(first_y),
    ));

    for pt in &stroke.points[1..] {
        let (x, y) = vp.to_pixels(pt);
        builder.line_to(point(bounds.origin.x + px(x), bounds.origin.y + px(y)));
    }

    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }

    // Paint rounded stroke caps at start and end
    let (x0, y0) = vp.to_pixels(&stroke.points[0]);
    window.paint_quad(
        fill(
            Bounds {
                origin: point(
                    bounds.origin.x + px(x0) - radius,
                    bounds.origin.y + px(y0) - radius,
                ),
                size: size(radius * 2.0, radius * 2.0),
            },
            color,
        )
        .corner_radii(radius),
    );

    if let Some(last) = stroke.points.last() {
        let (xn, yn) = vp.to_pixels(last);
        window.paint_quad(
            fill(
                Bounds {
                    origin: point(
                        bounds.origin.x + px(xn) - radius,
                        bounds.origin.y + px(yn) - radius,
                    ),
                    size: size(radius * 2.0, radius * 2.0),
                },
                color,
            )
            .corner_radii(radius),
        );
    }
}

fn paint_text_annotation(
    window: &mut Window,
    cx: &mut App,
    vp: &ContentViewport,
    bounds: Bounds<Pixels>,
    text: &TextAnnotation,
) {
    if text.content.trim().is_empty() {
        return;
    }

    let (px_x, px_y) = vp.to_pixels(&text.position);
    let abs_x = bounds.origin.x + px(px_x);
    let abs_y = bounds.origin.y + px(px_y);

    let font_size = px(text.font_size.clamp(12.0, 36.0));
    let color: Hsla = gpui::rgba(text.color).into();

    let mut run = window.text_style().to_run(text.content.len());
    run.color = color;
    run.font.weight = FontWeight::BOLD;

    let line = window.text_system().shape_line(
        SharedString::from(text.content.clone()),
        font_size,
        std::slice::from_ref(&run),
        None,
    );

    let pad_x = px(8.0);
    let pad_y = px(4.0);
    let text_w = line.width();
    let line_height = font_size * 1.3;

    // Dark translucent background pill for high contrast over any screen content
    let bg_bounds = Bounds {
        origin: point(abs_x - pad_x * 0.5, abs_y - pad_y * 0.5),
        size: size(text_w + pad_x, line_height + pad_y),
    };
    window.paint_quad(fill(bg_bounds, gpui::rgba(0x000000b3)).corner_radii(px(4.0)));

    let _ = line.paint(
        point(abs_x, abs_y),
        line_height,
        TextAlign::Left,
        None,
        window,
        cx,
    );
}
