/// Nearest-rank percentile (p in 0..=100). Empty input gives 0.
pub fn percentile(ms: &[u128], p: f64) -> u128 {
    if ms.is_empty() {
        return 0;
    }
    let mut v = ms.to_vec();
    v.sort_unstable();
    let rank = ((p / 100.0) * v.len() as f64).ceil() as usize;
    v[rank.clamp(1, v.len()) - 1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles() {
        assert_eq!(percentile(&[100, 200, 300, 400, 1000], 95.0), 1000);
        assert_eq!(percentile(&[100, 200, 300, 400, 1000], 50.0), 300);
    }
}
