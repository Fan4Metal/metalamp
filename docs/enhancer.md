<p align="center"><b>English</b> | <a href="enhancer.ru.md">Русский</a></p>

# The enhancer

MetalAMP's enhancer (Settings → Sound, and the Sound tuning window) is a multiband automatic gain control modelled on recordings of a well-known enhancer that works as a Winamp output plugin. The model was built by measurement alone: the original was recorded on a test rig with its controls set one at a time, and a parametric model was fitted to the recordings. Its code was not taken apart. This document describes how the enhancer works, how it was measured and fitted, how close it comes to the original and what remains.

[README](../README.md) describes the controls from a listener's side.

## Contents

- [Place in the sound chain](#place-in-the-sound-chain)
- [How it works](#how-it-works)
- [The controls](#the-controls)
- [Methodology](#methodology)
- [Current state](#current-state)
- [Known differences and further work](#known-differences-and-further-work)
- [Code and tests](#code-and-tests)

## Place in the sound chain

```
decoder → loudness gain → equalizer → enhancer → balance and mono → limiter → WASAPI
```

The enhancer runs on the audio thread for each block written to the device. Its output is lowered by a fixed 6 dB (`OUTPUT_TRIM_DB`) so that its peaks rarely pass full scale; the reasons for this value are given under [Output level and peaks](#output-level-and-peaks). While the enhancer is on, the limiter after it has a switch of its own, off by default.

## How it works

### Analysis and synthesis

- **Short-time spectrum.** The sound is analysed with a Hann window of 1024 samples at 44.1 and 48 kHz (the size scales with the sample rate, from 128 to 8192), with a hop of a quarter window, about 5.8 ms at 44.1 kHz.
- **Real gains.** Each frequency bin gets a real gain, so the phase is kept. The original showed no phase change either: against the bypassed path its group delay was 0.00 ms from 125 Hz to 16 kHz.
- **Dry path plus difference.** The dry sound is delayed by the window. Only the difference made by the gain (gain − 1) is synthesised and overlap-added onto it. With the enhancer off, the output is therefore the input delayed, sample for sample. When the enhancer is switched off, its last windows drain into that exact path.
- **Channels.** The stereo channels share one set of gains, so the stereo image is untouched; the original likewise left mid and side unchanged. In 5.1 the LFE channel is neither analysed nor processed.
- **Delay.** The enhancer's delay is one window, 23 ms at 44.1 kHz. The whole chain, the limiter included, keeps about 29 ms at 44.1 kHz and 27 ms at 48 kHz, whether the effects are on or off. The player's clock subtracts it.

### Six detectors

The original is a single gain control with six detectors. This was found by playing quiet pink noise at −45 dBFS and, in turn, one loud octave at −20 dBFS, then seeing which frequencies lost gain. Each detector hears a weighted part of the spectrum (`EDGES`, `SENSITIVITY`):

| Detector | Hears | Acts on |
|---|---|---|
| 1 | bass to about 90 Hz | 25–99 Hz |
| 2 | 90–200 Hz | 62–157 Hz |
| 3 | 250 Hz – 1 kHz | itself |
| 4 | 1–8 kHz | 1.6–4 kHz |
| 5 | 4–16 kHz | 6–10 kHz |
| 6 | from 16 kHz | from 16 kHz |

A bin's gain comes from the two detectors its frequency lies between, blended in amplitude (`INDEX`).

### Three stages

All three stages work on the same six detectors. They follow one another, and each stage's detectors hear the sound with the stages before it applied:

1. **Core.** For each detector the target gain is `offset − slope × level`, held between a floor and a most. It sits over a base curve in 30 third-octave bands from 25 Hz to 20 kHz.
2. **Fidelity.** An upward stage: from 0 up to a most that grows with the control, falling as the level rises.
3. **Ambience.** The same form as Fidelity, with its own thresholds, slopes and curve.

Because the later stages hear what the earlier ones did, Fidelity and Ambience together raise less than the sum of each, as recorded.

- **Envelopes.** The detectors average power over 54 ms. A gain follows its target with an attack of 2 ms and a release of 21 ms, all applied per hop. The effective timing comes mostly from the power average.
- **Start.** A stage starts at its most, as after silence. This applies on each new chain (a seek, another track) and when the enhancer is switched on. The original behaved so after a seek; started from a mid level, the model came 5–11 dB short on the first hit.
- **Levels.** A fixed three-band equalizer comes after the stages and is linear in dB:
  - Bass: under about 125 Hz;
  - Mid: 315–800 Hz, plus half of 1.6–5 kHz;
  - Treble: the other half of 1.6–5 kHz, plus everything from 12.5 kHz.

### The gain of a bin

In dB, a bin's gain is:

```
d × (base curve + Σ stage gains) + parts the drive does not scale + levels − 6 dB
```

Here `d` is the Drive (0 to 1.4).

- **The drive scales the whole gain linearly.** The detectors hear the earlier stages at the drive the law was recorded at (`HEAR_DRIVE`, 1.35), whatever the drive set. The original behaves so: in quiet passages, Drive 35, 70 and 105 kept 26, 52 and 78 % of the gain at 135. When the detectors followed the drive, the later stages made up part of the loss and the line bent.
- **A part the drive does not scale.** At any drive above 0, some of the gain stays in place. In the original, loud passages above 1 kHz lost more at Drive 35 than at Drive 0, which a pure product cannot do. The model keeps fitted shares of the stages' gain at their full value (`UNSCALED_*`):
  - for the core, a share of its compression per detector, 0.3–0.6 above 1 kHz;
  - for Fidelity and Ambience, a share of the whole stage.

  These shares fade in from 0 at Drive 0 to full at Drive 10 %, since at Drive 0 the original leaves only the levels.
- **Glide.** When a control moves, the base curve and the drive glide a quarter of the way each hop, about 25 ms, so the change does not click.

## The controls

| Control | Range | What it changes in the model |
|---|---|---|
| Drive | 0–140 % | multiplies the stages and the base curve in dB |
| Bass type | Deep 100 … Punch 100 | the core's most per detector and its base curve, by five knots at −1, −0.5, 0, 0.5, 1 |
| Treble type | Sharp 100 … Smooth 100 | the same, by its own knots |
| Fidelity | 0–10 | its stage's most and curve, by knots at 0, 0.25 … 1 |
| Ambience | 0–10 | the same for its stage |
| Bass, Mid, Treble | ±6 dB | the fixed equalizer after the stages |

Between knots the values are interpolated linearly. The controls change the parameters of the one gain control and add no stages of their own, as the recordings showed.

The built-in presets were read off the original's window: Neutral (its Default), Loud (the user's own preset in the original, the one recorded most, and the default here), Acoustic, Classical, Electronic, Jazz, New Age, Pop, Reggae and Rock.

## Methodology

### The rig

The original was measured in its own player through a virtual audio cable (VB-Audio). A Python rig drove the whole process without a person at the keyboard:

- **Automation.** The rig wrote each preset into the plugin's stored settings, started the player minimised with a test file, recorded the cable's output and closed the player. The plugin reads its settings when the player starts, and the player writes them back when it exits, so the order matters. Each batch ends by restoring the user's own settings.
- **One capture per batch.** On the measurement machine the antivirus asks about every new capture stream, so a whole batch is recorded in one stream and cut into takes.
- **Alignment.** Each take is aligned to its stimulus by cross-correlation with a marker at its start.
- **The bypassed path.** The plugin switched off is a flat path, the same to 0.01 dB. With all blocks off it is exactly −7.31 dB (the player's volume and the cable), with a further −1.6 dB above 8 kHz from the plugin's 44.1 → 48 kHz resampler. Every take is compared with this path, so the player and the cable cancel out.
- **Repeatability.** Two runs of one preset agree to 0.01 dB; two music takes of the same settings, to 0.00 dB.

### Stimuli

| Stimulus | What it measured |
|---|---|
| sine sweep, impulse | phase and delay (none) |
| pink noise in steps from −48 to −6 dBFS | the static gain against the level, per band |
| long steps | attack and release (the original settles within 30–80 ms; release about 0.1 s for the core and 0.2 s for Fidelity and Ambience) |
| quiet pink noise with one loud octave in turn | what each detector hears and where it acts |
| L, R, L = −R, uncorrelated noise | the stereo image (untouched, Ambience included) |
| six music passages of 12 s in different genres | the reference for the final fit and every check |

### Sweeps of the controls

The rig recorded each control on its own and in some combinations, about 30 presets in all:

- Drive 0, 35, 70, 105, 135, 140;
- the types at −100, −50, 0, 50, 100;
- the levels at −6, 0, +6;
- Fidelity and Ambience at 0, 2.5, 5, 7.5, 10.

On music, the rig recorded 26 takes from Loud with one control moved at a time, the presets Neutral, Rock, Acoustic and Electronic, and the Loud settings at two player volumes.

What the sweeps showed:

- **Drive** scales the gain curve linearly in dB (about 10.3 dB per unit at 100 Hz) and keeps its shape against the level.
- **The levels** are static shelves that add in dB, and they work at Drive 0 too.
- **The types** are not mirror images. Deep raises the low bass the more, the quieter it is; Punch raises 250 Hz – 1 kHz. Sharp raises 5–12 kHz; Smooth raises the whole top statically and lowers the middle a little.
- **Fidelity and Ambience** are upward gain controls, flat below a threshold and about 2:1 above it. Their most grows almost linearly with the control and with the drive.
- **The "style" setting** of the original changes only the look of its window.

### The fit

A Python port of the model (numpy, with numba for the envelopes) was fitted to the recordings. The port is checked against the Rust code with `--bench render`, which renders a passage through MetalAMP's own chain. The law's numbers are then generated into `enhancer_law.rs`.

1. **On test stimuli.** The first fit used 85 ms frames in third octaves of the quiet stimuli, all presets at once. The detectors' parameters were shared, and the controls moved the most and the base by knots. One gain control with corrections from Fidelity and Ambience could not reproduce their combination; three stages, each hearing the ones before it, could. The fit agreed within 1–1.2 dB on the stimuli. On music, however, some knots had become degenerate, a base and a most that cancel each other. Bass type +100 together with Fidelity and Ambience raised 150 Hz – 3 kHz by 10 dB on music, where the original raised it by 0.4.
2. **On music.** The whole law, about 650 parameters, was refitted by least squares on 21 ms band envelopes of the rig's music, at their own level.
   - **Data:** 28 takes of 6 passages.
   - **Regulariser:** the stimulus frames, so that the law stays right on quiet sound.
   - **Penalties:** against degenerate knots. A knot's base is pulled to 0, the core's most is kept above its threshold, and an upward stage's most is capped at 25 dB.
   - **Held out:** Rock, Acoustic and Electronic were left out of the fit and serve as a check.
   - **Computation:** the Jacobian is worked out by finite differences in 16 processes.
3. **Drive and its unscaled part.** Comparing quiet and loud moments at Drive 35, 70 and 105 led to two later changes:
   - detectors that hear the earlier stages at a fixed drive;
   - a part of the gain that the drive does not scale.

   Both were refitted the same way.

**The metric.** The model's and the original's envelopes are compared in five bands at 20–21 ms, with the overall level difference taken out by the median, and the result is the mean absolute difference in dB. The rig itself repeats to 0.00 dB. Two independent recordings of the same passage, made in an earlier session at other settings, differed by 0.37 dB on average. These older recordings also differ from the rig's own by about 0.67 dB, so only the rig's recordings serve as the reference.

### Output level and peaks

- **What the original does.** Its peaks reach +3.71 dBFS, where it hard-clips its own output (flat tops of up to 7 samples), before the player's volume. That volume is linear: 167 of 255 is −3.7 dB, which puts the ceiling exactly at 0 dBFS. In the user's setup the original therefore clips 0.007–0.086 % of samples on the rig's music, inaudibly.
- **What MetalAMP does.** It has no volume stage after the enhancer that would do this, so the enhancer leaves room for its peaks itself. The trim was chosen by rendering the rig's music through the chain and measuring two things on rock: how often the limiter worked, and how much it moved the bass's gain (15–150 Hz).

  | Trim | Limiter at work | Average reduction | Bass gain movement |
  |---|---|---|---|
  | −2 dB | 94–100 % of the time | 2.4–3.1 dB | heard as a gurgle |
  | −3.7 dB (as loud as the original at 167) | 76–95 % | 1.2–1.9 dB | twice the original's on a quieter song |
  | −5 dB | 28–64 % | 0.5–0.9 dB | |
  | −6 dB | 8–21 % | 0.2–0.3 dB | less than the original's |

  At −6 dB without the limiter, 0.002–0.065 % of samples pass full scale and are clipped by Windows, fewer than the original clips of its own output. For this reason the limiter is off by default while the enhancer is on.
- **The cost.** At 100 % volume, MetalAMP with the enhancer is 2.3 dB quieter than the original at volume 167.

## Current state

The figures below are measured on the rig's recordings, using the metric described above.

| Measure | Result |
|---|---|
| music envelopes, presets in the fit | 0.62 dB on average (Loud 0.59) |
| music envelopes, presets held out | Rock 0.58, Acoustic 0.62, Electronic 0.75 dB |
| each control's change of the five bands | follows the original's within about 0.5 dB |
| quiet stimulus frames, all recorded presets | 1.14 dB on average, 1.64 at worst |
| music envelopes at Drive 35 / 70 / 105 | 0.46 / 0.45 / 0.49 dB |
| overall level against the original before its volume | Loud +0.06, Neutral +0.15, Rock −0.25, Acoustic −0.05, Electronic −1.05 dB |
| phase, stereo image | unchanged, as in the original |
| bypass | sample-exact delayed path |

The model is about as close to the original as two recordings of the same passage are to each other. In listening, the enhancer at the Loud settings sounds like the original, and the bass no longer gurgles.

## Known differences and further work

1. **Attacks at low drive.** The largest remaining differences are on hits. At Drive 35, loud moments at 3–8 kHz are lowered by 3.3 dB in the model and by 5.0 dB in the original. The likely cause is the detectors' timing: the original's attack settles over 30–80 ms, while the model has one attack and release for all detectors. Matching it would also lower the model's transient peaks, which would allow a trim closer to −3.7 dB and the original's loudness.
2. **Smaller differences:**
   - Ambience 10 raises the middle by about 1.1 dB, the original by 0.4–0.9 dB;
   - Bass type −100 raises loud bass by about 4.5 dB, the original by 3.4 dB;
   - Treble type 0 takes 2.4 dB off above 8 kHz, the original 1.6 dB;
   - the Electronic preset is 1 dB quieter overall than the original.
3. **Not covered by the music recordings:**
   - combinations of several controls moved away from Loud at once (the stimulus sweeps hold some);
   - the presets Classical, Jazz, New Age, Pop and Reggae.
4. **Sample rate.** The original resamples to 48 kHz before it processes. The model works at the stream's own rate with frequencies in Hz, so the rate does not shift its bands. It was fitted at 48 kHz.

The rig, the recordings and the fitting scripts are kept outside the repository: the recordings hold commercial music.

## Code and tests

- `src/audio/dsp/enhancer.rs`: the enhancer, its controls (`Enhancement`), the presets, the detectors' layout and the law's structure (`law_of`, `synthesize`).
- `src/audio/dsp/enhancer_law.rs`: the fitted numbers (the third-octave centres, the stages, the knots, the levels' curves, the times, the unscaled shares). The file is generated by the fit and regenerated rather than edited.
- `src/audio/dsp/mod.rs`: the chain, the trim's place, the limiter's switch (`DspSettings::limiter_enabled`).
- `metalamp.exe --bench render <file> <out.wav> <start> <seconds> [gain] [raw]` renders a passage through the chain with the settings' sound as a float WAV, with the chain's delay taken out, for comparison with a recording of another player. `raw` leaves the limiter out.
- Tests in `enhancer.rs` and `mod.rs`:
  - quiet sound is raised more than loud;
  - Drive 0 leaves only the levels;
  - the drive scales the gain;
  - Fidelity and Ambience raise quiet sound;
  - the stages start at their most;
  - a changed control glides;
  - switching off drains to an exact path;
  - settings out of range are held in;
  - the presets are within range;
  - the limiter's switch while the enhancer is on;
  - the chain at every sample rate from 8 to 384 kHz and every channel count from mono to 5.1.
