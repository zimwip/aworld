//! Materials: a fragment shader in WGSL with its uniforms, its textures and how it blends.
//!
//! Every shader is the material's source after a common prelude (`PRELUDE`), which holds the
//! vertex shader, the projection (group 0) and the texture being drawn (`Texture`, group 1, read
//! with `tex(uv)`). A material with uniforms or textures of its own declares them in group 2:
//! binding 0 a `Params` struct with the uniforms in the order given, then for each texture a
//! `texture_2d<f32>` and its `sampler` (bindings 1 and 2 for the first, 3 and 4 for the next...).

use super::batch::Vertex;
use super::kit;
use super::math::{Vec2, Vec4};
use super::texture::{Texture2D, FORMAT};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

const PRELUDE: &str = "
struct Globals { proj: mat4x4<f32> }
@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var Texture: texture_2d<f32>;
@group(1) @binding(1) var Texture_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_main(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> VertexOutput {
    var out: VertexOutput;
    out.position = globals.proj * vec4<f32>(position, 1.0);
    out.uv = uv;
    out.color = color;
    return out;
}

// No mipmaps anywhere: reading level 0 works in any branch.
fn tex(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(Texture, Texture_sampler, uv, 0.0);
}
";

const DEFAULT: &str = "
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return tex(in.uv) * in.color;
}
";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UniformType {
    Float1,
    Float2,
    Float4,
}

impl UniformType {
    /// Size and alignment in a WGSL uniform struct.
    fn layout(self) -> (usize, usize) {
        match self {
            UniformType::Float1 => (4, 4),
            UniformType::Float2 => (8, 8),
            UniformType::Float4 => (16, 16),
        }
    }
}

#[derive(Clone, Debug)]
pub struct UniformDesc {
    pub name: String,
    pub ty: UniformType,
}

impl UniformDesc {
    pub fn new(name: &str, ty: UniformType) -> UniformDesc {
        UniformDesc { name: name.to_string(), ty }
    }
}

/// How what a material draws goes over what is already there.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Blend {
    /// Straight colours, the usual: colour × alpha + what was there × (1 − alpha).
    #[default]
    Alpha,
    /// Colours already multiplied by their alpha.
    Premultiplied,
    /// Added to what is there (light).
    Additive,
    /// Written as they are.
    Replace,
}

impl Blend {
    fn state(self) -> Option<wgpu::BlendState> {
        use wgpu::{BlendComponent as C, BlendFactor as F, BlendOperation as O};
        let over = C { src_factor: F::One, dst_factor: F::OneMinusSrcAlpha, operation: O::Add };
        match self {
            Blend::Alpha => Some(wgpu::BlendState { color: C { src_factor: F::SrcAlpha, dst_factor: F::OneMinusSrcAlpha, operation: O::Add }, alpha: over }),
            Blend::Premultiplied => Some(wgpu::BlendState { color: over, alpha: over }),
            Blend::Additive => {
                let add = C { src_factor: F::One, dst_factor: F::One, operation: O::Add };
                Some(wgpu::BlendState { color: add, alpha: add })
            }
            Blend::Replace => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct MaterialParams {
    pub uniforms: Vec<UniformDesc>,
    pub textures: Vec<String>,
    pub blend: Blend,
}

struct Inner {
    module: wgpu::ShaderModule,
    blend: Blend,
    /// Group 2, when the material has uniforms or textures.
    layout: Option<wgpu::BindGroupLayout>,
    pipeline_layout: wgpu::PipelineLayout,
    pipelines: RefCell<HashMap<u32, wgpu::RenderPipeline>>,
    /// Each uniform's name, type and place in `values`.
    uniforms: Vec<(String, UniformType, usize)>,
    values: RefCell<Vec<u8>>,
    texture_names: Vec<String>,
    textures: RefCell<Vec<Option<Texture2D>>>,
    /// Changes whenever a uniform or a texture does.
    stamp: Cell<u64>,
}

/// A material. Clones share it, its uniforms included.
#[derive(Clone)]
pub struct Material(Rc<Inner>);

/// A value a uniform can take.
pub trait UniformValue {
    fn bytes(&self) -> Vec<u8>;
}

impl UniformValue for f32 {
    fn bytes(&self) -> Vec<u8> {
        self.to_ne_bytes().to_vec()
    }
}

impl UniformValue for Vec2 {
    fn bytes(&self) -> Vec<u8> {
        bytemuck::bytes_of(&self.to_array()).to_vec()
    }
}

impl UniformValue for Vec4 {
    fn bytes(&self) -> Vec<u8> {
        bytemuck::bytes_of(&self.to_array()).to_vec()
    }
}

pub fn load_material(wgsl: &str, params: MaterialParams) -> Result<Material, String> {
    let k = kit();
    let device = &k.gpu.device;
    let source = format!("{PRELUDE}\n{wgsl}");
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("material"), source: wgpu::ShaderSource::Wgsl(source.into()) });
    errors(device)?;
    let mut uniforms = vec![];
    let mut at = 0usize;
    for u in &params.uniforms {
        let (size, align) = u.ty.layout();
        at = at.next_multiple_of(align);
        uniforms.push((u.name.clone(), u.ty, at));
        at += size;
    }
    let size = at.next_multiple_of(16).max(16);
    let layout = (!params.uniforms.is_empty() || !params.textures.is_empty()).then(|| {
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(size as u64) },
            count: None,
        }];
        for i in 0..params.textures.len() as u32 {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 1 + 2 * i,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                count: None,
            });
            entries.push(wgpu::BindGroupLayoutEntry { binding: 2 + 2 * i, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None });
        }
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("material"), entries: &entries })
    });
    let mut groups = vec![&k.globals_layout, &k.texture_layout];
    if let Some(l) = &layout {
        groups.push(l);
    }
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("material"), bind_group_layouts: &groups, push_constant_ranges: &[] });
    let material = Material(Rc::new(Inner {
        module,
        blend: params.blend,
        layout,
        pipeline_layout,
        pipelines: RefCell::new(HashMap::new()),
        uniforms,
        values: RefCell::new(vec![0; size]),
        texture_names: params.textures.clone(),
        textures: RefCell::new(vec![None; params.textures.len()]),
        stamp: Cell::new(0),
    }));
    // Made now for the screen, so that a mistake shows at once and not mid-game.
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    material.pipeline(super::SAMPLES);
    material.pipeline(1);
    errors(device)?;
    Ok(material)
}

