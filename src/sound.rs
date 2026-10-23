//! Sound, all synthesised at start-up: no audio files.
//!
//! Loops (engine at four speeds, wind, water, fire) run all the time at volume 0 and are
//! faded by `Mix`; one-shots (`Ev`) are queued by the game and played with a small cooldown.
//! The synthesis is plain Rust returning samples, so it is unit-tested without a sound card.

#[cfg(feature = "sound")]
mod out;
#[cfg(feature = "sound")]
use out::{Clip, Out};

pub const RATE: u32 = 22050;
const TAU: f32 = std::f32::consts::TAU;

/// Engine loop pitches (Hz). Integer, so each one-second loop holds whole cycles.
const ENGINE_HZ: [f32; 4] = [44.0, 66.0, 96.0, 134.0];

/// What the world sounds like right now. Every value is 0..1.
#[derive(Clone, Copy, Default, Debug)]
pub struct Mix {
    pub wind: f32,
    pub water: f32,
    pub fire: f32,
    /// Engine speed of the loudest car, if one is driving.
    pub rpm: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ev {
    /// Explosion of the given radius (cells).
    Boom(f32),
    /// Something heavy lands; strength is the impact speed.
    Thud(f32),
    Clink,
    Crack,
    Spray,
    Snap,
    /// The fire engine's two-tone siren (pin-pon).
    Siren,
    /// A firework bursting in the sky.
    Pop,
    /// A whale breathing out at the surface.
    Blow,
    /// Thunder, 0..1: loud and cracking close by, a low roll far away.
    Thunder(f32),
}

impl Ev {
    fn slot(self) -> usize {
        match self {
            Ev::Boom(_) => 0,
            Ev::Thud(_) => 1,
            Ev::Clink => 2,
            Ev::Crack => 3,
            Ev::Spray => 4,
            Ev::Snap => 5,
            Ev::Siren => 6,
            Ev::Pop => 7,
            Ev::Blow => 8,
            Ev::Thunder(_) => 9,
        }
    }
}

// ---------------------------------------------------------------- synthesis

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

fn secs(t: f32) -> usize {
    (t * RATE as f32) as usize
}

fn normalize(v: &mut [f32], peak: f32) {
    let top = v.iter().fold(0.0f32, |a, s| a.max(s.abs())).max(1e-6);
    for s in v {
        *s *= peak / top;
    }
}

fn lowpass(v: &mut [f32], a: f32) {
    let mut y = 0.0;
    for s in v {
        y += a * (*s - y);
        *s = y;
    }
}

/// Makes `raw` (n + fade samples) into an n-sample loop whose end meets its start.
fn seamless(raw: &[f32], fade: usize) -> Vec<f32> {
    let n = raw.len() - fade;
    let mut out = raw[..n].to_vec();
    for i in 0..fade {
        let f = i as f32 / fade as f32;
        out[i] = raw[i] * f + raw[n + i] * (1.0 - f);
    }
    out
}

fn noise(rng: &mut Rng, n: usize) -> Vec<f32> {
    (0..n).map(|_| rng.next()).collect()
}

/// One second of engine at `hz` firings: odd-ish harmonics plus a uneven-firing wobble.
pub fn engine(hz: f32) -> Vec<f32> {
    let n = RATE as usize;
    let v: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let ph = hz * t;
            let wobble = 0.65 + 0.35 * (TAU * ph * 0.5).sin();
            let mut s = 0.0;
            for h in 1..=9 {
                s += (TAU * ph * h as f32).sin() / (h as f32).powf(1.1);
            }
            s * wobble
        })
        .collect();
    // Filter around the loop twice and keep the second lap, so the seam is smooth.
    let mut twice = v.clone();
    twice.extend_from_slice(&v);
    lowpass(&mut twice, 0.5);
    let mut v = twice.split_off(n);
    normalize(&mut v, 0.8);
    v
}

pub fn wind() -> Vec<f32> {
    let mut rng = Rng(0x9e37_79b9);
    let fade = secs(0.4);
    let mut v = noise(&mut rng, secs(3.0) + fade);
    lowpass(&mut v, 0.12);
    lowpass(&mut v, 0.3);
    let mut v = seamless(&v, fade);
    normalize(&mut v, 0.9);
    v
}

