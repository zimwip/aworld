//! Everything the game draws with, reads the keyboard and the mouse with, and opens its window
//! with, in one place: winit for the window and its events, wgpu for every pixel. The card is
//! the one the liquids compute on (`world::gpu`), opened here with the window.
//!
//! The frame is drawn the way macroquad drew it, which the game was written with: immediate
//! calls (`draw_circle`, `draw_text`...) gathered into one batch, sent at `next_frame`. The game
//! itself is an `async` function that awaits `next_frame` once a frame; `run` drives it from the
//! window's event loop.

mod batch;
mod input;
mod material;
mod math;
mod shapes;
mod text;
mod texture;

pub use batch::{clear_background, gl_use_default_material, gl_use_material, pop_shift, push_shift, set_target, Vertex};
pub use input::{get_keys_pressed, is_key_down, is_key_pressed, is_mouse_button_down, is_mouse_button_pressed, is_mouse_button_released, mouse_position, mouse_wheel, KeyCode, MouseButton};
pub use material::{load_material, Blend, Material, MaterialParams, UniformDesc, UniformType};
pub use math::{vec2, vec4, Color, Rect, Vec2, BLACK, BLANK, ORANGE, WHITE};
pub use shapes::{draw_arc, draw_circle, draw_circle_lines, draw_ellipse, draw_line, draw_mesh, draw_poly, draw_rectangle, draw_rectangle_ex, draw_rectangle_lines, draw_triangle, DrawRectangleParams, Mesh};
pub use text::{draw_text, draw_text_ex, load_ttf_font_from_bytes, measure_text, Font, TextParams};
pub use texture::{draw_texture_ex, render_target, render_target_scaled, DrawTextureParams, FilterMode, Image, RenderTarget, Texture2D};

use crate::world::gpu::Gpu;
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll, Waker};
use web_time::Instant;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Fullscreen, Window, WindowId};

/// Samples a pixel on the screen (smooth edges on every shape).
const SAMPLES: u32 = 4;

/// What every draw needs of the card: the device, the layouts all shaders share, the samplers,
/// a white texture for plain colours.
pub(crate) struct Kit {
    gpu: &'static Gpu,
    globals_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    white: Texture2D,
}

// The kit lives in the main thread only (the window's), where every draw happens.
thread_local! {
    static KIT: OnceLock<&'static Kit> = const { OnceLock::new() };
}

fn kit() -> &'static Kit {
    KIT.with(|k| *k.get().expect("drawing before the window is open"))
}

fn make_kit(gpu: &'static Gpu) {
    let d = &gpu.device;
    let globals_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("globals"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(64) },
            count: None,
        }],
    });
    let texture_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("texture"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                count: None,
            },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
        ],
    });
    let sampler = |f: wgpu::FilterMode| {
        d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: f,
            min_filter: f,
            ..Default::default()
        })
    };
    let (linear, nearest) = (sampler(wgpu::FilterMode::Linear), sampler(wgpu::FilterMode::Nearest));
    let white = Texture2D::white(gpu);
    let kit: &'static Kit = Box::leak(Box::new(Kit { gpu, globals_layout, texture_layout, linear, nearest, white }));
    KIT.with(|k| k.set(kit).ok());
}

/// The window's settings.
pub struct Conf {
    pub window_title: String,
    /// In points.
    pub window_width: i32,
    pub window_height: i32,
    pub window_resizable: bool,
}

impl Default for Conf {
    fn default() -> Self {
        Conf { window_title: "aworld".into(), window_width: 800, window_height: 600, window_resizable: true }
    }
}

/// The window and what it draws into.
struct Screen {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// The view the surface is drawn through (without sRGB, see `frame`).
    surface_view_format: wgpu::TextureFormat,
    /// The frame is drawn here, multisampled, resolved into `frame`, which is then copied onto
    /// the window (and is what a screenshot reads).
    msaa: wgpu::TextureView,
    frame: wgpu::Texture,
    frame_view: wgpu::TextureView,
    blit: wgpu::RenderPipeline,
    blit_layout: wgpu::BindGroupLayout,
    buffers: batch::Buffers,
}

thread_local! {
    static SCREEN: RefCell<Option<Screen>> = const { RefCell::new(None) };
    /// The window, while its card is being opened (in the browser that takes a turn of the page's
    /// event loop): `SCREEN` once it is.
    static OPENING: Cell<bool> = const { Cell::new(false) };
    static CLOCK: Cell<(f64, f32)> = const { Cell::new((0.0, 1.0 / 60.0)) };
    static FRAME_DONE: Cell<bool> = const { Cell::new(false) };
    static WANTS: RefCell<Wants> = RefCell::new(Wants::default());
}

/// What the game asked of the window during the frame, done once the frame is drawn.
#[derive(Default)]
struct Wants {
    fullscreen: Option<bool>,
    cursor: Option<bool>,
    quit: bool,
}

fn with_screen<R>(f: impl FnOnce(&mut Screen) -> R) -> R {
    SCREEN.with(|s| f(s.borrow_mut().as_mut().expect("no window")))
}

