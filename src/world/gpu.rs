//! The graphics card, for the computations that are the same on every cell (the pressure of the
//! liquids, see `mgpcg_gpu.rs`): one wgpu device for the whole game, the one the window draws
//! with (see `gfx`). Off unless asked: on an integrated card the trip costs as much as the CPU's own
//! solve, so the card is for a dedicated one. `AWORLD_GPU=auto` sends it the big systems;
//! `AWORLD_GPU=1` sends it even the small ones the CPU would solve faster (for tests).
//!
//! In the browser the card draws but does not compute: reading results back means waiting for
//! the card, which a page cannot do (the liquids stay on the CPU there).

use std::sync::OnceLock;

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// The card's name, for the log.
    pub name: String,
    /// Whether the card can stamp the time between its kernels (asked for with
    /// `AWORLD_DEBUG_CARD`, where the card offers it).
    pub timestamps: bool,
}

/// How the card is wanted: off, as the size of the work decides, or for everything.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Use {
    Off,
    Auto,
    Always,
}

#[cfg(not(target_arch = "wasm32"))]
pub fn wanted() -> Use {
    match std::env::var("AWORLD_GPU").as_deref() {
        Ok("1") => Use::Always,
        Ok("auto") => Use::Auto,
        _ => Use::Off,
    }
}

#[cfg(target_arch = "wasm32")]
pub fn wanted() -> Use {
    Use::Off
}

/// The limits asked of the card: the defaults (the browser's own where WebGPU is missing), but
/// as many storage buffers a kernel as the card has (the liquids bind some thirty grids at
/// once).
fn limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
    let card = adapter.limits();
    let base = if adapter.get_info().backend == wgpu::Backend::Gl { wgpu::Limits::downlevel_webgl2_defaults() } else { wgpu::Limits::default() };
    let mut l = base.using_resolution(card.clone());
    l.max_storage_buffers_per_shader_stage = card.max_storage_buffers_per_shader_stage.min(64);
    l.max_storage_buffer_binding_size = card.max_storage_buffer_binding_size.min(1 << 30);
    l
}

static CARD: OnceLock<Option<Gpu>> = OnceLock::new();

/// Opens the device on `adapter`, with what the liquids need of it.
pub async fn open(adapter: &wgpu::Adapter) -> Option<Gpu> {
    let info = adapter.get_info();
    let stamps = wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
    let timestamps = cfg!(not(target_arch = "wasm32")) && std::env::var_os("AWORLD_DEBUG_CARD").is_some() && adapter.features().contains(stamps);
    let (device, queue) = adapter
        .request_device(
            &wgpu::DeviceDescriptor { label: Some("aworld"), required_features: if timestamps { stamps } else { wgpu::Features::empty() }, required_limits: limits(adapter), memory_hints: wgpu::MemoryHints::Performance },
            None,
        )
        .await
        .ok()?;
    // A lost device (the driver gave up) must not take the game with it.
    device.on_uncaptured_error(Box::new(|e| eprintln!("aworld: GPU: {e}")));
    let name = format!("{} ({:?})", info.name, info.backend);
    Some(Gpu { device, queue, name, timestamps })
}

/// The window's device, opened by `gfx` with the window: the liquids then share it. Done once,
/// before anything asks for the card.
pub fn install(gpu: Gpu) -> &'static Gpu {
    if CARD.set(Some(gpu)).is_err() {
        panic!("the card was opened twice");
    }
    CARD.get().unwrap().as_ref().unwrap()
}

/// The device, whatever it is used for: the window's, or (without a window, in the tests) one
/// opened on the first card found.
#[cfg(not(target_arch = "wasm32"))]
pub fn card() -> Option<&'static Gpu> {
    CARD.get_or_init(|| {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor { backends: wgpu::Backends::PRIMARY, ..Default::default() });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }))?;
        pollster::block_on(open(&adapter))
    })
    .as_ref()
}

/// The card for the liquids; `None` where there is none, or where it is not asked for.
#[cfg(not(target_arch = "wasm32"))]
pub fn gpu() -> Option<&'static Gpu> {
    static SAID: OnceLock<()> = OnceLock::new();
    if wanted() == Use::Off {
        return None;
    }
    let gpu = card()?;
    SAID.get_or_init(|| {
        if std::env::var_os("AWORLD_PROFILE").is_some() || wanted() == Use::Always {
            eprintln!("aworld: GPU: {}", gpu.name);
        }
    });
    Some(gpu)
}

#[cfg(target_arch = "wasm32")]
pub fn gpu() -> Option<&'static Gpu> {
    None
}
