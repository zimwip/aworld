//! The graphics card, for the computations that are the same on every cell (the pressure of the
//! liquids, see `mgpcg_gpu.rs`): one wgpu device for the whole game, opened the first time it is
//! asked for. `AWORLD_GPU=0` leaves the card alone (everything on the CPU, as before);
//! `AWORLD_GPU=1` sends the card even the small systems the CPU would solve faster (for tests).
//!
//! The web build has no card yet: it will, through WebGPU, once the game draws with wgpu too.

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::sync::OnceLock;

    pub struct Gpu {
        pub device: wgpu::Device,
        pub queue: wgpu::Queue,
        /// The card's name, for the log.
        pub name: String,
    }

    /// How the card is wanted: off, as the size of the work decides, or for everything.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum Use {
        Off,
        Auto,
        Always,
    }

    pub fn wanted() -> Use {
        match std::env::var("AWORLD_GPU").as_deref() {
            Ok("0") => Use::Off,
            Ok("1") => Use::Always,
            _ => Use::Auto,
        }
    }

    /// The card, opened once; `None` where there is none (or `AWORLD_GPU=0`).
    pub fn gpu() -> Option<&'static Gpu> {
        static GPU: OnceLock<Option<Gpu>> = OnceLock::new();
        GPU.get_or_init(|| {
            if wanted() == Use::Off {
                return None;
            }
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor { backends: wgpu::Backends::PRIMARY, ..Default::default() });
            let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }))?;
            let info = adapter.get_info();
            let (device, queue) = pollster::block_on(adapter.request_device(
                &wgpu::DeviceDescriptor { label: Some("aworld"), required_features: wgpu::Features::empty(), required_limits: wgpu::Limits::default().using_resolution(adapter.limits()), memory_hints: wgpu::MemoryHints::Performance },
                None,
            ))
            .ok()?;
            // A lost device (the driver gave up) must not take the game with it.
            device.on_uncaptured_error(Box::new(|e| eprintln!("aworld: GPU: {e}")));
            let name = format!("{} ({:?})", info.name, info.backend);
            if std::env::var_os("AWORLD_PROFILE").is_some() || wanted() == Use::Always {
                eprintln!("aworld: GPU: {name}");
            }
            Some(Gpu { device, queue, name })
        })
        .as_ref()
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::*;

#[cfg(target_arch = "wasm32")]
mod web {
    pub struct Gpu;

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum Use {
        Off,
        Auto,
        Always,
    }

    pub fn wanted() -> Use {
        Use::Off
    }

    pub fn gpu() -> Option<&'static Gpu> {
        None
    }
}