pub fn water() -> Vec<f32> {
    let mut rng = Rng(0x1234_5671);
    let fade = secs(0.4);
    let n = secs(2.0) + fade;
    let mut hi = noise(&mut rng, n);
    lowpass(&mut hi, 0.55);
    let mut lo = hi.clone();
    lowpass(&mut lo, 0.12);
    let mut burble = noise(&mut rng, n);
    lowpass(&mut burble, 0.002);
    normalize(&mut burble, 1.0);
    let v: Vec<f32> = (0..n).map(|i| (hi[i] - lo[i]) * (0.65 + 0.35 * burble[i])).collect();
    let mut v = seamless(&v, fade);
    normalize(&mut v, 0.9);
    v
}

pub fn fire() -> Vec<f32> {
    let mut rng = Rng(0xdead_beef);
    let fade = secs(0.3);
    let n = secs(2.0) + fade;
    let mut rumble = noise(&mut rng, n);
    lowpass(&mut rumble, 0.05);
    normalize(&mut rumble, 0.35);
    let mut v = rumble;
    let mut i = 0;
    while i < n {
        i += secs(0.02) + (rng.next().abs() * RATE as f32 * 0.12) as usize;
        let amp = 0.3 + rng.next().abs() * 0.7;
        for k in 0..secs(0.012) {
            if i + k < n {
                v[i + k] += rng.next() * amp * (1.0 - k as f32 / secs(0.012) as f32);
            }
        }
    }
    let mut v = seamless(&v, fade);
    normalize(&mut v, 0.9);
    v
}

pub fn boom(len: f32, deep: f32) -> Vec<f32> {
    let mut rng = Rng(0x0bad_cafe);
    let n = secs(len);
    let mut grit = noise(&mut rng, n);
    lowpass(&mut grit, 0.35);
    let mut rumble = noise(&mut rng, n);
    lowpass(&mut rumble, 0.03);
    normalize(&mut rumble, 1.0);
    let mut v: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let body = (TAU * (28.0 + 55.0 * deep * (-t * 6.0).exp()) * t).sin() * (-t * 2.4 / len * 2.0).exp();
            let crack = grit[i] * (-t * 14.0).exp() * 0.9;
            body * 0.8 + crack + rumble[i] * (-t * 3.0 / len).exp() * 0.6
        })
        .collect();
    let attack = secs(0.004);
    for i in 0..attack.min(n) {
        v[i] *= i as f32 / attack as f32;
    }
    normalize(&mut v, 0.95);
    v
}

/// Thunder: the sharp crack of the channel (strong in `near`), then a long rumble rolling on as the
/// sound of farther and farther parts of the bolt arrives.
pub fn thunder(near: f32) -> Vec<f32> {
    let mut rng = Rng(0x7a11_c0de ^ (near * 1000.0) as u32);
    let len = 4.5;
    let n = secs(len);
    let mut crack = noise(&mut rng, n);
    lowpass(&mut crack, 0.5);
    let mut rumble = noise(&mut rng, n);
    lowpass(&mut rumble, 0.012);
    normalize(&mut rumble, 1.0);
    let mut roll = noise(&mut rng, n);
    lowpass(&mut roll, 0.04);
    normalize(&mut roll, 1.0);
    // Slow bursts of loudness: the rolling.
    let mut bumps = noise(&mut rng, n);
    lowpass(&mut bumps, 0.0004);
    normalize(&mut bumps, 1.0);
    let mut v: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let snap = crack[i] * (-t * 9.0).exp() * near;
            let body = (rumble[i] * 0.9 + roll[i] * 0.45 * (-t * 1.2).exp()) * (1.0 - (-t * 30.0).exp()) * (-t * 0.75).exp();
            let wobble = 0.6 + 0.4 * bumps[i].abs() * 2.0;
            snap * 1.2 + body * wobble
        })
        .collect();
    let attack = secs(0.003);
    for i in 0..attack.min(n) {
        v[i] *= i as f32 / attack as f32;
    }
    // Fade out the tail.
    let tail = secs(0.8);
    for k in 0..tail.min(n) {
        v[n - 1 - k] *= k as f32 / tail as f32;
    }
    normalize(&mut v, 0.95);
    v
}

