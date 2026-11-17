//! Ten-band graphic EQ with compensated band interaction, double-precision
//! processing and a finite 50 ms coefficient ramp. Band values describe the
//! combined response at the labelled frequencies, rather than ten unrelated
//! bells which can add up to 30 dB in the crowded upper bands.

pub const BANDS: [f32; 10] = [
    60.0, 170.0, 310.0, 600.0, 1000.0, 3000.0, 6000.0, 12000.0, 14000.0, 16000.0,
];
pub const RANGE_DB: f32 = 12.0;
const RAMP_S: f64 = 0.05;

#[derive(Clone, Copy)]
pub struct Preset {
    pub name: &'static str,
    pub ru: &'static str,
    pub preamp: f32,
    pub bands: [f32; 10],
}
/// Gentle starting points, with output headroom included in the preamp.
/// Saved user curves retain their values and their manually chosen preamp.
pub const PRESETS: [Preset; 11] = [
    Preset {
        name: "Flat",
        ru: "Ровный",
        preamp: 0.0,
        bands: [0.0; 10],
    },
    Preset {
        name: "Warm",
        ru: "Тёплый",
        preamp: -2.5,
        bands: [2.0, 1.5, 0.5, 0.0, -0.5, -0.5, -1.0, -1.0, -1.0, -1.0],
    },
    Preset {
        name: "Bright",
        ru: "Светлый",
        preamp: -2.5,
        bands: [-1.0, -0.5, 0.0, 0.0, 0.0, 0.5, 1.5, 2.0, 2.0, 2.0],
    },
    Preset {
        name: "Bass",
        ru: "Бас",
        preamp: -3.5,
        bands: [3.0, 2.5, 1.0, 0.0, -0.5, -0.5, 0.0, 0.0, 0.0, 0.0],
    },
    Preset {
        name: "Bass and treble",
        ru: "Бас и верх",
        preamp: -2.5,
        bands: [2.0, 1.5, 0.0, -1.0, -1.0, 0.0, 1.0, 1.5, 1.5, 1.5],
    },
    Preset {
        name: "Vocal",
        ru: "Вокал",
        preamp: -2.5,
        bands: [-2.0, -1.0, 0.0, 0.5, 1.5, 2.0, 0.0, -0.5, -0.5, -0.5],
    },
    Preset {
        name: "Rock",
        ru: "Рок",
        preamp: -2.5,
        bands: [2.0, 1.0, -0.5, -1.0, -0.5, 0.5, 1.0, 1.0, 1.0, 1.0],
    },
    Preset {
        name: "Pop",
        ru: "Поп",
        preamp: -1.5,
        bands: [0.5, 0.0, -0.5, 0.0, 1.0, 1.0, 0.5, 0.5, 0.5, 0.5],
    },
    Preset {
        name: "Electronic",
        ru: "Электронная музыка",
        preamp: -3.0,
        bands: [2.5, 1.5, 0.0, -0.5, -0.5, 0.0, 1.0, 1.5, 1.5, 1.5],
    },
    Preset {
        name: "Acoustic",
        ru: "Акустика",
        preamp: -1.5,
        bands: [-0.5, -0.5, 0.0, 0.5, 0.5, 1.0, 0.5, 0.5, 0.5, 0.5],
    },
    Preset {
        name: "Small speakers",
        ru: "Небольшие колонки",
        preamp: -1.5,
        bands: [-3.0, 0.0, 1.0, 1.0, 0.5, 0.0, 0.5, 0.0, -0.5, -0.5],
    },
];

#[derive(Clone, Copy, Debug, PartialEq)]
struct Coeffs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}
impl Default for Coeffs {
    fn default() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        }
    }
}
impl Coeffs {
    fn peak(rate: u32, hz: f64, q: f64, db: f64) -> Self {
        if db.abs() < 1e-10 {
            return Self::default();
        }
        let a = 10f64.powf(db / 40.0);
        let w = std::f64::consts::TAU * hz / f64::from(rate);
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q);
        let a0 = 1.0 + alpha / a;
        Self {
            b0: (1.0 + alpha * a) / a0,
            b1: -2.0 * c / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: -2.0 * c / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }
    fn gain_db(self, rate: u32, hz: f64) -> f64 {
        let w = std::f64::consts::TAU * hz / f64::from(rate);
        let (s, c) = w.sin_cos();
        let (s2, c2) = (2.0 * w).sin_cos();
        let numerator =
            (self.b0 + self.b1 * c + self.b2 * c2).powi(2) + (self.b1 * s + self.b2 * s2).powi(2);
        let denominator =
            (1.0 + self.a1 * c + self.a2 * c2).powi(2) + (self.a1 * s + self.a2 * s2).powi(2);
        10.0 * (numerator / denominator.max(1e-30)).max(1e-30).log10()
    }
    fn towards(self, to: Self, k: f64) -> Self {
        Self {
            b0: self.b0 + (to.b0 - self.b0) * k,
            b1: self.b1 + (to.b1 - self.b1) * k,
            b2: self.b2 + (to.b2 - self.b2) * k,
            a1: self.a1 + (to.a1 - self.a1) * k,
            a2: self.a2 + (to.a2 - self.a2) * k,
        }
    }
}

