//! The sound processing between the decoder and the device, on the audio
//! thread, a block at a time: the track's loudness gain (`normalize`),
//! the ten-band equalizer, the enhancer, and a look-ahead limiter after
//! them whenever any of them is on, so nothing clips. `DspSettings` is
//! what the settings window edits (`settings.dsp`); the engine gets a
//! copy with `Cmd::Dsp` and keeps the filters' state across changes.

pub mod biquad;
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
    pub width: u8,
    pub warmth: u8,
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
            width: 0,
            warmth: 0,
        }
    }
}

impl DspSettings {
    /// Anything to do at all.
    pub fn active(&self) -> bool {
        self.normalize || self.eq || self.enhancer
    }

    fn amounts(&self) -> enhancer::Amounts {
        let f = |v: u8| f32::from(v.min(10)) / 10.0;
        enhancer::Amounts {
            bass: f(self.bass),
            clarity: f(self.clarity),
            width: f(self.width),
            warmth: f(self.warmth),
        }
    }
}

/// The gain that brings a track measured at `lufs` with a sample peak of
/// `peak` (1 = full scale) to `TARGET_LUFS`, capped so that its peaks
/// stay within `PEAK_HEADROOM_DB` of the limiter's ceiling.
pub fn track_gain(lufs: f32, peak: f32) -> f32 {
    let wanted = TARGET_LUFS - lufs;
    let cap = CEILING_DB + PEAK_HEADROOM_DB - biquad::gain_to_db(peak.max(1e-4));
    db_to_gain(wanted.min(cap))
}

pub struct Chain {
    settings: DspSettings,
    rate: u32,
    channels: usize,
    eq: eq::Equalizer,
    enhancer: enhancer::Enhancer,
    limiter: limiter::Limiter,
    /// The track gain applied now; it slides to the one asked for.
    gain: f32,
    slide: f32,
}

impl Chain {
    pub fn new(rate: u32, channels: u16, settings: &DspSettings) -> Chain {
        let channels = usize::from(channels.max(1));
        Chain {
            settings: settings.clone(),
            rate,
            channels,
            eq: eq::Equalizer::new(rate, channels, settings.preamp, &settings.bands),
            enhancer: enhancer::Enhancer::new(rate, channels, settings.amounts()),
            limiter: limiter::Limiter::new(rate, channels, db_to_gain(CEILING_DB)),
            gain: 1.0,
            // A gain change takes about 300 ms.
            slide: 1.0 - (-1.0 / (rate as f32 * 0.3)).exp(),
        }
    }

    /// The filters' and the limiter's state dropped, the gain kept: what
    /// they hold is the sound before (another track, a seek).
    pub fn reset(&mut self) {
        let gain = self.gain;
        *self = Chain::new(self.rate, self.channels as u16, &self.settings);
        self.gain = gain;
    }

    /// New settings; the filters' state is kept, but for a chain turned on
    /// from a wire, whose state is of the sound long before.
    pub fn configure(&mut self, settings: &DspSettings) {
        if !self.settings.active() && settings.active() {
            self.settings = settings.clone();
            self.reset();
            return;
        }
        if settings.preamp != self.settings.preamp || settings.bands != self.settings.bands {
            self.eq.configure(settings.preamp, &settings.bands);
        }
        if settings.amounts() != self.settings.amounts() {
            self.enhancer.configure(settings.amounts());
        }
        self.settings = settings.clone();
    }

    /// Processes interleaved `samples` of a track whose loudness gain is
    /// `track_gain` (1 when not known).
    pub fn process(&mut self, samples: &mut [f32], track_gain: f32) {
        let s = &self.settings;
        if !s.active() {
            return;
        }
        if s.normalize {
            let target = track_gain;
            for frame in samples.chunks_exact_mut(self.channels) {
                if (self.gain - target).abs() > 1e-4 {
                    self.gain += (target - self.gain) * self.slide;
                } else {
                    self.gain = target;
                }
                for x in frame {
                    *x *= self.gain;
                }
            }
        } else {
            self.gain = 1.0;
        }
        if s.eq {
            self.eq.process(samples);
        }
        if s.enhancer {
            self.enhancer.process(samples);
        }
        self.limiter.process(samples);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_gain_brings_to_the_target_within_the_peak_cap() {
        // A track at -10 LUFS is turned down 8 dB.
        assert!((biquad::gain_to_db(track_gain(-10.0, 1.0)) + 8.0).abs() < 0.01);
        // A quiet track at -30 LUFS with peaks at -6 dBFS: it wants +12 dB,
        // which would put its peaks at +6; the cap allows +2 dBFS, so +8.
        assert!((biquad::gain_to_db(track_gain(-30.0, 0.5)) - 8.02).abs() < 0.05);
    }

    #[test]
    fn an_idle_chain_is_a_wire_and_a_gain_slides() {
        let mut c = Chain::new(48000, 2, &DspSettings::default());
        let mut s = vec![0.5; 200];
        c.process(&mut s, 0.5);
        assert!(s.iter().all(|&x| x == 0.5));
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
        assert!(s[2 * 300] > 0.49, "{}", s[2 * 300]);
        assert!((s[2 * 48000 - 1] - 0.25).abs() < 0.01);
    }

    /// The peak in dBFS of a stereo 1 kHz sine at `level` dBFS through a
    /// chain with the equalizer on, its preamp at `preamp` dB and the bands
    /// flat, past the limiter's start.
    fn through_preamp(level: f32, preamp: f32) -> f32 {
        let settings = DspSettings {
            eq: true,
            preamp,
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
    }
}
