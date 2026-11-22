pub fn f64_median(v: &mut [f64]) -> Option<f64> {
    let n = v.len();

    if n == 0 {
        return None;
    }

    let mid = n / 2;
    v.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));

    if n % 2 == 1 {
        Some(v[mid])
    } else {
        let lower = *v[..mid].iter().max_by(|a, b| a.total_cmp(b)).unwrap();
        Some((lower + v[mid]) / 2.0)
    }
}
