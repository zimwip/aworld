//! Text: the letters of a TrueType font, rasterized once per size into an atlas texture and drawn
//! as textured quads, laid out the way macroquad laid them out.

use super::batch::{geometry, Vertex};
use super::math::{Color, Rect};
use super::texture::{FilterMode, Texture2D};
use super::{dpi_scale, WHITE};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy)]
struct Glyph {
    offset_x: i32,
    offset_y: i32,
    advance: f32,
    /// Where it is in the atlas, in pixels.
    rect: Rect,
}

/// Letters packed in rows (shelves) into one RGBA texture: white, the coverage in the alpha.
struct Atlas {
    pixels: Vec<u8>,
    size: u32,
    texture: Texture2D,
    /// The next free place: x along the current shelf, its top, its height so far.
    x: u32,
    y: u32,
    row: u32,
}

impl Atlas {
    fn new(size: u32) -> Atlas {
        let pixels = [255, 255, 255, 0].repeat((size * size) as usize);
        let texture = Texture2D::from_rgba8(size as u16, size as u16, &pixels);
        texture.set_filter(FilterMode::Linear);
        Atlas { pixels, size, texture, x: 0, y: 0, row: 0 }
    }

    /// Finds room for a `w` × `h` letter (with a pixel of margin), growing the atlas if it is full.
    fn place(&mut self, w: u32, h: u32) -> (u32, u32) {
        if self.x + w + 1 > self.size {
            self.x = 0;
            self.y += self.row + 1;
            self.row = 0;
        }
        if self.y + h + 1 > self.size {
            self.grow();
            return self.place(w, h);
        }
        let at = (self.x, self.y);
        self.x += w + 1;
        self.row = self.row.max(h);
        at
    }

    /// Twice as big, the letters where they were (what was drawn already keeps the old texture).
    fn grow(&mut self) {
        let size = self.size * 2;
        let mut pixels = [255, 255, 255, 0].repeat((size * size) as usize);
        for y in 0..self.size as usize {
            let row = self.size as usize * 4;
            pixels[y * size as usize * 4..][..row].copy_from_slice(&self.pixels[y * row..][..row]);
        }
        self.pixels = pixels;
        self.size = size;
        self.texture = Texture2D::from_rgba8(size as u16, size as u16, &self.pixels);
        self.texture.set_filter(FilterMode::Linear);
    }
}

struct Inner {
    font: fontdue::Font,
    glyphs: RefCell<HashMap<(char, u16), Glyph>>,
    atlas: RefCell<Atlas>,
}

/// A font. Clones share it.
#[derive(Clone)]
pub struct Font(Rc<Inner>);

impl std::fmt::Debug for Font {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("Font")
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TextDimensions {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone)]
pub struct TextParams<'a> {
    pub font: Option<&'a Font>,
    pub font_size: u16,
    pub font_scale: f32,
    pub color: Color,
}

impl Default for TextParams<'_> {
    fn default() -> Self {
        TextParams { font: None, font_size: 20, font_scale: 1.0, color: WHITE }
    }
}

pub fn load_ttf_font_from_bytes(bytes: &[u8]) -> Result<Font, String> {
    let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).map_err(|e| e.to_string())?;
    Ok(Font(Rc::new(Inner { font, glyphs: RefCell::new(HashMap::new()), atlas: RefCell::new(Atlas::new(512)) })))
}

thread_local! {
    static DEFAULT_FONT: Font = load_ttf_font_from_bytes(include_bytes!("../../assets/font.ttf")).expect("font");
}

fn default_font() -> Font {
    DEFAULT_FONT.with(|f| f.clone())
}

impl Font {
    fn glyph(&self, c: char, size: u16) -> Glyph {
        if let Some(g) = self.0.glyphs.borrow().get(&(c, size)) {
            return *g;
        }
        let (m, coverage) = self.0.font.rasterize(c, size as f32);
        let (w, h) = (m.width as u32, m.height as u32);
        let mut atlas = self.0.atlas.borrow_mut();
        let (x, y) = atlas.place(w, h);
        let rgba: Vec<u8> = coverage.iter().flat_map(|&a| [255, 255, 255, a]).collect();
        let size_px = atlas.size as usize;
        for row in 0..h as usize {
            let dst = ((y as usize + row) * size_px + x as usize) * 4;
            atlas.pixels[dst..dst + w as usize * 4].copy_from_slice(&rgba[row * w as usize * 4..][..w as usize * 4]);
        }
        atlas.texture.update_part(x, y, w, h, &rgba);
        let g = Glyph { offset_x: m.xmin, offset_y: m.ymin, advance: m.advance_width, rect: Rect::new(x as f32, y as f32, w as f32, h as f32) };
        self.0.glyphs.borrow_mut().insert((c, size), g);
        g
    }

