//! Roundings to a whole number that give the library's result without calling the C library,
//! which `floor`, `ceil` and `round` are on x86 builds for older processors. The hot loops (the
//! wind carrying heat, the cells reading the wind) round millions of times a second.

/// `v.floor() as i32`.
#[inline]
pub fn floor_i(v: f32) -> i32 {
    if !(v.abs() < 8_388_608.0) {
        return v.floor() as i32;
    }
    let t = v as i32;
    if (t as f32) > v { t - 1 } else { t }
}

/// `v.floor()`, to the bit (the floor of -0.0 is -0.0).
#[inline]
pub fn floor_f(v: f32) -> f32 {
    if !(v.abs() < 8_388_608.0) {
        return v.floor();
    }
    // (A floor has the sign of what it floors.)
    (floor_i(v) as f32).copysign(v)
}

/// `v.ceil() as i32`.
#[inline]
pub fn ceil_i(v: f32) -> i32 {
    if !(v.abs() < 8_388_608.0) {
        return v.ceil() as i32;
    }
    let t = v as i32;
    if (t as f32) < v { t + 1 } else { t }
}

/// `v.round() as i32` (halves away from zero).
#[inline]
pub fn round_i(v: f32) -> i32 {
    if !(v.abs() < 8_388_608.0) {
        return v.round() as i32;
    }
    let t = v as i32;
    // (Exact: what is left of a float past its whole part is a float.)
    let f = v - t as f32;
    if f >= 0.5 {
        t + 1
    } else if f <= -0.5 {
        t - 1
    } else {
        t
    }
}

#[cfg(test)]
mod rounding {
    use super::*;

    #[test]
    fn the_quick_roundings_match_the_library_ones() {
        let mut v = vec![0.0f32, -0.0, 0.3, -0.3, f32::MIN_POSITIVE, -f32::MIN_POSITIVE, 0.5, -0.5, 1.5, -1.5, 2.5, -2.5, 0.49999997, -0.49999997, 1e-30, -1e-30, 8_388_607.5, -8_388_607.5, 1e9, -1e9, f32::MAX, f32::MIN];
        let mut r = 0x2545_f491u32;
        for _ in 0..1_000_000 {
            r ^= r << 13;
            r ^= r >> 17;
            r ^= r << 5;
            v.push(f32::from_bits(r) % 4096.0);
            v.push((r % 200_000) as f32 / 97.0 - 1000.0);
        }
        for x in v.into_iter().filter(|x| x.is_finite()) {
            assert_eq!(floor_i(x), x.floor() as i32, "floor {x}");
            assert_eq!(floor_f(x).to_bits(), x.floor().to_bits(), "floor {x}");
            assert_eq!(round_i(x), x.round() as i32, "round {x}");
            assert_eq!(ceil_i(x), x.ceil() as i32, "ceil {x}");
        }
    }
}
