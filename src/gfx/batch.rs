//! The frame being drawn: every shape, texture and letter of it goes into one list of triangles,
//! cut into draw calls where the texture or the material changes and into passes where the
//! target changes, then sent to the card in one go at the end of the frame (`flush`).

use super::material::{default_material, Material};
use super::math::{Color, Mat4};
use super::texture::{RenderTarget, Texture2D};
use super::{kit, Kit};
use std::cell::RefCell;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub color: [u8; 4],
}

impl Vertex {
    pub fn new(x: f32, y: f32, z: f32, u: f32, v: f32, color: Color) -> Vertex {
        Vertex { position: [x, y, z], uv: [u, v], color: color.into() }
    }
}

/// Uniforms go into one buffer for the frame, each block at a multiple of this.
pub(super) const UNIFORM_ALIGN: usize = 256;

struct Call {
    material: Material,
    /// `Material::stamp` when the call was opened: a call only grows while the material's
    /// uniforms and textures stay as they were.
    stamp: u64,
    texture: Texture2D,
    extra: Vec<Texture2D>,
    /// Where the material's uniforms are in the frame's uniform buffer.
    uniforms: Option<u32>,
    first: u32,
    count: u32,
}

struct Pass {
    target: Option<RenderTarget>,
    clear: Option<Color>,
    calls: std::ops::Range<usize>,
    /// Where the projection is in the frame's uniform buffer.
    globals: u32,
}

pub(super) struct Batch {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    calls: Vec<Call>,
    passes: Vec<Pass>,
    uniforms: Vec<u8>,
    material: Option<Material>,
    target: Option<RenderTarget>,
    shift: Vec<f32>,
    /// The screen's size, in points.
    pub screen: (f32, f32),
    /// Whether the frame texture already holds something drawn this frame.
    screen_started: bool,
}

thread_local! {
    static BATCH: RefCell<Batch> = RefCell::new(Batch {
        vertices: vec![],
        indices: vec![],
        calls: vec![],
        passes: vec![],
        uniforms: vec![],
        material: None,
        target: None,
        shift: vec![],
        screen: (1.0, 1.0),
        screen_started: false,
    });
}

pub(super) fn with<R>(f: impl FnOnce(&mut Batch) -> R) -> R {
    BATCH.with(|b| f(&mut b.borrow_mut()))
}

impl Batch {
    fn push_uniforms(&mut self, bytes: &[u8]) -> u32 {
        let at = self.uniforms.len().next_multiple_of(UNIFORM_ALIGN);
        self.uniforms.resize(at, 0);
        self.uniforms.extend_from_slice(bytes);
        at as u32
    }

    /// The pass that draws into the current target, opened if need be.
    fn pass(&mut self) -> &mut Pass {
        if !self.passes.last().is_some_and(|p| RenderTarget::same(&p.target, &self.target)) {
            self.open_pass();
        }
        self.passes.last_mut().unwrap()
    }

    fn open_pass(&mut self) {
        let (w, h) = match &self.target {
            Some(t) => (t.texture.width() / t.scale, t.texture.height() / t.scale),
            None => self.screen,
        };
        let proj = Mat4::orthographic_rh(0.0, w, h, 0.0, -1.0, 1.0);
        let globals = self.push_uniforms(bytemuck::bytes_of(&proj.to_cols_array()));
        let n = self.calls.len();
        self.passes.push(Pass { target: self.target.clone(), clear: None, calls: n..n, globals });
    }
}

/// Adds triangles to the frame, with `texture` (none: plain colour) and the material in use.
pub(super) fn geometry(texture: Option<&Texture2D>, vertices: &[Vertex], indices: &[u16]) {
    if indices.is_empty() {
        return;
    }
    let white;
    let texture = match texture {
        Some(t) => t,
        None => {
            white = kit().white.clone();
            &white
        }
    };
    with(|b| {
        let material = b.material.clone().unwrap_or_else(default_material);
        b.pass();
        let stamp = material.stamp();
        let open = b.passes.last().unwrap().calls.end == b.calls.len()
            && b.calls.last().is_some_and(|c| c.material.same(&material) && c.stamp == stamp && c.texture.same(texture) && b.passes.last().unwrap().calls.start < b.calls.len());
        if !open {
            let uniforms = material.uniform_bytes().map(|u| b.push_uniforms(&u));
            let first = b.indices.len() as u32;
            let extra = material.textures();
            b.calls.push(Call { material, stamp, texture: texture.clone(), extra, uniforms, first, count: 0 });
            let n = b.calls.len();
            b.passes.last_mut().unwrap().calls.end = n;
        }
        let base = b.vertices.len() as u32;
        let dx: f32 = b.shift.iter().sum();
        b.vertices.extend(vertices.iter().map(|v| Vertex { position: [v.position[0] + dx, v.position[1], v.position[2]], ..*v }));
        b.indices.extend(indices.iter().map(|&i| base + i as u32));
        b.calls.last_mut().unwrap().count += indices.len() as u32;
    });
}

/// Draws with `material` until `gl_use_default_material`.
pub fn gl_use_material(material: &Material) {
    with(|b| b.material = Some(material.clone()));
}

pub fn gl_use_default_material() {
    with(|b| b.material = None);
}

/// Draws into `target` (in its own pixels, top left at (0, 0)) until `set_target(None)`.
pub fn set_target(target: Option<&RenderTarget>) {
    with(|b| b.target = target.cloned());
}

