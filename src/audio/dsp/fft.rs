/// An in-place radix-2 FFT of `re.len()` points (a power of two), with
/// `twiddles[k]` = `(cos, sin)` of `-2π k / n` for `k < n / 2`.
pub(crate) fn fft(re: &mut [f32], im: &mut [f32], twiddles: &[(f32, f32)]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n && twiddles.len() == n / 2);
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let half = len / 2;
        let step = n / len;
        for start in (0..n).step_by(len) {
            for k in 0..half {
                let (wr, wi) = twiddles[k * step];
                let (a, b) = (start + k, start + k + half);
                let (xr, xi) = (re[b], im[b]);
                let (vr, vi) = (xr * wr - xi * wi, xr * wi + xi * wr);
                re[b] = re[a] - vr;
                im[b] = im[a] - vi;
                re[a] += vr;
                im[a] += vi;
            }
        }
        len <<= 1;
    }
}
