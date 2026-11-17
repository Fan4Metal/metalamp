//! The sound processing between the decoder and the device, on the audio
//! thread, a block at a time: the track's loudness gain (`normalize`),
//! the ten-band equalizer, the enhancer, the balance and mono
//! (`channels.rs`, on whether the rest is or not), and a look-ahead
//! limiter after them whenever any of the first three is on, so nothing
//! clips (unless `limiter` is
//! turned off: then what goes past full scale is clipped by Windows, as in
//! the players of old, and a raised preamp is heard). `DspSettings` is
//! what the settings window edits (`settings.dsp`); the engine gets a
//! copy with `Cmd::Dsp` and keeps the filters' state across changes.
//! Dry and processed sound share a fixed delay, including in bypass:
//! toggling processing never drops or repeats part of the waveform.

pub mod biquad;
pub mod channels;
pub mod enhancer;
pub mod eq;
pub mod limiter;
pub mod loudness;

use serde::{Deserialize, Serialize};

use biquad::db_to_gain;

/// Where loudness normalisation brings a track: ReplayGain 2.0's
/// reference.
pub const TARGET_LUFS: f32 = -18.0;
/// The limiter's ceiling in dBFS.
pub const CEILING_DB: f32 = -1.0;
/// How far over the ceiling a raised track's peaks may go before the
/// limiter: a quiet track with a few high peaks is still brought up.
const PEAK_HEADROOM_DB: f32 = 3.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DspSettings {
    /// Every track at `TARGET_LUFS` (measured on its first play).
    pub normalize: bool,
    pub eq: bool,
    /// Decibels, within `eq::RANGE_DB`.
    pub preamp: f32,
    pub bands: [f32; 10],
    pub enhancer: bool,
    /// 0 to 10 each.
    pub bass: u8,
    pub clarity: u8,
    pub bass_harmonics: u8,
    pub air: u8,
    pub width: u8,
    pub warmth: u8,
    /// The look-ahead limiter after the rest; on by default.
    pub limiter: bool,
    /// -100 (the left alone) to 100 (the right alone); 0: both.
    pub balance: i8,
    /// Both channels carry their mean.
    pub mono: bool,
}

impl Default for DspSettings {
    fn default() -> Self {
        DspSettings {
            normalize: false,
            eq: false,
            preamp: 0.0,
            bands: [0.0; 10],
            enhancer: false,
            bass: 0,
            clarity: 0,
            bass_harmonics: 0,
            air: 0,
            width: 0,
            warmth: 0,
            limiter: true,
            balance: 0,
            mono: false,
        }
    }
}

impl DspSettings {
    /// Anything to do but the balance and mono (`channels.rs`, which go
    /// on alone, and need no limiter).
    pub fn active(&self) -> bool {
        self.normalize || self.eq || self.enhancer
    }

    /// The enhancer's amounts, all 0 while it is off (so turning it off
    /// slides it out, and on again in, with continuously warm filters).
    fn amounts(&self) -> enhancer::Amounts {
        if !self.enhancer {
            return enhancer::Amounts::default();
        }
        let f = |v: u8| f32::from(v.min(10)) / 10.0;
        enhancer::Amounts {
            bass: f(self.bass),
            clarity: f(self.clarity),
            bass_harmonics: f(self.bass_harmonics),
            air: f(self.air),
            width: f(self.width),
            warmth: f(self.warmth),
        }
    }
}

/// The gain that brings a track measured at `lufs` with a sample peak of
/// `peak` (1 = full scale) to `TARGET_LUFS`, capped so that its peaks
/// stay within `PEAK_HEADROOM_DB` of the limiter's ceiling; without the
/// limiter (`limiter` false), within full scale, so that normalisation
/// alone never clips.
pub fn track_gain(lufs: f32, peak: f32, limiter: bool) -> f32 {
    let wanted = TARGET_LUFS - lufs;
    let top = if limiter {
        CEILING_DB + PEAK_HEADROOM_DB
    } else {
        0.0
    };
    let cap = top - biquad::gain_to_db(peak.max(1e-4));
    db_to_gain(wanted.min(cap))
}