/// Fills the current target with `color`, dropping what was drawn in it before.
pub fn clear_background(color: Color) {
    with(|b| {
        let busy = b.passes.last().is_some_and(|p| RenderTarget::same(&p.target, &b.target) && !p.calls.is_empty());
        if busy {
            // A pass of its own, so that what was drawn before still happens first.
            b.open_pass();
        }
        b.pass().clear = Some(color);
    });
}

/// Moves everything drawn until `pop_shift` right by `dx` points.
pub fn push_shift(dx: f32) {
    with(|b| b.shift.push(dx));
}

pub fn pop_shift() {
    with(|b| {
        b.shift.pop();
    });
}

/// The buffers the frame goes into, grown as it needs.
pub(super) struct Buffers {
    vertex: Option<wgpu::Buffer>,
    index: Option<wgpu::Buffer>,
    uniform: Option<wgpu::Buffer>,
}

impl Buffers {
    pub fn new() -> Buffers {
        Buffers { vertex: None, index: None, uniform: None }
    }
}

fn upload(slot: &mut Option<wgpu::Buffer>, bytes: &[u8], usage: wgpu::BufferUsages, label: &str) -> wgpu::Buffer {
    let k = kit();
    let size = (bytes.len().max(16) as u64).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
    if slot.as_ref().is_none_or(|b| b.size() < size) {
        *slot = Some(k.gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.next_power_of_two(), usage: usage | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
    }
    let buf = slot.clone().unwrap();
    let mut padded;
    let bytes = if bytes.len() as u64 % wgpu::COPY_BUFFER_ALIGNMENT != 0 {
        padded = bytes.to_vec();
        padded.resize(size as usize, 0);
        &padded[..]
    } else {
        bytes
    };
    if !bytes.is_empty() {
        k.gpu.queue.write_buffer(&buf, 0, bytes);
    }
    buf
}

/// Sends what was drawn since the last flush to the card: the screen's part into `frame` (the
/// multisampled `msaa`, resolved into it).
pub(super) fn flush(bufs: &mut Buffers, msaa: &wgpu::TextureView, frame: &wgpu::TextureView) {
    let k: &Kit = kit();
    let (vertices, indices, calls, passes, uniforms, started) = with(|b| {
        let started = b.screen_started;
        if b.passes.iter().any(|p| p.target.is_none()) {
            b.screen_started = true;
        }
        b.material = None;
        (std::mem::take(&mut b.vertices), std::mem::take(&mut b.indices), std::mem::take(&mut b.calls), std::mem::take(&mut b.passes), std::mem::take(&mut b.uniforms), started)
    });
    if passes.is_empty() {
        return;
    }
    let vbuf = upload(&mut bufs.vertex, bytemuck::cast_slice(&vertices), wgpu::BufferUsages::VERTEX, "vertices");
    let ibuf = upload(&mut bufs.index, bytemuck::cast_slice(&indices), wgpu::BufferUsages::INDEX, "indices");
    let mut ubytes = uniforms;
    ubytes.resize(ubytes.len().next_multiple_of(UNIFORM_ALIGN) + UNIFORM_ALIGN, 0);
    let ubuf = upload(&mut bufs.uniform, &ubytes, wgpu::BufferUsages::UNIFORM, "uniforms");
    let globals = k.gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("globals"),
        layout: &k.globals_layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &ubuf, offset: 0, size: wgpu::BufferSize::new(64) }) }],
    });
    // The material bind groups, made before the passes borrow them.
    let groups: Vec<Option<wgpu::BindGroup>> = calls.iter().map(|c| c.material.bind_group(&ubuf, &c.extra)).collect();
    let mut enc = k.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
    let mut screen_seen = started;
    for pass in &passes {
        let (view, resolve, samples) = match &pass.target {
            Some(t) => (t.texture.view(), None, 1),
            None => (msaa, Some(frame), super::SAMPLES),
        };
        let load = match pass.clear {
            Some(c) => wgpu::LoadOp::Clear(wgpu::Color { r: c.r as f64, g: c.g as f64, b: c.b as f64, a: c.a as f64 }),
            None if pass.target.is_none() && !screen_seen => wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            None => wgpu::LoadOp::Load,
        };
        if pass.target.is_none() {
            screen_seen = true;
        }
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, resolve_target: resolve, ops: wgpu::Operations { load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        if pass.calls.is_empty() {
            continue;
        }
        rp.set_vertex_buffer(0, vbuf.slice(..));
        rp.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
        rp.set_bind_group(0, &globals, &[pass.globals]);
        for i in pass.calls.clone() {
            let c = &calls[i];
            rp.set_pipeline(&c.material.pipeline(samples));
            rp.set_bind_group(1, &c.texture.bind_group(), &[]);
            if let Some(g) = &groups[i] {
                rp.set_bind_group(2, g, &[c.uniforms.unwrap_or(0)]);
            }
            rp.draw_indexed(c.first..c.first + c.count, 0, 0..1);
        }
    }
    k.gpu.queue.submit([enc.finish()]);
}

/// A new frame: nothing of it is on the frame texture yet.
pub(super) fn begin_frame(screen: (f32, f32)) {
    with(|b| {
        b.screen = screen;
        b.screen_started = false;
        b.target = None;
        b.shift.clear();
    });
}
