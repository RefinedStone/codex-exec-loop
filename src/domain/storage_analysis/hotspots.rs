//! The threshold compares exact integers; u128 products cannot overflow u64 byte inputs.
use super::HotspotThresholds;

pub fn should_expand(
    child: u64,
    parent: u64,
    thresholds: &HotspotThresholds,
) -> Result<bool, String> {
    let (absolute, minimum, percent) = thresholds.values()?;
    Ok(child >= absolute
        || (child >= minimum
            && parent > 0
            && u128::from(child) * 100 >= u128::from(parent) * u128::from(percent)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn boundaries_and_large_integer_comparisons() {
        let t = HotspotThresholds::default();
        let mib = 1024_u64.pow(2);
        assert!(!should_expand(64 * mib, 1024 * mib, &t).unwrap());
        assert!(should_expand(768 * mib, 1024 * mib, &t).unwrap());
        assert!(should_expand(3 * 1024 * mib, 100 * 1024 * mib, &t).unwrap());
        assert!(!should_expand(256 * mib - 1, 512 * mib, &t).unwrap());
        assert!(should_expand(256 * mib, 512 * mib, &t).unwrap());
        let custom = HotspotThresholds {
            absolute_bytes: u64::MAX.to_string(),
            relative_min_bytes: "1".into(),
            parent_percent: 20,
        };
        assert!(!should_expand(199, 1000, &custom).unwrap());
        assert!(should_expand(200, 1000, &custom).unwrap());
        assert!(should_expand(u64::MAX / 2, u64::MAX, &custom).unwrap());
    }
}