fn finite_gain(db: f32) -> f64 {
    if db.is_finite() {
        f64::from(db.clamp(-RANGE_DB, RANGE_DB))
    } else {
        0.0
    }
}

/// The settled transfer function used both by DSP and the response plot.
#[derive(Clone)]
pub struct Response {
    rate: u32,
    coefficients: [Coeffs; 10],
    preamp_db: f64,
}
impl Response {
    pub fn new(rate: u32, preamp: f32, bands: &[f32; 10]) -> Self {
        let rate = rate.max(1);
        let active = BANDS.map(|f| f < rate as f32 * 0.45);
        let wanted = std::array::from_fn::<_, 10, _>(|i| {
            if active[i] {
                finite_gain(bands[i])
            } else {
                0.0
            }
        });
        let mut q = [1.0; 10];
        let count = active.iter().filter(|&&a| a).count();
        for i in 0..count {
            let hz = f64::from(BANDS[i]);
            let prev = if i > 0 {
                f64::from(BANDS[i - 1])
            } else {
                hz * hz / f64::from(BANDS[(i + 1).min(9)])
            };
            let next = if i + 1 < count {
                f64::from(BANDS[i + 1])
            } else {
                hz * hz / prev
            };
            let warp = |f: f64| {
                (std::f64::consts::PI * f.min(f64::from(rate) * 0.49) / f64::from(rate)).tan()
            };
            let width = warp((hz * next).sqrt()) - warp((hz * prev).sqrt());
            q[i] = (warp(hz) / width.max(1e-9)).clamp(0.3, 12.0);
        }
        let make = |gains: &[f64; 10]| {
            std::array::from_fn::<_, 10, _>(|i| {
                if active[i] {
                    Coeffs::peak(rate, f64::from(BANDS[i]), q[i], gains[i])
                } else {
                    Coeffs::default()
                }
            })
        };
        let mut gains = wanted;
        // Newton fitting compensates the response of neighbouring bells.
        // Setup work only: audio processing never solves a system or allocates.
        for _ in 0..8 {
            let filters = make(&gains);
            let mut system = [[0.0; 11]; 10];
            let mut error = 0.0f64;
            for i in 0..10 {
                if !active[i] {
                    system[i][i] = 1.0;
                    continue;
                }
                let hz = f64::from(BANDS[i]);
                let actual: f64 = filters.iter().map(|c| c.gain_db(rate, hz)).sum();
                system[i][10] = wanted[i] - actual;
                error = error.max(system[i][10].abs());
                for j in 0..10 {
                    if active[j] {
                        let lo = Coeffs::peak(rate, f64::from(BANDS[j]), q[j], gains[j] - 0.05)
                            .gain_db(rate, hz);
                        let hi = Coeffs::peak(rate, f64::from(BANDS[j]), q[j], gains[j] + 0.05)
                            .gain_db(rate, hz);
                        system[i][j] = (hi - lo) / 0.1;
                    }
                }
            }
            if error < 0.005 {
                break;
            }
            if let Some(delta) = solve(system) {
                for i in 0..10 {
                    gains[i] = (gains[i] + delta[i].clamp(-6.0, 6.0)).clamp(-30.0, 30.0);
                }
            } else {
                break;
            }
        }
        Self {
            rate,
            coefficients: make(&gains),
            preamp_db: finite_gain(preamp),
        }
    }
    pub fn gain_db(&self, hz: f32) -> f32 {
        let hz = f64::from(hz).clamp(0.0, f64::from(self.rate) * 0.5);
        (self.preamp_db
            + self
                .coefficients
                .iter()
                .map(|c| c.gain_db(self.rate, hz))
                .sum::<f64>()) as f32
    }
    /// Dense log grid plus every command frequency, including DC/Nyquist.
    #[cfg(test)]
    pub fn peak_db(&self) -> f32 {
        let upper = (self.rate as f32 * 0.5).min(24000.0);
        let mut peak = self.gain_db(0.0).max(self.gain_db(upper));
        for i in 0..512 {
            let hz = 10.0 * (upper / 10.0).powf(i as f32 / 511.0);
            peak = peak.max(self.gain_db(hz));
        }
        for hz in BANDS {
            if hz < upper {
                peak = peak.max(self.gain_db(hz));
            }
        }
        peak
    }
}

