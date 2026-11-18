//! Activity of the incoming program, before any adaptive boost. All three
//! gain controls use the same decision, including when a shelf lifts hiss.

pub(super) const STEP: usize = 32;

#[derive(Clone, Copy)]
pub(super) enum Activity {
    Present,
    Hold,
    Quiet,
}

impl Activity {
    pub(super) fn target(self, wanted: f32, current: f32) -> f32 {
        match self {
            Self::Present => wanted,
            Self::Hold => current.min(wanted),
            Self::Quiet => 0.0,
        }
    }
}

pub(super) struct Detector {
    peak: f32,
    frames: usize,
    hold_steps: usize,
    left: usize,
    state: Activity,
}

impl Detector {
    pub(super) fn new(rate: u32) -> Self {
        Self {
            peak: 0.0,
            frames: 0,
            hold_steps: (rate as usize * 80).div_ceil(1000 * STEP),
            left: 0,
            state: Activity::Quiet,
        }
    }

    pub(super) fn observe(&mut self, frame: &[f32]) -> Activity {
        // A 5.1 LFE alone should not open the gate on the other channels.
        for (ch, &x) in frame.iter().enumerate() {
            if frame.len() != 6 || ch != 3 {
                self.peak = self.peak.max(x.abs());
            }
        }
        self.frames += 1;
        if self.frames == STEP {
            // Open at -57 dBFS peak and stay open down to -63 dBFS.
            let threshold = if self.left > 0 { 0.000708 } else { 0.001413 };
            self.state = if self.peak >= threshold {
                self.left = self.hold_steps;
                Activity::Present
            } else if self.left > 0 {
                self.left -= 1;
                Activity::Hold
            } else {
                Activity::Quiet
            };
            self.peak = 0.0;
            self.frames = 0;
        }
        self.state
    }
}
