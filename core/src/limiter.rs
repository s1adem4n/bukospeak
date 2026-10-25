//! A soft limiter, for a mix or a voice whose gain was raised.

/// Where the limiter starts to bend; below it samples pass unchanged.
const KNEE: f32 = 0.8;

/// Bends samples beyond the knee smoothly towards ±1, so they never clip: linear below the knee,
/// and tanh above it, which meets it with the same slope.
pub fn limit(sample: f32) -> f32 {
    let magnitude = sample.abs();
    if magnitude <= KNEE {
        return sample;
    }
    let headroom = 1.0 - KNEE;
    (KNEE + headroom * ((magnitude - KNEE) / headroom).tanh()).copysign(sample)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_passes_quiet_samples_and_bends_loud_ones_below_full_scale() {
        assert_eq!(limit(0.5), 0.5);
        assert_eq!(limit(-KNEE), -KNEE);
        for loud in [0.9, 1.0, 2.0, 100.0] {
            // tanh reaches 1.0 in f32 for large arguments, so the bound is inclusive.
            assert!(limit(loud) > KNEE && limit(loud) <= 1.0, "limit({loud}) = {}", limit(loud));
            assert_eq!(limit(-loud), -limit(loud));
        }
        assert!(limit(0.9) < limit(1.0) && limit(1.0) < limit(2.0));
    }
}