pub struct Chain {
    settings: DspSettings,
    rate: u32,
    channels: usize,
    eq: eq::Equalizer,
    enhancer: enhancer::Enhancer,
    limiter: limiter::Limiter,
    channels_stage: channels::Channels,
    /// The track gain applied now; it slides to the one asked for.
    gain: f64,
    slide: f64,
    eq_mix: f64,
    eq_slide: f64,
    eq_dry: Vec<f32>,
}

impl Chain {
    pub fn new(rate: u32, channels: u16, settings: &DspSettings) -> Chain {
        let rate = rate.max(1);
        let channels = usize::from(channels.max(1));
        Chain {
            settings: settings.clone(),
            rate,
            channels,
            eq: eq::Equalizer::new(rate, channels, settings.preamp, &settings.bands),
            enhancer: enhancer::Enhancer::new(rate, channels, settings.amounts()),
            limiter: limiter::Limiter::new(rate, channels, db_to_gain(CEILING_DB)),
            channels_stage: {
                let mut c = channels::Channels::new(rate, channels);
                c.settle(settings.balance, settings.mono);
                c
            },
            gain: 1.0,
            // A gain change takes about 300 ms.
            slide: 1.0 - (-1.0 / (f64::from(rate) * 0.3)).exp(),
            eq_mix: if settings.eq { 1.0 } else { 0.0 },
            eq_slide: 1.0 - (-1.0 / (f64::from(rate) * 0.05)).exp(),
            eq_dry: vec![0.0; channels],
        }
    }

    /// The filters' and the limiter's state dropped, the gain kept: what
    /// they hold is the sound before (another track, a seek).
    pub fn reset(&mut self) {
        let gain = self.gain;
        *self = Chain::new(self.rate, self.channels as u16, &self.settings);
        self.gain = gain;
    }

    /// New settings; every stage's history and delay are kept. Only a
    /// seek or an explicitly started stream resets the sample timeline.
    pub fn configure(&mut self, settings: &DspSettings) {
        if settings.preamp != self.settings.preamp || settings.bands != self.settings.bands {
            self.eq.configure(settings.preamp, &settings.bands);
        }
        if settings.amounts() != self.settings.amounts() {
            self.enhancer.configure(settings.amounts());
        }
        self.settings = settings.clone();
    }

    /// Fixed processing delay in the output format, including bypass.
    pub fn latency_frames(&self) -> usize {
        enhancer::LATENCY + self.limiter.latency_frames()
    }

    /// A track (or a crossfade into one) begins: its gain applies at once,
    /// not slid from the track's before; a gain arriving later slides.
    pub fn begin_track(&mut self, track_gain: f32) {
        self.gain = if self.settings.active() && self.settings.normalize {
            f64::from(track_gain)
        } else {
            1.0
        };
    }