pub fn thud() -> Vec<f32> {
    let mut rng = Rng(77);
    let n = secs(0.25);
    let mut v: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            (TAU * (110.0 - 220.0 * t).max(45.0) * t).sin() * (-t * 22.0).exp() + rng.next() * 0.15 * (-t * 60.0).exp()
        })
        .collect();
    normalize(&mut v, 0.9);
    v
}

pub fn clink() -> Vec<f32> {
    let mut rng = Rng(4242);
    let n = secs(0.45);
    let mut v: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let ring: f32 = [2300.0f32, 3100.0, 4700.0, 5200.0].iter().enumerate().map(|(k, f)| (TAU * f * t).sin() * (-t * (9.0 + 5.0 * k as f32)).exp()).sum();
            ring + rng.next() * 0.8 * (-t * 120.0).exp()
        })
        .collect();
    normalize(&mut v, 0.8);
    v
}

pub fn crack() -> Vec<f32> {
    let mut rng = Rng(999);
    let n = secs(0.16);
    let mut raw = noise(&mut rng, n);
    let mut low = raw.clone();
    lowpass(&mut low, 0.15);
    for i in 0..n {
        let t = i as f32 / RATE as f32;
        raw[i] = (raw[i] - low[i]) * (-t * 38.0).exp() + (TAU * 180.0 * t).sin() * (-t * 50.0).exp() * 0.4;
    }
    normalize(&mut raw, 0.85);
    raw
}

pub fn spray() -> Vec<f32> {
    let mut rng = Rng(31337);
    let n = secs(0.3);
    let mut v = noise(&mut rng, n);
    let mut low = v.clone();
    lowpass(&mut low, 0.08);
    for i in 0..n {
        let t = i as f32 / n as f32;
        v[i] = (v[i] - low[i]) * (t * 12.0).min(1.0) * (1.0 - t).powi(2);
    }
    normalize(&mut v, 0.7);
    v
}

pub fn snap() -> Vec<f32> {
    let mut rng = Rng(5);
    let n = secs(0.18);
    let mut v: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            rng.next() * (-t * 140.0).exp() + (TAU * 1400.0 * t).sin() * (-t * 45.0).exp() * 0.6
        })
        .collect();
    normalize(&mut v, 0.85);
    v
}

/// The French fire engine's two-tone siren: 435 Hz and 488 Hz, half a second each, twice.
pub fn siren() -> Vec<f32> {
    let n = secs(2.0);
    let mut ph = 0.0f32;
    let mut v: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let hz = if (t * 2.0) as u32 % 2 == 0 { 435.0 } else { 488.0 };
            ph += hz / RATE as f32;
            // A bit of a horn: a few odd harmonics, soft edges at the start and the end.
            let s = (TAU * ph).sin() + (TAU * ph * 3.0).sin() * 0.3 + (TAU * ph * 5.0).sin() * 0.12;
            s * (t * 20.0).min(1.0) * ((2.0 - t) * 20.0).min(1.0)
        })
        .collect();
    normalize(&mut v, 0.5);
    v
}

/// A firework burst: a dull thump followed by a shower of crackles.
pub fn pop() -> Vec<f32> {
    let mut rng = Rng(777);
    let n = secs(1.2);
    let mut v = vec![0.0f32; n];
    for (i, s) in v.iter_mut().enumerate() {
        let t = i as f32 / RATE as f32;
        *s = rng.next() * (-t * 30.0).exp() * 0.8 + (TAU * 70.0 * t).sin() * (-t * 12.0).exp();
    }
    // Crackles: tiny clicks scattered over the second after the bang.
    for k in 0..60 {
        let at = secs(0.15 + (k as f32 * 0.618).fract() * 0.95);
        let amp = 0.5 * (1.0 - k as f32 / 60.0) + 0.1;
        for j in 0..secs(0.006).min(n - at) {
            v[at + j] += rng.next() * amp * (1.0 - j as f32 / secs(0.006) as f32);
        }
    }
    normalize(&mut v, 0.8);
    v
}

