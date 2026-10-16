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
    /// Row rectangles from the last draw: (min, max).
    rects: Vec<(Vec2, Vec2)>,
}

impl Menu {
    pub fn new(title: impl Into<String>, items: Vec<MenuItem>) -> Self {
        Self { title: title.into(), items, ..Default::default() }
    }

    pub fn draw(&mut self, c: &mut Canvas, size: Vec2, dp: f32) {
        let s = dp;
        c.rect(0.0, 0.0, size.x, size.y, [0.0, 0.0, 0.0, 0.55]);
        let title_h = if self.subtitle.is_empty() { 40.0 * s } else { 58.0 * s };
        let n = self.items.len().max(1) as f32;
        // Rows shrink (down to 30 dp) when there are many, so the list always fits.
        let fit = (size.y - title_h - 24.0 * s) / n;
        let row_h = (46.0 * s).min(fit * 0.85).max(30.0 * s);
        let gap = (8.0 * s).min(fit * 0.15).max(3.0 * s);
        let width = (size.x * 0.62).clamp(280.0 * s, 560.0 * s).min(size.x - 32.0 * s);
        let total = n * (row_h + gap) - gap;
        let x0 = (size.x - width) * 0.5;
        let mut y = ((size.y - total - title_h) * 0.5).max(12.0 * s) + title_h;

        c.text(size.x * 0.5, y - title_h, 3.2 * s, 0.5, &self.title, WHITE);
        if !self.subtitle.is_empty() {
            c.text(size.x * 0.5, y - title_h + 30.0 * s, 1.6 * s, 0.5, &self.subtitle, [1.0, 1.0, 1.0, 0.7]);
        }
        self.rects.clear();
        for (i, item) in self.items.iter().enumerate() {
            let sel = i == self.selected;
            let bg = if sel { [ACCENT[0], ACCENT[1], ACCENT[2], 0.35] } else { [0.0, 0.0, 0.0, 0.6] };
            c.rect(x0, y, width, row_h, bg);
            c.rect(x0, y, 4.0 * s, row_h, if sel { ACCENT } else { [1.0, 1.0, 1.0, 0.25] });
            let ty = y + row_h * 0.5 - 7.0 * s;
            match &item.value {
                None => c.text(x0 + width * 0.5, ty, 2.0 * s, 0.5, &item.label, WHITE),
                Some(v) => {
                    c.text(x0 + 16.0 * s, ty, 2.0 * s, 0.0, &item.label, [1.0, 1.0, 1.0, 0.75]);
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
            y += row_h + gap;
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
