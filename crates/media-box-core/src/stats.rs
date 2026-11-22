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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn medians_odd_even_empty_and_unsorted() {
        assert_eq!(f64_median(&mut []), None);
        assert_eq!(f64_median(&mut [3.0]), Some(3.0));

        // 乱序奇数个：取正中位
        assert_eq!(f64_median(&mut [3.0, 1.0, 2.0]), Some(2.0));
        // 偶数个：取中间两数平均
        assert_eq!(f64_median(&mut [4.0, 1.0, 3.0, 2.0]), Some(2.5));
        // 全等不受分区影响
        assert_eq!(f64_median(&mut [5.0, 5.0, 5.0]), Some(5.0));

        // total_cmp 提供全序：含 NaN 也不 panic（NaN 排在最后）
        assert_eq!(f64_median(&mut [1.0, f64::NAN, 3.0]), Some(3.0));
    }
}
