//! Where the samples go: the sound card.
//!
//! One small mixer everywhere (`Mixer`): clips of mono samples at `RATE`, played looped or once,
//! each with its volume. It plays through cpal on Windows, macOS and the web (WASAPI, CoreAudio,
//! WebAudio: nothing to install). On Linux cpal would link ALSA at build time, so building would
//! need `libasound2-dev` and the game would not even start without `libasound.so.2`. Instead,
//! Linux loads ALSA when the game starts: it builds with nothing installed, and a machine without
//! ALSA or without a sound card just plays silently.
//!
//! The API: `Out::open`, `load` a clip, `play` it (looped or once) and `set_volume` on every
//! playback of a clip.

use super::RATE;
use std::sync::{Arc, Mutex};

/// One-shots playing at once; the oldest is dropped past this.
const MAX_VOICES: usize = 32;

struct Voice {
    clip: usize,
    pos: usize,
    looped: bool,
    volume: f32,
    /// Volume actually applied, ramped towards `volume` over a period: no clicks.
    gain: f32,
}

#[derive(Default)]
struct Mixer {
    clips: Vec<Arc<[f32]>>,
    voices: Vec<Voice>,
}

impl Mixer {
    fn fill(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        let n = out.len() as f32;
        let clips = &self.clips;
        self.voices.retain_mut(|v| {
            let s = &clips[v.clip];
            let step = (v.volume - v.gain) / n;
            for o in out.iter_mut() {
                if v.pos >= s.len() {
                    if !v.looped || s.is_empty() {
                        return false;
                    }
                    v.pos = 0;
                }
                v.gain += step;
                *o += s[v.pos] * v.gain;
                v.pos += 1;
            }
            v.gain = v.volume;
            true
        });
    }
}

#[derive(Clone)]
pub struct Out {
    mixer: Arc<Mutex<Mixer>>,
}

pub struct Clip(usize);

impl Out {
    pub fn open() -> Option<Out> {
        let mixer = Arc::new(Mutex::new(Mixer::default()));
        device::start(mixer.clone())?;
        Some(Out { mixer })
    }

    pub async fn load(&self, samples: &[f32]) -> Option<Clip> {
        let mut m = self.mixer.lock().ok()?;
        m.clips.push(samples.into());
        Some(Clip(m.clips.len() - 1))
    }

    pub fn play(&self, c: &Clip, looped: bool, volume: f32) {
        let Ok(mut m) = self.mixer.lock() else { return };
        if m.voices.len() >= MAX_VOICES {
            if let Some(i) = m.voices.iter().position(|v| !v.looped) {
                m.voices.remove(i);
            }
        }
        m.voices.push(Voice { clip: c.0, pos: 0, looped, volume, gain: volume });
    }

    pub fn set_volume(&self, c: &Clip, volume: f32) {
        let Ok(mut m) = self.mixer.lock() else { return };
        for v in m.voices.iter_mut().filter(|v| v.clip == c.0) {
            v.volume = volume;
        }
    }
}

#[cfg(target_os = "linux")]
mod device {
    use super::{Mixer, RATE};
    use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
    use std::sync::{Arc, Mutex};

    /// Frames mixed and written at a time (about 23 ms).
    const PERIOD: usize = 512;

    pub fn start(mixer: Arc<Mutex<Mixer>>) -> Option<()> {
        let alsa = Alsa::load()?;
        let pcm = alsa.open()?;
        std::thread::Builder::new().name("sound".into()).spawn(move || run(alsa, pcm, mixer)).ok()?;
        Some(())
    }

    /// The sound thread: mix a period, write it, repeat (the write blocks, which paces it).
    fn run(alsa: Alsa, pcm: Pcm, mixer: Arc<Mutex<Mixer>>) {
        let mut mix = [0.0f32; PERIOD];
        let mut buf = [0i16; PERIOD];
        loop {
            match mixer.lock() {
                Ok(mut m) => m.fill(&mut mix),
                Err(_) => return,
            }
            for (b, s) in buf.iter_mut().zip(mix) {
                *b = (s.clamp(-1.0, 1.0) * 32000.0) as i16;
            }
            let mut done = 0;
            while done < PERIOD {
                let left = &buf[done..];
                let r = unsafe { (alsa.writei)(pcm.0, left.as_ptr() as *const c_void, left.len() as c_ulong) };
                if r >= 0 {
                    done += r as usize;
                } else if unsafe { (alsa.recover)(pcm.0, r as c_int, 1) } < 0 {
                    // The device is gone: carry on silently.
                    return;
                }
            }
        }
    }

    // ------------------------------------------------------------ ALSA, loaded at run time

    const RTLD_NOW: c_int = 2;
    const SND_PCM_STREAM_PLAYBACK: c_int = 0;
    const SND_PCM_FORMAT_S16_LE: c_int = 2;
    const SND_PCM_ACCESS_RW_INTERLEAVED: c_int = 3;

