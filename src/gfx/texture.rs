//! Pictures: `Image` on the CPU, `Texture2D` on the card, `RenderTarget` a texture drawn into.

use super::batch::{geometry, Vertex};
use super::kit;
use crate::world::gpu::Gpu;
use super::math::{Color, Rect, Vec2};
use std::sync::{Arc, Mutex};

/// The format of every texture, the screen's included: plain 8-bit RGBA, blended as it is
/// (no sRGB conversion), as the game always was.
pub(super) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// RGBA pixels, row by row from the top.
#[derive(Clone)]
pub struct Image {
    pub bytes: Vec<u8>,
    pub width: u16,
    pub height: u16,
}

impl Image {
    pub fn gen_image_color(width: u16, height: u16, color: Color) -> Image {
        let px: [u8; 4] = color.into();
        Image { bytes: px.repeat(width as usize * height as usize), width, height }
    }

    pub fn get_image_data_mut(&mut self) -> &mut [[u8; 4]] {
        bytemuck::cast_slice_mut(&mut self.bytes)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FilterMode {
    Linear,
    Nearest,
}

struct Inner {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    w: u32,
    h: u32,
    filter: Mutex<FilterMode>,
    bind: Mutex<Option<wgpu::BindGroup>>,
}

/// A texture on the card. Clones share it (and it can go to the simulation's thread with the
/// game: it is only drawn from the main thread).
#[derive(Clone)]
pub struct Texture2D(Arc<Inner>);

impl Texture2D {
    fn new(w: u32, h: u32, target: bool) -> Texture2D {
        Texture2D::new_on(kit().gpu, w, h, target)
    }

    fn new_on(gpu: &Gpu, w: u32, h: u32, target: bool) -> Texture2D {
        let mut usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        if target {
            usage |= wgpu::TextureUsages::RENDER_ATTACHMENT;
        }
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("texture"),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Texture2D(Arc::new(Inner { texture, view, w: w.max(1), h: h.max(1), filter: Mutex::new(FilterMode::Linear), bind: Mutex::new(None) }))
    }

    /// One white pixel, for plain colours (made before the kit, which holds it).
    pub(super) fn white(gpu: &Gpu) -> Texture2D {
        let t = Texture2D::new_on(gpu, 1, 1, false);
        t.write(gpu, 0, 0, 1, 1, &[255; 4]);
        t
    }

    pub fn from_image(image: &Image) -> Texture2D {
        let t = Texture2D::new(image.width as u32, image.height as u32, false);
        t.update(image);
        t
    }

    pub fn from_rgba8(width: u16, height: u16, bytes: &[u8]) -> Texture2D {
        Texture2D::from_image(&Image { bytes: bytes.to_vec(), width, height })
    }

    /// Replaces the whole texture with `image` (of the same size).
    pub fn update(&self, image: &Image) {
        self.update_part(0, 0, image.width as u32, image.height as u32, &image.bytes);
    }

    /// Replaces the `w` × `h` pixels at (`x`, `y`) with the same pixels of `image` (of the same
    /// size as the texture): only that part goes to the card.
    pub fn update_rect(&self, image: &Image, x: u32, y: u32, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        let row = 4 * image.width as u32;
        kit().gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.0.texture, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
            &image.bytes,
            wgpu::TexelCopyBufferLayout { offset: (y * row + 4 * x) as u64, bytes_per_row: Some(row), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }

    /// Replaces the `w` × `h` pixels at (`x`, `y`) with `bytes` (RGBA, row by row).
    pub fn update_part(&self, x: u32, y: u32, w: u32, h: u32, bytes: &[u8]) {
        self.write(kit().gpu, x, y, w, h, bytes);
    }

    fn write(&self, gpu: &Gpu, x: u32, y: u32, w: u32, h: u32, bytes: &[u8]) {
        if w == 0 || h == 0 {
            return;
        }
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.0.texture, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
            bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }

    pub fn set_filter(&self, filter: FilterMode) {
        if std::mem::replace(&mut *self.0.filter.lock().unwrap(), filter) != filter {
            *self.0.bind.lock().unwrap() = None;
        }
    }

    pub fn width(&self) -> f32 {
        self.0.w as f32
    }

    pub fn height(&self) -> f32 {
        self.0.h as f32
    }

    pub(super) fn same(&self, other: &Texture2D) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub(super) fn view(&self) -> &wgpu::TextureView {
        &self.0.view
    }

    pub(super) fn sampler(&self) -> &'static wgpu::Sampler {
        let k = kit();
        match *self.0.filter.lock().unwrap() {
            FilterMode::Linear => &k.linear,
            FilterMode::Nearest => &k.nearest,
        }
    }

    /// The texture and its sampler, as the shaders' group 1.
    pub(super) fn bind_group(&self) -> wgpu::BindGroup {
        self.0
            .bind
            .lock()
            .unwrap()
            .get_or_insert_with(|| {
                kit().gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("texture"),
                    layout: &kit().texture_layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.0.view) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(self.sampler()) },
                    ],
                })
            })
            .clone()
    }
}

/// A texture to draw into (see `set_target`).
#[derive(Clone)]
pub struct RenderTarget {
    pub texture: Texture2D,
    /// Its pixels to a unit of what is drawn into it.
    pub scale: f32,
}

impl RenderTarget {
    pub(super) fn same(a: &Option<RenderTarget>, b: &Option<RenderTarget>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => a.texture.same(&b.texture),
            _ => false,
        }
    }
}

pub fn render_target(width: u32, height: u32) -> RenderTarget {
    RenderTarget { texture: Texture2D::new(width, height, true), scale: 1.0 }
}

/// A target `width` × `height` units, of `scale` pixels each (the screen's points: see `dpi_scale`).
pub fn render_target_scaled(width: f32, height: f32, scale: f32) -> RenderTarget {
    RenderTarget { texture: Texture2D::new((width * scale).ceil() as u32, (height * scale).ceil() as u32, true), scale }
}

#[derive(Debug, Clone, Default)]
pub struct DrawTextureParams {
    /// The size on the screen (default: the texture's, or the source's).
    pub dest_size: Option<Vec2>,
    /// The part of the texture drawn, in its pixels (default: all of it).
    pub source: Option<Rect>,
}

/// Draws `texture` with its top left corner at (`x`, `y`), tinted by `color`.
pub fn draw_texture_ex(texture: &Texture2D, x: f32, y: f32, color: Color, params: DrawTextureParams) {
    let (tw, th) = (texture.width(), texture.height());
    let src = params.source.unwrap_or(Rect::new(0.0, 0.0, tw, th));
    let size = params.dest_size.unwrap_or(src.size());
    let (u0, v0, u1, v1) = (src.x / tw, src.y / th, (src.x + src.w) / tw, (src.y + src.h) / th);
    let (w, h) = (size.x, size.y);
    let v = [
        Vertex::new(x, y, 0.0, u0, v0, color),
        Vertex::new(x + w, y, 0.0, u1, v0, color),
        Vertex::new(x + w, y + h, 0.0, u1, v1, color),
        Vertex::new(x, y + h, 0.0, u0, v1, color),
    ];
    geometry(Some(texture), &v, &[0, 1, 2, 0, 2, 3]);
}