/// A whale's breath: a deep, airy whoosh.
pub fn blow() -> Vec<f32> {
    let mut rng = Rng(4242);
    let n = secs(1.4);
    let mut v = noise(&mut rng, n);
    lowpass(&mut v, 0.12);
    for (i, s) in v.iter_mut().enumerate() {
        let t = i as f32 / n as f32;
        *s *= (t * 8.0).min(1.0) * (1.0 - t).powf(1.5);
    }
    normalize(&mut v, 0.7);
    v
}

/// 16-bit mono PCM WAV, for macroquad's audio (Linux plays the samples directly).
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub fn wav(samples: &[f32]) -> Vec<u8> {
    let data = samples.len() as u32 * 2;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32000.0) as i16).to_le_bytes());
    }
    out
}

// ---------------------------------------------------------------- playback

#[cfg(feature = "sound")]
struct Bank {
    out: Out,
    engine: Vec<Clip>,
    wind: Clip,
    water: Clip,
    fire: Clip,
    boom_big: Clip,
    boom_small: Clip,
    thud: Clip,
    clink: Clip,
    crack: Clip,
    spray: Clip,
    snap: Clip,
    siren: Clip,
    pop: Clip,
    blow: Clip,
    thunder_near: Clip,
    thunder_far: Clip,
}

pub struct Sounds {
    #[cfg(feature = "sound")]
    bank: Option<Bank>,
    pub muted: bool,
    queue: Vec<Ev>,
    cooldown: [f32; 10],
    /// Smoothed loop volumes: engine x4, wind, water, fire.
    level: [f32; 7],
}

impl Sounds {
    /// No audio at all (tests, `--demo`, or a machine without a sound card).
    pub fn silent() -> Self {
        Sounds {
            #[cfg(feature = "sound")]
            bank: None,
            muted: true,
            queue: vec![],
            cooldown: [0.0; 10],
            level: [0.0; 7],
        }
    }

    #[cfg(feature = "sound")]
    pub async fn load() -> Self {
        let Some(out) = Out::open() else { return Sounds::silent() };
        let one = |samples: Vec<f32>| {
            let out = &out;
            async move { out.load(&samples).await }
        };
        let looped = |samples: Vec<f32>| {
            let out = &out;
            async move {
                let c = out.load(&samples).await?;
                out.play(&c, true, 0.0);
                Some(c)
            }
        };
        let all = async {
            let mut engines = vec![];
            for hz in ENGINE_HZ {
                engines.push(looped(engine(hz)).await?);
            }
            Some(Bank {
                out: out.clone(),
                engine: engines,
                wind: looped(wind()).await?,
                water: looped(water()).await?,
                fire: looped(fire()).await?,
                boom_big: one(boom(2.0, 1.0)).await?,
                boom_small: one(boom(0.8, 0.4)).await?,
                thud: one(thud()).await?,
                clink: one(clink()).await?,
                crack: one(crack()).await?,
                spray: one(spray()).await?,
                snap: one(snap()).await?,
                siren: one(siren()).await?,
                pop: one(pop()).await?,
                blow: one(blow()).await?,
                thunder_near: one(thunder(1.0)).await?,
                thunder_far: one(thunder(0.15)).await?,
            })
        }
        .await;
        Sounds { muted: all.is_none(), bank: all, ..Sounds::silent() }
    }

    pub fn emit(&mut self, e: Ev) {
        if !self.muted && self.queue.len() < 12 {
            self.queue.push(e);
        }
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        if self.muted {
            self.queue.clear();
        }
    }

    /// Target loop volumes for a mix: engine x4 (crossfaded by rpm), wind, water, fire.
    pub fn targets(mix: &Mix) -> [f32; 7] {
        let mut t = [0.0f32; 7];
        if let Some(r) = mix.rpm {
            let pos = r.clamp(0.0, 1.0) * 3.0;
            for (k, v) in t.iter_mut().take(4).enumerate() {
                let w = (1.0 - (pos - k as f32).abs()).max(0.0);
                *v = w.sqrt() * 0.34;
            }
        }
        t[4] = mix.wind.clamp(0.0, 1.0) * 0.8;
        t[5] = mix.water.clamp(0.0, 1.0) * 0.55;
        t[6] = mix.fire.clamp(0.0, 1.0) * 0.5;
        t
    }