    extern "C" {
        fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    struct Pcm(*mut c_void);
    // Only the sound thread uses the device once it is open.
    unsafe impl Send for Pcm {}

    /// The five ALSA functions the game needs.
    struct Alsa {
        open: unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int, c_int) -> c_int,
        set_params: unsafe extern "C" fn(*mut c_void, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int,
        close: unsafe extern "C" fn(*mut c_void) -> c_int,
        writei: unsafe extern "C" fn(*mut c_void, *const c_void, c_ulong) -> c_long,
        recover: unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int,
    }

    impl Alsa {
        fn load() -> Option<Alsa> {
            unsafe {
                let lib = dlopen(c"libasound.so.2".as_ptr(), RTLD_NOW);
                if lib.is_null() {
                    return None;
                }
                let sym = |name: &std::ffi::CStr| {
                    let p = dlsym(lib, name.as_ptr());
                    (!p.is_null()).then_some(p)
                };
                // The library stays loaded for the rest of the game.
                Some(Alsa {
                    open: std::mem::transmute(sym(c"snd_pcm_open")?),
                    set_params: std::mem::transmute(sym(c"snd_pcm_set_params")?),
                    close: std::mem::transmute(sym(c"snd_pcm_close")?),
                    writei: std::mem::transmute(sym(c"snd_pcm_writei")?),
                    recover: std::mem::transmute(sym(c"snd_pcm_recover")?),
                })
            }
        }

        /// Opens the default output, mono 16-bit at `RATE` (ALSA resamples if needed).
        fn open(&self) -> Option<Pcm> {
            for dev in [c"default", c"pipewire"] {
                unsafe {
                    let mut pcm = std::ptr::null_mut();
                    if (self.open)(&mut pcm, dev.as_ptr(), SND_PCM_STREAM_PLAYBACK, 0) < 0 {
                        continue;
                    }
                    let latency_us = 80_000;
                    if (self.set_params)(pcm, SND_PCM_FORMAT_S16_LE, SND_PCM_ACCESS_RW_INTERLEAVED, 1, RATE, 1, latency_us) < 0 {
                        (self.close)(pcm);
                        continue;
                    }
                    return Some(Pcm(pcm));
                }
            }
            None
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod device {
    use super::{Mixer, RATE};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::{Arc, Mutex};

    /// The mixer through the default output, at the device's own rate (the mixer's samples are
    /// stretched to it) and on all its channels.
    pub fn start(mixer: Arc<Mutex<Mixer>>) -> Option<()> {
        let device = cpal::default_host().default_output_device()?;
        let config: cpal::StreamConfig = device.default_output_config().ok()?.into();
        let channels = config.channels as usize;
        let step = RATE as f64 / config.sample_rate.0 as f64;
        // The mixer's samples, a period at a time, and where the device is in them.
        let mut mixed = vec![0.0f32; 512];
        let mut pos = mixed.len() as f64;
        let stream = device
            .build_output_stream(
                &config,
                move |out: &mut [f32], _| {
                    for frame in out.chunks_mut(channels) {
                        if pos as usize >= mixed.len() {
                            pos -= mixed.len() as f64;
                            match mixer.lock() {
                                Ok(mut m) => m.fill(&mut mixed),
                                Err(_) => mixed.fill(0.0),
                            }
                        }
                        frame.fill(mixed[pos as usize].clamp(-1.0, 1.0));
                        pos += step;
                    }
                },
                |e| eprintln!("aworld: sound: {e}"),
                None,
            )
            .ok()?;
        stream.play().ok()?;
        // It plays for as long as the game runs.
        #[cfg(not(target_arch = "wasm32"))]
        std::mem::forget(stream);
        // A page may only make sound once the player has done something on it: each click or key
        // asks again (once it plays, asking does nothing).
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::{closure::Closure, JsCast};
            let again = Closure::<dyn FnMut()>::new(move || {
                let _ = stream.play();
            });
            let page = web_sys::window()?;
            for event in ["pointerdown", "keydown"] {
                let _ = page.add_event_listener_with_callback(event, again.as_ref().unchecked_ref());
            }
            again.forget();
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mixer(clips: &[&[f32]]) -> Mixer {
        Mixer { clips: clips.iter().map(|c| (*c).into()).collect(), voices: vec![] }
    }

    fn voice(clip: usize, looped: bool, volume: f32) -> Voice {
        Voice { clip, pos: 0, looped, volume, gain: volume }
    }

    #[test]
    fn one_shot_plays_once_then_leaves() {
        let mut m = mixer(&[&[0.5, 0.5, 0.5]]);
        m.voices.push(voice(0, false, 1.0));
        let mut out = [0.0; 5];
        m.fill(&mut out);
        assert_eq!(out, [0.5, 0.5, 0.5, 0.0, 0.0]);
        assert!(m.voices.is_empty());
    }

    #[test]
    fn loop_wraps_and_voices_add_up() {
        let mut m = mixer(&[&[1.0, -1.0], &[0.25]]);
        m.voices.push(voice(0, true, 0.5));
        m.voices.push(voice(1, false, 1.0));
        let mut out = [0.0; 4];
        m.fill(&mut out);
        assert_eq!(out, [0.75, -0.5, 0.5, -0.5]);
        assert_eq!(m.voices.len(), 1);
    }

    #[test]
    fn volume_change_is_ramped() {
        let mut m = mixer(&[&[1.0; 8]]);
        m.voices.push(voice(0, true, 0.0));
        m.voices[0].volume = 1.0;
        let mut out = [0.0; 4];
        m.fill(&mut out);
        assert!(out.windows(2).all(|w| w[1] > w[0]), "{out:?}");
        assert!((out[3] - 1.0).abs() < 1e-6);
    }
}