/// The mistakes in what was made since the error scope was opened. (In the browser the answer
/// comes later, and the page cannot wait for it: they go to the console.)
fn errors(device: &wgpu::Device) -> Result<(), String> {
    let scope = device.pop_error_scope();
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(e) = pollster::block_on(scope) {
        return Err(e.to_string());
    }
    #[cfg(target_arch = "wasm32")]
    drop(scope);
    Ok(())
}

thread_local! {
    static DEFAULT_MATERIAL: Material = load_material(DEFAULT, MaterialParams::default()).expect("default shader");
}

pub(super) fn default_material() -> Material {
    DEFAULT_MATERIAL.with(|m| m.clone())
}

impl Material {
    pub fn set_uniform<T: UniformValue>(&self, name: &str, value: T) {
        let Some(&(_, _, at)) = self.0.uniforms.iter().find(|u| u.0 == name) else {
            panic!("no uniform {name}");
        };
        let b = value.bytes();
        self.0.values.borrow_mut()[at..at + b.len()].copy_from_slice(&b);
        self.0.stamp.set(self.0.stamp.get() + 1);
    }

    pub fn set_texture(&self, name: &str, texture: Texture2D) {
        let Some(i) = self.0.texture_names.iter().position(|n| n == name) else {
            panic!("no texture {name}");
        };
        self.0.textures.borrow_mut()[i] = Some(texture);
        self.0.stamp.set(self.0.stamp.get() + 1);
    }

    pub(super) fn same(&self, other: &Material) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    pub(super) fn stamp(&self) -> u64 {
        self.0.stamp.get()
    }

    pub(super) fn uniform_bytes(&self) -> Option<Vec<u8>> {
        self.0.layout.as_ref().map(|_| self.0.values.borrow().clone())
    }

    pub(super) fn textures(&self) -> Vec<Texture2D> {
        let white = &kit().white;
        self.0.textures.borrow().iter().map(|t| t.clone().unwrap_or_else(|| white.clone())).collect()
    }

    /// Group 2 for a draw call: the uniform buffer of the frame and the call's textures.
    pub(super) fn bind_group(&self, uniforms: &wgpu::Buffer, textures: &[Texture2D]) -> Option<wgpu::BindGroup> {
        let layout = self.0.layout.as_ref()?;
        let size = self.0.values.borrow().len() as u64;
        let mut entries = vec![wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: uniforms, offset: 0, size: wgpu::BufferSize::new(size) }) }];
        for (i, t) in textures.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry { binding: 1 + 2 * i as u32, resource: wgpu::BindingResource::TextureView(t.view()) });
            entries.push(wgpu::BindGroupEntry { binding: 2 + 2 * i as u32, resource: wgpu::BindingResource::Sampler(t.sampler()) });
        }
        Some(kit().gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("material"), layout, entries: &entries }))
    }

    /// The pipeline for a target with `samples` samples a pixel.
    pub(super) fn pipeline(&self, samples: u32) -> wgpu::RenderPipeline {
        self.0
            .pipelines
            .borrow_mut()
            .entry(samples)
            .or_insert_with(|| {
                kit().gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("material"),
                    layout: Some(&self.0.pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &self.0.module,
                        entry_point: Some("vs_main"),
                        compilation_options: Default::default(),
                        buffers: &[wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<Vertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Unorm8x4],
                        }],
                    },
                    primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
                    fragment: Some(wgpu::FragmentState {
                        module: &self.0.module,
                        entry_point: Some("fs_main"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState { format: FORMAT, blend: self.0.blend.state(), write_mask: wgpu::ColorWrites::ALL })],
                    }),
                    multiview: None,
                    cache: None,
                })
            })
            .clone()
    }
}