fn solve(mut a: [[f64; 11]; 10]) -> Option<[f64; 10]> {
    for col in 0..10 {
        let pivot = (col..10).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() < 1e-9 {
            return None;
        }
        a.swap(col, pivot);
        let divisor = a[col][col];
        for value in &mut a[col][col..] {
            *value /= divisor;
        }
        let pivot_row = a[col];
        for (index, row) in a.iter_mut().enumerate() {
            if index != col {
                let factor = row[col];
                for (value, &pivot) in row[col..].iter_mut().zip(&pivot_row[col..]) {
                    *value -= factor * pivot;
                }
            }
        }
    }
    Some(std::array::from_fn(|i| a[i][10]))
}

#[derive(Clone, Copy, Default)]
struct State {
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}
impl State {
    fn run(&mut self, c: Coeffs, x: f64) -> f64 {
        // Direct form I retains actual input/output history when
        // coefficients move. DF-II's stored sums belong to the previous
        // coefficients and can produce a thump during a live change.
        let y = c.b0 * x + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = if y.abs() < 1e-24 { 0.0 } else { y };
        y
    }
}
pub struct Equalizer {
    rate: u32,
    channels: usize,
    states: Vec<[State; 10]>,
    now: Response,
    target: Response,
    remaining: usize,
    gain: f64,
    target_gain: f64,
}
impl Equalizer {
    pub fn new(rate: u32, channels: usize, preamp_db: f32, gains: &[f32; 10]) -> Self {
        let response = Response::new(rate, preamp_db, gains);
        let gain = 10f64.powf(response.preamp_db / 20.0);
        let channels = channels.max(1);
        Self {
            rate: rate.max(1),
            channels,
            states: vec![[State::default(); 10]; channels],
            now: response.clone(),
            target: response,
            remaining: 0,
            gain,
            target_gain: gain,
        }
    }
    pub fn configure(&mut self, preamp_db: f32, gains: &[f32; 10]) {
        self.target = Response::new(self.rate, preamp_db, gains);
        self.target_gain = 10f64.powf(self.target.preamp_db / 20.0);
        self.remaining = (f64::from(self.rate) * RAMP_S).round().max(1.0) as usize;
    }
    pub fn process(&mut self, samples: &mut [f32]) {
        for frame in samples.chunks_exact_mut(self.channels) {
            if self.remaining > 0 {
                let k = 1.0 / self.remaining as f64;
                for (now, &to) in self
                    .now
                    .coefficients
                    .iter_mut()
                    .zip(&self.target.coefficients)
                {
                    *now = now.towards(to, k);
                }
                self.gain += (self.target_gain - self.gain) * k;
                self.remaining -= 1;
                if self.remaining == 0 {
                    self.now = self.target.clone();
                    self.gain = self.target_gain;
                }
            }
            for (x, states) in frame.iter_mut().zip(&mut self.states) {
                let mut y = f64::from(*x) * self.gain;
                for (state, &c) in states.iter_mut().zip(&self.now.coefficients) {
                    y = state.run(c, y);
                }
                *x = y as f32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn measured(rate: u32, bands: &[f32; 10], hz: f64) -> f64 {
        let mut e = Equalizer::new(rate, 2, 0.0, bands);
        let mut s: Vec<f32> = (0..rate * 2)
            .flat_map(|i| {
                let x = (0.02 * (std::f64::consts::TAU * hz * f64::from(i) / f64::from(rate)).sin())
                    as f32;
                [x, x]
            })
            .collect();
        e.process(&mut s);
        let rms = (s[rate as usize * 2..]
            .iter()
            .map(|&x| f64::from(x).powi(2))
            .sum::<f64>()
            / (rate * 2) as f64)
            .sqrt();
        20.0 * (rms * 2f64.sqrt() / 0.02).log10()
    }
    #[test]
    fn command_gains_match_the_combined_response() {
        for rate in [8000, 22050, 44100, 48000, 96000, 192000] {
            for bands in [
                [6.0; 10],
                [-6.0; 10],
                std::array::from_fn(|i| if i % 2 == 0 { 12.0 } else { -12.0 }),
            ] {
                let r = Response::new(rate, 0.0, &bands);
                for (i, hz) in BANDS.into_iter().enumerate() {
                    if hz < rate as f32 * 0.45 {
                        assert!(
                            (r.gain_db(hz) - bands[i]).abs() < 0.15,
                            "{rate} {hz} target{} actual{}",
                            bands[i],
                            r.gain_db(hz)
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn presets_have_headroom_and_moderate_curves() {
        for p in PRESETS {
            for rate in [44100, 48000, 96000, 192000] {
                let r = Response::new(rate, p.preamp, &p.bands);
                assert!(r.peak_db() <= 0.05, "{} {rate}: {} dB", p.name, r.peak_db());
                assert!(p.bands.iter().all(|v| v.abs() <= 3.0));
            }
        }
    }
    #[test]
    fn flat_is_an_exact_wire_and_response_matches_audio() {
        for rate in [44100, 192000] {
            let input: Vec<f32> = (0..10000).map(|i| 0.1 * (i as f32 * 0.31).sin()).collect();
            let mut flat = input.clone();
            Equalizer::new(rate, 2, 0.0, &[0.0; 10]).process(&mut flat);
            assert_eq!(flat, input);
            let bands = PRESETS[6].bands;
            for hz in [60.0, 1000.0, 14000.0] {
                let actual = measured(rate, &bands, hz);
                let expected = Response::new(rate, 0.0, &bands).gain_db(hz as f32);
                assert!(
                    (actual - f64::from(expected)).abs() < 0.02,
                    "{rate} {hz}: {actual} {expected}"
                );
            }
        }
    }
    #[test]
    fn disabled_bands_cannot_replay_old_sound() {
        let mut bands = [0.0; 10];
        bands[0] = 12.0;
        let mut e = Equalizer::new(48000, 2, 0.0, &bands);
        let mut s: Vec<f32> = (0..4800)
            .flat_map(|i| {
                let x = 0.25 * (std::f64::consts::TAU * 60.0 * f64::from(i) / 48000.0).sin() as f32;
                [x, x]
            })
            .collect();
        e.process(&mut s);
        e.configure(0.0, &[0.0; 10]);
        e.process(&mut vec![0.0; 48000]);
        e.configure(0.0, &bands);
        let mut silent = vec![0.0; 48000];
        e.process(&mut silent);
        assert!(silent.iter().all(|&x| x == 0.0));
    }
    #[test]
    fn changing_filters_preserves_dc_without_a_thump() {
        for rate in [48000, 192000] {
            let mut eq = Equalizer::new(rate, 2, 0.0, &[-12.0; 10]);
            eq.process(&mut vec![0.2; rate as usize * 4]);
            eq.configure(0.0, &[12.0; 10]);
            let mut dc = vec![0.2; rate as usize / 2];
            eq.process(&mut dc);
            let error = dc.iter().map(|&x| (x - 0.2).abs()).fold(0f32, f32::max);
            assert!(error < 0.0001, "{rate}: {error}");
        }
    }

    #[test]
    fn preamp_changes_are_finite_smooth_and_block_independent() {
        let mut one = Equalizer::new(48000, 2, 0.0, &[0.0; 10]);
        let mut many = Equalizer::new(48000, 2, 0.0, &[0.0; 10]);
        one.configure(6.0, &[0.0; 10]);
        many.configure(6.0, &[0.0; 10]);
        let mut a = vec![0.2; 12000];
        let mut b = a.clone();
        one.process(&mut a);
        for part in b.chunks_mut(74) {
            many.process(part);
        }
        assert_eq!(a, b);
        assert!((a[0] - 0.2).abs() < 0.0001);
        assert!((a[a.len() - 1] - 0.2 * 10f32.powf(6.0 / 20.0)).abs() < 1e-7);
        let jump = a
            .as_chunks::<2>()
            .0
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).abs())
            .fold(0f32, f32::max);
        assert!(jump < 0.0001, "{jump}");
    }
}