    fn measure(&self, text: &str, size: u16, scale: f32) -> TextDimensions {
        let dpi = dpi_scale();
        let px = (size as f32 * dpi).ceil() as u16;
        let (mut width, mut min_y, mut max_y) = (0.0, f32::MAX, f32::MIN);
        for c in text.chars() {
            let g = self.glyph(c, px);
            let oy = g.offset_y as f32 * scale;
            width += g.advance * scale;
            min_y = f32::min(min_y, oy);
            max_y = f32::max(max_y, g.rect.h * scale + oy);
        }
        if text.is_empty() {
            return TextDimensions::default();
        }
        TextDimensions { width: width / dpi, height: (max_y - min_y) / dpi }
    }
}

pub fn measure_text(text: &str, font: Option<&Font>, font_size: u16, font_scale: f32) -> TextDimensions {
    let f;
    let font = match font {
        Some(f) => f,
        None => {
            f = default_font();
            &f
        }
    };
    font.measure(text, font_size, font_scale)
}

pub fn draw_text(text: &str, x: f32, y: f32, font_size: f32, color: Color) -> TextDimensions {
    draw_text_ex(text, x, y, TextParams { font_size: font_size as u16, color, ..Default::default() })
}

/// Draws `text` with its baseline at `y`.
pub fn draw_text_ex(text: &str, x: f32, y: f32, params: TextParams) -> TextDimensions {
    if text.is_empty() {
        return TextDimensions::default();
    }
    let f;
    let font = match params.font {
        Some(f) => f,
        None => {
            f = default_font();
            &f
        }
    };
    let dpi = dpi_scale();
    let scale = params.font_scale;
    let px = (params.font_size as f32 * dpi).ceil() as u16;
    let (mut total, mut min_y, mut max_y) = (0.0f32, f32::MAX, f32::MIN);
    let mut last = None;
    let mut vertices = Vec::with_capacity(text.len() * 4);
    let mut indices = Vec::with_capacity(text.len() * 6);
    // Every letter into the atlas first: one of them may grow it, and the texture coordinates
    // are fractions of its final size.
    let glyphs: Vec<(char, Glyph)> = text.chars().map(|c| (c, font.glyph(c, px))).collect();
    let s = font.0.atlas.borrow().size as f32;
    for (c, g) in glyphs {
        let kern = last.and_then(|l| font.0.font.horizontal_kern(l, c, px as f32)).unwrap_or(0.0);
        last = Some(c);
        let (ox, oy) = (g.offset_x as f32 * scale, g.offset_y as f32 * scale);
        let gh = g.rect.h * scale;
        min_y = min_y.min(oy);
        max_y = max_y.max(gh + oy);
        let dx = (ox + total) / dpi + x + kern / dpi;
        let dy = (-gh - oy) / dpi + y;
        let (w, h) = (g.rect.w / dpi * scale, g.rect.h / dpi * scale);
        total += g.advance * scale + kern;
        if g.rect.w == 0.0 || g.rect.h == 0.0 {
            continue;
        }
        let (u0, v0, u1, v1) = (g.rect.x / s, g.rect.y / s, (g.rect.x + g.rect.w) / s, (g.rect.y + g.rect.h) / s);
        let k = vertices.len() as u16;
        vertices.extend_from_slice(&[
            Vertex::new(dx, dy, 0.0, u0, v0, params.color),
            Vertex::new(dx + w, dy, 0.0, u1, v0, params.color),
            Vertex::new(dx + w, dy + h, 0.0, u1, v1, params.color),
            Vertex::new(dx, dy + h, 0.0, u0, v1, params.color),
        ]);
        indices.extend_from_slice(&[k, k + 1, k + 2, k, k + 2, k + 3]);
    }
    let texture = font.0.atlas.borrow().texture.clone();
    geometry(Some(&texture), &vertices, &indices);
    TextDimensions { width: total / dpi, height: (max_y - min_y) / dpi }
}
