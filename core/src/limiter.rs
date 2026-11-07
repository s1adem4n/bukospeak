const KNEE: f32 = 0.8;

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
            assert!(limit(loud) > KNEE && limit(loud) <= 1.0, "limit({loud}) = {}", limit(loud));
            assert_eq!(limit(-loud), -limit(loud));
        }
        assert!(limit(0.9) < limit(1.0) && limit(1.0) < limit(2.0));
    }
}