impl Screen {
    async fn new(window: Arc<Window>) -> Screen {
        // In the browser, WebGPU where there is one, WebGL 2 otherwise.
        let backends = if cfg!(target_arch = "wasm32") { wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL } else { wgpu::Backends::PRIMARY };
        let instance = wgpu::util::new_instance_with_webgpu_detection(&wgpu::InstanceDescriptor { backends, ..Default::default() }).await;
        let surface = instance.create_surface(window.clone()).expect("surface");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, compatible_surface: Some(&surface), force_fallback_adapter: false })
            .await
            .expect("no graphics card that can draw in this window");
        let gpu = crate::world::gpu::install(crate::world::gpu::open(&adapter).await.expect("the graphics card would not open"));
        make_kit(gpu);
        let caps = surface.get_capabilities(&adapter);
        // The colours are blended as they are, without sRGB conversion: the surface is viewed
        // without it.
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let view_format = format.remove_srgb_suffix();
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: if view_format != format { vec![view_format] } else { vec![] },
        };
        surface.configure(&gpu.device, &config);
        let blit_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                count: None,
            }],
        });
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("blit"), source: wgpu::ShaderSource::Wgsl(BLIT.into()) });
        let layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("blit"), bind_group_layouts: &[&blit_layout], push_constant_ranges: &[] });
        let blit = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: view_format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview: None,
            cache: None,
        });
        let (msaa, frame, frame_view) = Screen::targets(gpu, config.width, config.height);
        Screen { window, surface, config, surface_view_format: view_format, msaa, frame, frame_view, blit, blit_layout, buffers: batch::Buffers::new() }
    }

    fn targets(gpu: &Gpu, w: u32, h: u32) -> (wgpu::TextureView, wgpu::Texture, wgpu::TextureView) {
        let make = |samples: u32, usage: wgpu::TextureUsages| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("frame"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format: texture::FORMAT,
                usage,
                view_formats: &[],
            })
        };
        let msaa = make(SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default());
        let frame = make(1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
        let frame_view = frame.create_view(&Default::default());
        (msaa, frame, frame_view)
    }

    fn resize(&mut self, w: u32, h: u32) {
        let gpu = kit().gpu;
        self.config.width = w.max(1);
        self.config.height = h.max(1);
        self.surface.configure(&gpu.device, &self.config);
        (self.msaa, self.frame, self.frame_view) = Screen::targets(gpu, self.config.width, self.config.height);
    }

    fn flush(&mut self) {
        batch::flush(&mut self.buffers, &self.msaa, &self.frame_view);
    }

    /// The frame onto the window.
    fn present(&mut self) {
        self.flush();
        let gpu = kit().gpu;
        let out = match self.surface.get_current_texture() {
            Ok(t) => t,
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                let size = self.window.inner_size();
                self.resize(size.width, size.height);
                return;
            }
            Err(e) => {
                eprintln!("aworld: window: {e}");
                return;
            }
        };
        let view = out.texture.create_view(&wgpu::TextureViewDescriptor { format: Some(self.surface_view_format), ..Default::default() });
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit"),
            layout: &self.blit_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.frame_view) }],
        });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                ..Default::default()
            });
            rp.set_pipeline(&self.blit);
            rp.set_bind_group(0, &group, &[]);
            rp.draw(0..3, 0..1);
        }
        gpu.queue.submit([enc.finish()]);
        self.window.pre_present_notify();
        out.present();
    }
}

/// Copies the frame texture onto the window, pixel for pixel.
const BLIT: &str = "
@group(0) @binding(0) var frame: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(textureLoad(frame, vec2<i32>(p.xy), 0).rgb, 1.0);
}
";

/// The window's points to pixels.
pub fn dpi_scale() -> f32 {
    SCREEN.with(|s| s.borrow().as_ref().map_or(1.0, |s| s.window.scale_factor() as f32))
}

/// The screen's size, in points.
pub fn screen_width() -> f32 {
    batch::with(|b| b.screen.0)
}

pub fn screen_height() -> f32 {
    batch::with(|b| b.screen.1)
}

/// Seconds the last frame took.
pub fn get_frame_time() -> f32 {
    CLOCK.with(|c| c.get().1)
}

/// Seconds since the game started.
pub fn get_time() -> f64 {
    CLOCK.with(|c| c.get().0)
}

pub fn set_fullscreen(on: bool) {
    WANTS.with(|w| w.borrow_mut().fullscreen = Some(on));
}

pub fn show_mouse(shown: bool) {
    WANTS.with(|w| w.borrow_mut().cursor = Some(shown));
}

/// Closes the window (in the browser, the page goes back to the game).
#[cfg(target_arch = "wasm32")]
pub fn request_quit() {
    WANTS.with(|w| w.borrow_mut().quit = true);
}

