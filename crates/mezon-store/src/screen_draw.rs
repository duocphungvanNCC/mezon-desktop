use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DrawTool {
    Pen,
    Text,
    Eraser,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NormalizedPoint {
    pub x: f32,
    pub y: f32,
}

impl NormalizedPoint {
    pub fn new(x: f32, y: f32) -> Self {
        Self {
            x: x.clamp(0.0, 1.0),
            y: y.clamp(0.0, 1.0),
        }
    }

    pub fn distance_squared(&self, other: &Self) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        dx * dx + dy * dy
    }

    pub fn distance(&self, other: &Self) -> f32 {
        self.distance_squared(other).sqrt()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PenStroke {
    pub id: u64,
    pub color: u32,
    pub width: f32,
    pub points: Vec<NormalizedPoint>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextAnnotation {
    pub id: u64,
    pub color: u32,
    pub font_size: f32,
    pub position: NormalizedPoint,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DrawAnnotation {
    Stroke(PenStroke),
    Text(TextAnnotation),
}

impl DrawAnnotation {
    pub fn id(&self) -> u64 {
        match self {
            Self::Stroke(s) => s.id,
            Self::Text(t) => t.id,
        }
    }
}

pub const COLOR_RED: u32 = 0xEF4444FF;
pub const COLOR_ORANGE: u32 = 0xF97316FF;
pub const COLOR_YELLOW: u32 = 0xF59E0BFF;
pub const COLOR_GREEN: u32 = 0x10B981FF;
pub const COLOR_BLUE: u32 = 0x3B82F6FF;
pub const COLOR_PURPLE: u32 = 0x8B5CF6FF;
pub const COLOR_WHITE: u32 = 0xFFFFFFFF;
pub const COLOR_BLACK: u32 = 0x18181BFF;

pub const PRESET_COLORS: &[u32] = &[
    COLOR_RED,
    COLOR_YELLOW,
    COLOR_GREEN,
    COLOR_BLUE,
    COLOR_WHITE,
    COLOR_BLACK,
];

pub const STROKE_WIDTH_THIN: f32 = 2.0;
pub const STROKE_WIDTH_MEDIUM: f32 = 4.0;
pub const STROKE_WIDTH_THICK: f32 = 8.0;

pub const PRESET_WIDTHS: &[f32] = &[
    STROKE_WIDTH_THIN,
    STROKE_WIDTH_MEDIUM,
    STROKE_WIDTH_THICK,
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContentViewport {
    pub offset_x: f32,
    pub offset_y: f32,
    pub width: f32,
    pub height: f32,
}

impl ContentViewport {
    pub fn compute(container_w: f32, container_h: f32, content_w: f32, content_h: f32) -> Self {
        if container_w <= 0.0 || container_h <= 0.0 {
            return Self {
                offset_x: 0.0,
                offset_y: 0.0,
                width: container_w.max(1.0),
                height: container_h.max(1.0),
            };
        }
        let content_ratio = if content_w > 0.0 && content_h > 0.0 {
            content_w / content_h
        } else {
            16.0 / 9.0
        };
        let container_ratio = container_w / container_h;

        let (disp_w, disp_h) = if container_ratio > content_ratio {
            (container_h * content_ratio, container_h)
        } else {
            (container_w, container_w / content_ratio)
        };

        let offset_x = (container_w - disp_w) * 0.5;
        let offset_y = (container_h - disp_h) * 0.5;

        Self {
            offset_x,
            offset_y,
            width: disp_w,
            height: disp_h,
        }
    }

    pub fn to_normalized(&self, px: f32, py: f32) -> NormalizedPoint {
        let u = if self.width > 0.0 {
            (px - self.offset_x) / self.width
        } else {
            0.0
        };
        let v = if self.height > 0.0 {
            (py - self.offset_y) / self.height
        } else {
            0.0
        };
        NormalizedPoint::new(u, v)
    }

    pub fn to_pixels(&self, pt: &NormalizedPoint) -> (f32, f32) {
        (
            self.offset_x + pt.x * self.width,
            self.offset_y + pt.y * self.height,
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScreenDrawState {
    pub is_active: bool,
    pub active_tool: DrawTool,
    pub stroke_color: u32,
    pub stroke_width: f32,
    pub text_font_size: f32,
    pub annotations: Vec<DrawAnnotation>,
    pub undo_stack: Vec<Vec<DrawAnnotation>>,
    pub redo_stack: Vec<Vec<DrawAnnotation>>,
    pub current_stroke: Option<PenStroke>,
    pub editing_text_id: Option<u64>,
    pub editing_text_content: String,
    pub next_id: u64,
}

impl Default for ScreenDrawState {
    fn default() -> Self {
        Self {
            is_active: false,
            active_tool: DrawTool::Pen,
            stroke_color: COLOR_RED,
            stroke_width: STROKE_WIDTH_MEDIUM,
            text_font_size: 16.0,
            annotations: Vec::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            current_stroke: None,
            editing_text_id: None,
            editing_text_content: String::new(),
            next_id: 1,
        }
    }
}

impl ScreenDrawState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn toggle_active(&mut self) {
        self.is_active = !self.is_active;
        if !self.is_active {
            self.current_stroke = None;
            self.editing_text_id = None;
        }
    }

    pub fn set_active(&mut self, active: bool) {
        self.is_active = active;
        if !active {
            self.current_stroke = None;
            self.editing_text_id = None;
        }
    }

    pub fn set_tool(&mut self, tool: DrawTool) {
        self.active_tool = tool;
        if tool != DrawTool::Text {
            self.editing_text_id = None;
        }
        if tool != DrawTool::Pen {
            self.current_stroke = None;
        }
    }

    pub fn set_color(&mut self, color: u32) {
        self.stroke_color = color;
    }

    pub fn set_width(&mut self, width: f32) {
        self.stroke_width = width.clamp(1.0, 32.0);
    }

    fn push_undo(&mut self) {
        self.undo_stack.push(self.annotations.clone());
        self.redo_stack.clear();
        if self.undo_stack.len() > 50 {
            self.undo_stack.remove(0);
        }
    }

    pub fn start_stroke(&mut self, x: f32, y: f32) {
        let pt = NormalizedPoint::new(x, y);
        let id = self.next_id;
        self.next_id += 1;
        self.current_stroke = Some(PenStroke {
            id,
            color: self.stroke_color,
            width: self.stroke_width,
            points: vec![pt],
        });
    }

    pub fn add_stroke_point(&mut self, x: f32, y: f32) {
        if let Some(stroke) = self.current_stroke.as_mut() {
            let pt = NormalizedPoint::new(x, y);
            if let Some(last) = stroke.points.last() {
                // Minimum distance squared threshold (~1.5px on 1080p screen) to avoid duplicate points
                if last.distance_squared(&pt) < 0.000002 {
                    return;
                }
            }
            stroke.points.push(pt);
        }
    }

    pub fn finish_stroke(&mut self) {
        if let Some(stroke) = self.current_stroke.take() {
            if !stroke.points.is_empty() {
                self.push_undo();
                self.annotations.push(DrawAnnotation::Stroke(stroke));
            }
        }
    }

    pub fn cancel_stroke(&mut self) {
        self.current_stroke = None;
    }

    pub fn add_text(&mut self, x: f32, y: f32, content: String) -> u64 {
        self.push_undo();
        let id = self.next_id;
        self.next_id += 1;
        let annotation = TextAnnotation {
            id,
            color: self.stroke_color,
            font_size: self.text_font_size,
            position: NormalizedPoint::new(x, y),
            content,
        };
        self.annotations.push(DrawAnnotation::Text(annotation));
        id
    }

    pub fn update_text(&mut self, id: u64, content: String) {
        if let Some(item) = self.annotations.iter_mut().find_map(|ann| match ann {
            DrawAnnotation::Text(t) if t.id == id => Some(t),
            _ => None,
        }) {
            item.content = content;
        }
    }

    pub fn delete_annotation(&mut self, id: u64) -> bool {
        let before_len = self.annotations.len();
        self.annotations.retain(|ann| ann.id() != id);
        if self.annotations.len() != before_len {
            self.push_undo();
            true
        } else {
            false
        }
    }

    pub fn erase_at(&mut self, x: f32, y: f32, radius: f32) -> bool {
        let pt = NormalizedPoint::new(x, y);
        let before_len = self.annotations.len();
        let mut hit = false;

        self.annotations.retain(|ann| match ann {
            DrawAnnotation::Stroke(s) => {
                let stroke_radius = radius + (s.width / 1000.0).max(0.005);
                let hit_stroke = stroke_intersects_point(s, &pt, stroke_radius);
                if hit_stroke {
                    hit = true;
                    false
                } else {
                    true
                }
            }
            DrawAnnotation::Text(t) => {
                let text_radius = radius + (t.font_size / 1000.0).max(0.015);
                let hit_text = text_intersects_point(t, &pt, text_radius);
                if hit_text {
                    hit = true;
                    false
                } else {
                    true
                }
            }
        });

        if hit && self.annotations.len() != before_len {
            self.push_undo();
            true
        } else {
            false
        }
    }

    pub fn undo(&mut self) -> bool {
        if let Some(prev) = self.undo_stack.pop() {
            let current = std::mem::replace(&mut self.annotations, prev);
            self.redo_stack.push(current);
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        if let Some(next) = self.redo_stack.pop() {
            let current = std::mem::replace(&mut self.annotations, next);
            self.undo_stack.push(current);
            true
        } else {
            false
        }
    }

    pub fn start_editing_text(&mut self, id: u64) {
        if let Some(ann) = self.annotations.iter().find_map(|a| match a {
            DrawAnnotation::Text(t) if t.id == id => Some(t),
            _ => None,
        }) {
            self.editing_text_content = ann.content.clone();
            self.editing_text_id = Some(id);
        }
    }

    pub fn commit_editing_text(&mut self) {
        if let Some(id) = self.editing_text_id.take() {
            let trimmed = self.editing_text_content.trim();
            if trimmed.is_empty() {
                self.delete_annotation(id);
            } else {
                let content = std::mem::take(&mut self.editing_text_content);
                self.update_text(id, content);
            }
        }
    }

    pub fn cancel_editing_text(&mut self) {
        if let Some(id) = self.editing_text_id.take() {
            if let Some(ann) = self.annotations.iter().find_map(|a| match a {
                DrawAnnotation::Text(t) if t.id == id => Some(t),
                _ => None,
            }) {
                if ann.content.trim().is_empty() {
                    self.delete_annotation(id);
                }
            }
            self.editing_text_content.clear();
        }
    }

    pub fn clear(&mut self) {
        if !self.annotations.is_empty() {
            self.push_undo();
            self.annotations.clear();
            self.editing_text_id = None;
            self.editing_text_content.clear();
            self.current_stroke = None;
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }
}

fn stroke_intersects_point(stroke: &PenStroke, pt: &NormalizedPoint, radius: f32) -> bool {
    let r2 = radius * radius;
    if stroke.points.len() == 1 {
        return stroke.points[0].distance_squared(pt) <= r2;
    }
    for window in stroke.points.windows(2) {
        let a = &window[0];
        let b = &window[1];
        if point_to_segment_distance_squared(pt, a, b) <= r2 {
            return true;
        }
    }
    false
}

fn text_intersects_point(text: &TextAnnotation, pt: &NormalizedPoint, radius: f32) -> bool {
    let approx_width = (text.content.len() as f32 * 0.012).clamp(0.02, 0.4);
    let approx_height = (text.font_size / 600.0).clamp(0.02, 0.08);

    let left = text.position.x - radius;
    let right = text.position.x + approx_width + radius;
    let top = text.position.y - radius;
    let bottom = text.position.y + approx_height + radius;

    pt.x >= left && pt.x <= right && pt.y >= top && pt.y <= bottom
}

fn point_to_segment_distance_squared(p: &NormalizedPoint, a: &NormalizedPoint, b: &NormalizedPoint) -> f32 {
    let vx = b.x - a.x;
    let vy = b.y - a.y;
    let l2 = vx * vx + vy * vy;
    if l2 == 0.0 {
        return p.distance_squared(a);
    }
    let wx = p.x - a.x;
    let wy = p.y - a.y;
    let t = ((wx * vx + wy * vy) / l2).clamp(0.0, 1.0);
    let proj_x = a.x + t * vx;
    let proj_y = a.y + t * vy;
    let dx = p.x - proj_x;
    let dy = p.y - proj_y;
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_draw_state_defaults() {
        let state = ScreenDrawState::default();
        assert!(!state.is_active);
        assert_eq!(state.active_tool, DrawTool::Pen);
        assert_eq!(state.stroke_color, COLOR_RED);
        assert!(state.annotations.is_empty());
    }

    #[test]
    fn test_pen_stroke_lifecycle() {
        let mut state = ScreenDrawState::default();
        state.set_active(true);
        state.start_stroke(0.1, 0.2);
        state.add_stroke_point(0.15, 0.25);
        state.add_stroke_point(0.2, 0.3);
        assert!(state.current_stroke.is_some());
        state.finish_stroke();

        assert_eq!(state.annotations.len(), 1);
        assert!(state.current_stroke.is_none());
        assert!(state.can_undo());

        if let DrawAnnotation::Stroke(s) = &state.annotations[0] {
            assert_eq!(s.points.len(), 3);
            assert_eq!(s.color, COLOR_RED);
        } else {
            panic!("Expected stroke annotation");
        }
    }

    #[test]
    fn test_text_annotation() {
        let mut state = ScreenDrawState::default();
        let id = state.add_text(0.3, 0.4, "Hello Mezon".to_string());
        assert_eq!(state.annotations.len(), 1);
        state.update_text(id, "Updated text".to_string());

        if let DrawAnnotation::Text(t) = &state.annotations[0] {
            assert_eq!(t.content, "Updated text");
            assert_eq!(t.position.x, 0.3);
            assert_eq!(t.position.y, 0.4);
        } else {
            panic!("Expected text annotation");
        }
    }

    #[test]
    fn test_eraser_hit_test() {
        let mut state = ScreenDrawState::default();
        state.start_stroke(0.1, 0.1);
        state.add_stroke_point(0.5, 0.5);
        state.finish_stroke();
        assert_eq!(state.annotations.len(), 1);

        // Erase far away - should not hit
        let erased = state.erase_at(0.9, 0.9, 0.05);
        assert!(!erased);
        assert_eq!(state.annotations.len(), 1);

        // Erase directly on midpoint (0.3, 0.3)
        let erased = state.erase_at(0.3, 0.3, 0.05);
        assert!(erased);
        assert_eq!(state.annotations.len(), 0);
    }

    #[test]
    fn test_undo_redo_and_clear() {
        let mut state = ScreenDrawState::default();
        state.start_stroke(0.1, 0.1);
        state.add_stroke_point(0.2, 0.2);
        state.finish_stroke();

        state.add_text(0.5, 0.5, "Text".to_string());
        assert_eq!(state.annotations.len(), 2);

        // Undo text
        assert!(state.undo());
        assert_eq!(state.annotations.len(), 1);

        // Redo text
        assert!(state.redo());
        assert_eq!(state.annotations.len(), 2);

        // Clear all
        state.clear();
        assert_eq!(state.annotations.len(), 0);

        // Undo clear
        assert!(state.undo());
        assert_eq!(state.annotations.len(), 2);
    }

    #[test]
    fn test_content_viewport_aspect_ratio_contain() {
        // Container wider than 16:9 content (pillarbox)
        let vp = ContentViewport::compute(1920.0, 800.0, 1920.0, 1080.0);
        assert!((vp.height - 800.0).abs() < 0.01);
        let expected_w = 800.0 * (16.0 / 9.0);
        assert!((vp.width - expected_w).abs() < 0.01);
        assert!((vp.offset_x - (1920.0 - expected_w) * 0.5).abs() < 0.01);
        assert_eq!(vp.offset_y, 0.0);

        // Normalized mapping roundtrip
        let norm = vp.to_normalized(vp.offset_x + 0.5 * vp.width, vp.offset_y + 0.5 * vp.height);
        assert!((norm.x - 0.5).abs() < 0.001);
        assert!((norm.y - 0.5).abs() < 0.001);

        let (px, py) = vp.to_pixels(&norm);
        assert!((px - (vp.offset_x + 0.5 * vp.width)).abs() < 0.01);
        assert!((py - (vp.offset_y + 0.5 * vp.height)).abs() < 0.01);
    }

    #[test]
    fn test_text_editing_commit_and_cancel() {
        let mut state = ScreenDrawState::default();
        let id = state.add_text(0.2, 0.2, String::new());
        state.start_editing_text(id);
        assert_eq!(state.editing_text_id, Some(id));

        state.editing_text_content = "   ".to_string();
        // Commit whitespace only - should delete empty annotation
        state.commit_editing_text();
        assert_eq!(state.annotations.len(), 0);
        assert_eq!(state.editing_text_id, None);

        let id2 = state.add_text(0.3, 0.3, "Initial".to_string());
        state.start_editing_text(id2);
        state.editing_text_content = "Updated note".to_string();
        state.commit_editing_text();
        assert_eq!(state.annotations.len(), 1);
        if let DrawAnnotation::Text(t) = &state.annotations[0] {
            assert_eq!(t.content, "Updated note");
        }
    }
}