    pub fn update(&mut self, dt: f32, mix: &Mix) {
        let target = if self.muted { [0.0; 7] } else { Self::targets(mix) };
        let k = (dt * 7.0).min(1.0);
        for (l, t) in self.level.iter_mut().zip(target) {
            *l += (t - *l) * k;
        }
        for c in &mut self.cooldown {
            *c = (*c - dt).max(0.0);
        }
        let queue = std::mem::take(&mut self.queue);
        #[cfg(feature = "sound")]
        if let Some(b) = &self.bank {
            for (s, l) in b.engine.iter().zip(self.level) {
                b.out.set_volume(s, l);
            }
            b.out.set_volume(&b.wind, self.level[4]);
            b.out.set_volume(&b.water, self.level[5]);
            b.out.set_volume(&b.fire, self.level[6]);
            for e in queue {
                let slot = e.slot();
                if self.cooldown[slot] > 0.0 {
                    continue;
                }
                self.cooldown[slot] = match e {
                    Ev::Boom(_) => 0.12,
                    // The siren is two seconds long: never two at once.
                    Ev::Siren => 2.0,
                    Ev::Pop => 0.2,
                    Ev::Thunder(_) => 0.4,
                    _ => 0.07,
                };
                let (s, v) = match e {
                    Ev::Boom(r) => (if r >= 7.0 { &b.boom_big } else { &b.boom_small }, (0.45 + r * 0.06).min(1.0)),
                    Ev::Thud(p) => (&b.thud, (0.2 + p * 0.12).min(0.9)),
                    Ev::Clink => (&b.clink, 0.6),
                    Ev::Crack => (&b.crack, 0.7),
                    Ev::Spray => (&b.spray, 0.45),
                    Ev::Snap => (&b.snap, 0.7),
                    Ev::Siren => (&b.siren, 0.35),
                    Ev::Pop => (&b.pop, 0.55),
                    Ev::Blow => (&b.blow, 0.5),
                    Ev::Thunder(p) => (if p > 0.55 { &b.thunder_near } else { &b.thunder_far }, (0.25 + 0.75 * p).min(1.0)),
                };
                b.out.play(s, false, v);
            }
        }
        #[cfg(not(feature = "sound"))]
        drop(queue);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sane(v: &[f32]) {
        assert!(!v.is_empty());
        assert!(v.iter().all(|s| s.is_finite() && s.abs() <= 1.0001), "samples out of range");
        let peak = v.iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!(peak > 0.3, "too quiet: {peak}");
    }

    #[test]
    fn every_sound_is_audible_and_in_range() {
        for hz in ENGINE_HZ {
            sane(&engine(hz));
        }
        for v in [wind(), water(), fire(), boom(2.0, 1.0), boom(0.8, 0.4), thud(), clink(), crack(), spray(), snap(), siren(), pop(), blow(), thunder(1.0), thunder(0.15)] {
            sane(&v);
        }
    }

    #[test]
    fn loops_have_no_click_at_the_seam() {
        for v in [wind(), water(), fire(), engine(66.0)] {
            let jump = (v[0] - v[v.len() - 1]).abs();
            let typical = v.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
            assert!(jump <= typical * 1.5 + 0.02, "seam jump {jump} vs max step {typical}");
        }
    }

    #[test]
    fn wav_header_is_valid() {
        let w = wav(&[0.0, 0.5, -0.5, 1.0]);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize, w.len() - 8);
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()), 8);
        assert_eq!(w.len(), 44 + 8);
    }

    #[test]
    fn engine_crossfades_by_speed() {
        let idle = Sounds::targets(&Mix { rpm: Some(0.0), ..Mix::default() });
        let fast = Sounds::targets(&Mix { rpm: Some(1.0), ..Mix::default() });
        assert!(idle[0] > 0.2 && idle[3] == 0.0);
        assert!(fast[3] > 0.2 && fast[0] == 0.0);
        let none = Sounds::targets(&Mix::default());
        assert!(none.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn muted_game_queues_nothing() {
        let mut s = Sounds::silent();
        s.emit(Ev::Snap);
        assert!(s.queue.is_empty());
        s.muted = false;
        s.emit(Ev::Snap);
        assert_eq!(s.queue.len(), 1);
    }
}
