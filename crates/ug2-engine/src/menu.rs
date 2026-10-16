//! Touch-first menu lists: a title and rows, each either a button or a value with
//! ‹ › arrows. Rows are big enough for a thumb; keys and gamepads move a highlight.

use glam::{vec2, Vec2};

use crate::hud::{Canvas, Rgba};

const ACCENT: Rgba = [0.25, 0.85, 1.0, 1.0];
const WHITE: Rgba = [1.0, 1.0, 1.0, 1.0];

#[derive(Clone, Debug)]
pub struct MenuItem {
    pub label: String,
    /// Shown on the right with ‹ › arrows when present.
    pub value: Option<String>,
}

impl MenuItem {
    pub fn button(label: impl Into<String>) -> Self {
        Self { label: label.into(), value: None }
    }

    pub fn choice(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self { label: label.into(), value: Some(value.into()) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuHit {
    Activate(usize),
    Prev(usize),
    Next(usize),
}

#[derive(Default)]
pub struct Menu {
    pub title: String,
    pub subtitle: String,
    pub items: Vec<MenuItem>,
    pub selected: usize,
    /// Keep to the left two thirds of the screen (the garage shows the car on the right).
    pub anchor_left: bool,
    /// Row rectangles from the last draw: (min, max).
    rects: Vec<(Vec2, Vec2)>,
}

impl Menu {
    pub fn new(title: impl Into<String>, items: Vec<MenuItem>) -> Self {
        Self { title: title.into(), items, ..Default::default() }
    }

    pub fn draw(&mut self, c: &mut Canvas, size: Vec2, dp: f32) {
        let s = dp;
        let title_h = if self.subtitle.is_empty() { 40.0 * s } else { 58.0 * s };
        let n = self.items.len().max(1);
        // Long lists go into two columns rather than shrinking rows below a thumb's size.
        let avail = size.y - title_h - 24.0 * s;
        let cols = if n as f32 * 38.0 * s > avail { 2 } else { 1 };
        let per_col = n.div_ceil(cols);
        let fit = avail / per_col as f32;
        let row_h = (46.0 * s).min(fit * 0.85).max(30.0 * s);
        let gap = (8.0 * s).min(fit * 0.15).max(3.0 * s);
        let col_gap = 10.0 * s;
        let total_w = if self.anchor_left {
            size.x * 0.64
        } else {
            let one = (size.x * 0.62).clamp(280.0 * s, 560.0 * s);
            (one * if cols == 2 { 1.6 } else { 1.0 }).min(size.x - 32.0 * s)
        };
        let col_w = (total_w - col_gap * (cols - 1) as f32) / cols as f32;
        let x_start = if self.anchor_left { 16.0 * s } else { (size.x - total_w) * 0.5 };
        if self.anchor_left {
            c.rect(0.0, 0.0, x_start + total_w + 16.0 * s, size.y, [0.0, 0.0, 0.0, 0.5]);
        } else {
            c.rect(0.0, 0.0, size.x, size.y, [0.0, 0.0, 0.0, 0.55]);
        }
        let total_h = per_col as f32 * (row_h + gap) - gap;
        let y0 = ((size.y - total_h - title_h) * 0.5).max(12.0 * s) + title_h;
        let tx = x_start + total_w * 0.5;
        c.text(tx, y0 - title_h, 3.2 * s, 0.5, &self.title, WHITE);
        if !self.subtitle.is_empty() {
            c.text(tx, y0 - title_h + 30.0 * s, 1.6 * s, 0.5, &self.subtitle, [1.0, 1.0, 1.0, 0.7]);
        }
        self.rects.clear();
        for (i, item) in self.items.iter().enumerate() {
            let (col, row) = (i / per_col, i % per_col);
            let x0 = x_start + col as f32 * (col_w + col_gap);
            let y = y0 + row as f32 * (row_h + gap);
            let width = col_w;
            let sel = i == self.selected;
            let bg = if sel { [ACCENT[0], ACCENT[1], ACCENT[2], 0.35] } else { [0.0, 0.0, 0.0, 0.6] };
            c.rect(x0, y, width, row_h, bg);
            c.rect(x0, y, 4.0 * s, row_h, if sel { ACCENT } else { [1.0, 1.0, 1.0, 0.25] });
            let fit_text = |t: &str, max_w: f32| (2.0 * s).min(max_w / Canvas::text_width(t, 1.0).max(1.0));
            match &item.value {
                None => {
                    let size = fit_text(&item.label, width - 24.0 * s);
                    c.text(x0 + width * 0.5, y + row_h * 0.5 - 3.5 * size, size, 0.5, &item.label, WHITE)
                }
                Some(v) => {
                    let ls = fit_text(&item.label, width * 0.38);
                    c.text(x0 + 16.0 * s, y + row_h * 0.5 - 3.5 * ls, ls, 0.0, &item.label, [1.0, 1.0, 1.0, 0.75]);
                    let vx = x0 + width * 0.68;
                    let arrow = 9.0 * s;
                    let cy = y + row_h * 0.5;
                    c.triangle(vec2(vx - width * 0.26, cy), vec2(vx - width * 0.26 + arrow, cy - arrow), vec2(vx - width * 0.26 + arrow, cy + arrow), ACCENT);
                    c.triangle(vec2(x0 + width - 14.0 * s, cy), vec2(x0 + width - 14.0 * s - arrow, cy - arrow), vec2(x0 + width - 14.0 * s - arrow, cy + arrow), ACCENT);
                    let max_w = width * 0.4;
                    let size = (2.0 * s).min(max_w / Canvas::text_width(v, 1.0).max(1.0));
                    c.text(vx + 4.0 * s, cy - 3.5 * size, size, 0.5, v, WHITE);
                }
            }
            self.rects.push((vec2(x0, y), vec2(x0 + width, y + row_h)));
        }
    }

    /// What a tap at `p` does: value rows split into ‹ (left part) and › (right part).
    pub fn hit(&mut self, p: Vec2) -> Option<MenuHit> {
        let i = self.rects.iter().position(|(a, b)| p.x >= a.x && p.x <= b.x && p.y >= a.y && p.y <= b.y)?;
        self.selected = i;
        let (a, b) = self.rects[i];
        Some(match self.items[i].value {
            None => MenuHit::Activate(i),
            Some(_) => {
                if p.x < a.x + (b.x - a.x) * 0.66 {
                    MenuHit::Prev(i)
                } else {
                    MenuHit::Next(i)
                }
            }
        })
    }

    pub fn move_selection(&mut self, delta: i32) {
        let n = self.items.len() as i32;
        if n > 0 {
            self.selected = (self.selected as i32 + delta).rem_euclid(n) as usize;
        }
    }
}