/// Writes what was drawn so far this frame to a PNG file.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_screenshot(path: &str) {
    with_screen(|s| {
        s.flush();
        let gpu = kit().gpu;
        let (w, h) = (s.config.width, s.config.height);
        let row = (4 * w).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("screenshot"), size: (row * h) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            s.frame.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        gpu.queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        gpu.device.poll(wgpu::Maintain::Wait);
        let data = buf.slice(..).get_mapped_range();
        let mut px = Vec::with_capacity((4 * w * h) as usize);
        for y in 0..h as usize {
            px.extend(data[y * row as usize..][..4 * w as usize].chunks(4).flat_map(|p| [p[0], p[1], p[2], 255]));
        }
        let Ok(file) = std::fs::File::create(path) else {
            eprintln!("aworld: cannot write {path}");
            return;
        };
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        if let Err(e) = enc.write_header().and_then(|mut wr| wr.write_image_data(&px)) {
            eprintln!("aworld: {path}: {e}");
        }
    });
}

/// Ends the frame: it is drawn on the window, and the game goes on with the next.
pub fn next_frame() -> NextFrame {
    NextFrame(false)
}

pub struct NextFrame(bool);

impl Future for NextFrame {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, _: &mut Context) -> Poll<()> {
        if self.0 {
            return Poll::Ready(());
        }
        self.0 = true;
        FRAME_DONE.with(|f| f.set(true));
        Poll::Pending
    }
}

struct App {
    conf: Conf,
    game: Option<Pin<Box<dyn Future<Output = ()>>>>,
    start: Instant,
    last: Instant,
}

impl App {
    fn screen_size(window: &Window) -> (f32, f32) {
        let s = window.inner_size();
        let k = window.scale_factor() as f32;
        (s.width.max(1) as f32 / k, s.height.max(1) as f32 / k)
    }

    /// One frame of the game: run it until it awaits `next_frame`, then show what it drew.
    fn frame(&mut self, el: &ActiveEventLoop) {
        let Some(game) = self.game.as_mut() else { return };
        if SCREEN.with(|s| s.borrow().is_none()) {
            return;
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32();
        self.last = now;
        CLOCK.with(|c| c.set((now.duration_since(self.start).as_secs_f64(), dt)));
        let size = with_screen(|s| App::screen_size(&s.window));
        batch::begin_frame(size);
        let mut cx = Context::from_waker(Waker::noop());
        FRAME_DONE.with(|f| f.set(false));
        if game.as_mut().poll(&mut cx).is_ready() {
            self.game = None;
            el.exit();
            return;
        }
        with_screen(|s| s.present());
        input::end_frame();
        let wants = WANTS.with(|w| std::mem::take(&mut *w.borrow_mut()));
        with_screen(|s| {
            if let Some(on) = wants.fullscreen {
                s.window.set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
            }
            if let Some(shown) = wants.cursor {
                s.window.set_cursor_visible(shown);
            }
        });
        if wants.quit {
            el.exit();
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if OPENING.with(|o| o.replace(true)) {
            return;
        }
        let attrs = Window::default_attributes().with_title(self.conf.window_title.clone()).with_resizable(self.conf.window_resizable);
        #[cfg(not(target_arch = "wasm32"))]
        let attrs = attrs.with_inner_size(winit::dpi::LogicalSize::new(self.conf.window_width, self.conf.window_height));
        #[cfg(target_arch = "wasm32")]
        let attrs = {
            use wasm_bindgen::JsCast;
            use winit::platform::web::WindowAttributesExtWebSys;
            // The page's canvas (see web/index.html), sized by the page's style.
            let canvas = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id("glcanvas")).and_then(|c| c.dyn_into::<web_sys::HtmlCanvasElement>().ok());
            attrs.with_canvas(canvas).with_prevent_default(true).with_focusable(true)
        };
        let window = Arc::new(el.create_window(attrs).expect("window"));
        let opened = Screen::new(window.clone());
        let done = move |screen: Screen| {
            SCREEN.with(|s| *s.borrow_mut() = Some(screen));
            window.request_redraw();
        };
        #[cfg(not(target_arch = "wasm32"))]
        done(pollster::block_on(opened));
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(async move { done(opened.await) });
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match &event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => SCREEN.with(|s| {
                if let Some(s) = s.borrow_mut().as_mut() {
                    s.resize(size.width, size.height);
                }
            }),
            WindowEvent::RedrawRequested => {
                self.frame(el);
                if self.game.is_some() {
                    SCREEN.with(|s| s.borrow().as_ref().map(|s| s.window.request_redraw()));
                }
            }
            _ => input::event(&event, dpi_scale()),
        }
    }
}

/// Opens the window and runs `game` in it until it returns or the window closes.
pub fn run(conf: Conf, game: impl Future<Output = ()> + 'static) {
    let event_loop = EventLoop::new().expect("event loop");
    let now = Instant::now();
    let app = App { conf, game: Some(Box::pin(game)), start: now, last: now };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut app = app;
        event_loop.run_app(&mut app).expect("event loop");
    }
    // The page's event loop runs the game: this returns at once.
    #[cfg(target_arch = "wasm32")]
    winit::platform::web::EventLoopExtWebSys::spawn_app(event_loop, app);
}