    /// Processes interleaved `samples` of a track whose loudness gain is
    /// `track_gain` (1 when not known).
    pub fn process(&mut self, samples: &mut [f32], track_gain: f32) {
        let s = &self.settings;
        // Controls can still be fading out after their switches were
        // cleared. Keep protecting those boosted frames for this entire
        // block, even if the controls settle before its end.
        let limiting = s.limiter
            && (s.active() || self.gain > 1.0 || self.eq_mix > 0.0 || self.enhancer.active());
        let target = if s.normalize {
            f64::from(track_gain)
        } else {
            1.0
        };
        let eq_target = if s.eq { 1.0 } else { 0.0 };
        for frame in samples.chunks_exact_mut(self.channels) {
            if (self.gain - target).abs() > 1e-7 {
                self.gain += (target - self.gain) * self.slide;
            } else {
                self.gain = target;
            }
            for x in frame.iter_mut() {
                *x *= self.gain as f32;
            }
            if (self.eq_mix - eq_target).abs() > 1e-7 {
                self.eq_mix += (eq_target - self.eq_mix) * self.eq_slide;
            } else {
                self.eq_mix = eq_target;
            }
            self.eq_dry.copy_from_slice(frame);
            self.eq.process(frame);
            for (x, &dry) in frame.iter_mut().zip(&self.eq_dry) {
                *x = if self.eq_mix == 0.0 {
                    dry
                } else {
                    dry + self.eq_mix as f32 * (*x - dry)
                };
            }
        }
        // Also after it was turned off, while its amounts slide out; and
        // off, it still delays the sound by its `LATENCY`, so turning it
        // on or off does not shift it.
        self.enhancer.process(samples);
        self.channels_stage.process(samples, s.balance, s.mono);
        self.limiter.process_enabled(samples, limiting);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_gain_brings_to_the_target_within_the_peak_cap() {
        // A track at -10 LUFS is turned down 8 dB.
        assert!((biquad::gain_to_db(track_gain(-10.0, 1.0, true)) + 8.0).abs() < 0.01);
        // A quiet track at -30 LUFS with peaks at -6 dBFS: it wants +12 dB,
        // which would put its peaks at +6; the cap allows +2 dBFS, so +8.
        assert!((biquad::gain_to_db(track_gain(-30.0, 0.5, true)) - 8.02).abs() < 0.05);
        // Without the limiter, up to full scale: +6.
        assert!((biquad::gain_to_db(track_gain(-30.0, 0.5, false)) - 6.02).abs() < 0.05);
    }

    #[test]
    fn an_idle_chain_is_a_delayed_wire_and_a_gain_slides() {
        let mut c = Chain::new(48000, 2, &DspSettings::default());
        let latency = c.latency_frames();
        let mut s = vec![0.5; 2 * (latency + 100)];
        c.process(&mut s, 0.5);
        assert!(s[..2 * latency].iter().all(|&x| x == 0.0));
        assert!(s[2 * latency..].iter().all(|&x| x == 0.5));
        let mut c = Chain::new(
            48000,
            2,
            &DspSettings {
                normalize: true,
                ..Default::default()
            },
        );
        let mut s = vec![0.5; 2 * 48000];
        c.process(&mut s, 0.5);
        // Early on (past the limiter's 5 ms delay) the gain is still near
        // 1, after a second it is 0.5.
        let early = 2 * (c.latency_frames() + 20);
        assert!(s[early] > 0.49, "{}", s[early]);
        assert!((s[2 * 48000 - 1] - 0.25).abs() < 0.01);
    }

    /// The balance and mono work with the rest off, and do not bring the
    /// limiter in: a level over its ceiling passes.
    #[test]
    fn balance_and_mono_go_on_alone() {
        let settings = DspSettings {
            balance: -50,
            mono: true,
            ..Default::default()
        };
        assert!(!settings.active());
        let mut c = Chain::new(48000, 2, &settings);
        let latency = c.latency_frames();
        let mut s: Vec<f32> = (0..latency + 100).flat_map(|_| [1.0, 0.9]).collect();
        c.process(&mut s, 1.0);
        // The mean 0.95 on both, the right halved.
        assert!(
            (s[2 * latency] - 0.95).abs() < 1e-6 && (s[2 * latency + 1] - 0.475).abs() < 1e-6,
            "{:?}",
            &s[2 * latency..2 * latency + 2]
        );
    }

    #[test]
    fn zero_enhancement_and_limiter_switches_keep_the_sample_timeline() {
        let mut settings = DspSettings::default();
        let mut c = Chain::new(48000, 2, &settings);
        let latency = c.latency_frames();
        let original: Vec<f32> = (0..12000).map(|i| 0.2 * (i as f32 * 0.31).sin()).collect();
        let mut s = original.clone();
        s.extend(std::iter::repeat_n(0.0, latency * 2));
        for (i, part) in s.chunks_mut(200).enumerate() {
            settings.enhancer = i % 4 >= 2;
            settings.limiter = i % 3 != 0;
            c.configure(&settings);
            c.process(part, 1.0);
        }
        assert!(s[..latency * 2].iter().all(|&x| x == 0.0));
        assert_eq!(&s[latency * 2..], &original);
    }

    #[test]
    fn turning_processing_off_releases_gains_without_a_step() {
        let mut settings = DspSettings {
            eq: true,
            preamp: -12.0,
            normalize: true,
            enhancer: true,
            bass: 5,
            ..Default::default()
        };
        let mut c = Chain::new(48000, 2, &settings);
        let mut s = vec![0.1; 48000 * 4];
        c.process(&mut s[..48000], 0.5);
        settings.eq = false;
        settings.normalize = false;
        settings.enhancer = false;
        c.configure(&settings);
        c.process(&mut s[48000..], 0.5);
        let jump = s[47000..]
            .windows(3)
            .step_by(2)
            .map(|w| (w[2] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(jump < 0.0001, "{jump}");
        assert!((s[s.len() - 1] - 0.1).abs() < 0.001);
    }

    #[test]
    fn bypass_settles_to_exact_unity_at_high_sample_rates() {
        for rate in [48000, 192000] {
            let mut settings = DspSettings {
                normalize: true,
                eq: true,
                preamp: -6.0,
                enhancer: true,
                bass: 5,
                ..Default::default()
            };
            let mut c = Chain::new(rate, 2, &settings);
            c.begin_track(0.5);
            c.process(&mut vec![0.1; rate as usize], 0.5);
            settings.normalize = false;
            settings.eq = false;
            settings.enhancer = false;
            c.configure(&settings);
            let mut s = vec![0.1; 2 * rate as usize * 6];
            c.process(&mut s, 0.5);
            assert_eq!(c.gain, 1.0, "{rate}");
            assert_eq!(c.eq_mix, 0.0, "{rate}");
            assert!(s[s.len() - 1000..].iter().all(|&x| x == 0.1), "{rate}");
        }
    }

    #[test]
    fn normalization_fading_out_keeps_limiter_protection() {
        let mut settings = DspSettings {
            normalize: true,
            ..Default::default()
        };
        let mut c = Chain::new(48000, 2, &settings);
        c.begin_track(4.0);
        c.process(&mut vec![0.5; 48000], 4.0);
        settings.normalize = false;
        c.configure(&settings);
        // A single long block also must retain protection at its start.
        let mut s = vec![0.5; 48000 * 12];
        c.process(&mut s, 4.0);
        let peak = s.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(peak <= db_to_gain(CEILING_DB) + 1e-6, "{peak}");
        assert_eq!(s[s.len() - 1], 0.5);
    }

    /// The peak in dBFS of a stereo 1 kHz sine at `level` dBFS through a
    /// chain with the equalizer on, its preamp at `preamp` dB and the bands
    /// flat, past the limiter's start.
    fn through_preamp(level: f32, preamp: f32) -> f32 {
        through(level, preamp, true)
    }

    fn through(level: f32, preamp: f32, limiter: bool) -> f32 {
        let settings = DspSettings {
            eq: true,
            preamp,
            limiter,
            ..Default::default()
        };
        let mut c = Chain::new(48000, 2, &settings);
        let a = db_to_gain(level);
        let mut s: Vec<f32> = (0..48000)
            .flat_map(|i| {
                let x = a * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48000.0).sin();
                [x, x]
            })
            .collect();
        c.process(&mut s, 1.0);
        let peak = s[48000..].iter().fold(0f32, |m, x| m.max(x.abs()));
        biquad::gain_to_db(peak)
    }

    #[test]
    fn the_preamp_is_heard_within_the_ceiling() {
        // Down: always heard.
        assert!((through_preamp(-20.0, -6.0) + 26.0).abs() < 0.1);
        assert!((through_preamp(-3.0, -12.0) + 15.0).abs() < 0.1);
        // Up: heard as long as it stays under the limiter's ceiling...
        assert!((through_preamp(-20.0, 6.0) + 14.0).abs() < 0.1);
        // ...a loud track is held at it.
        assert!((through_preamp(-3.0, 12.0) - CEILING_DB).abs() < 0.2);
        // Without the limiter, raised past full scale (Windows clips it).
        assert!((through(-3.0, 12.0, false) - 9.0).abs() < 0.1);
    }
}
