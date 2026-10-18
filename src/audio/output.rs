//! Where the samples go. [`Output`] is what the audio thread needs of a
//! device; [`Wasapi`] is the built-in one, a shared WASAPI stream on the
//! default device or the one chosen in the settings (qrec's
//! `playback::run`). An output plugin (`out_*.dll`, TODO.md) would
//! be another implementation. `list_devices` names the devices for the
//! settings window.

use windows::Win32::Media::Audio::{
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, DEVICE_STATE_ACTIVE, IAudioClient, IAudioClock,
    IAudioRenderClient, IAudioStreamVolume, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    WAVEFORMATEX, eConsole, eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, STGM_READ};
use windows::core::{HSTRING, Result};

/// A device that plays interleaved float samples of one format.
pub trait Output {
    /// Frames that can be written now without waiting.
    fn room(&mut self) -> Result<u32>;
    /// Writes `samples` (a whole number of frames, at most `room`).
    fn write(&mut self, samples: &[f32]) -> Result<()>;
    /// The gain (0 to 1) of what is heard from now on, not only of what is
    /// written next: a change must not wait for the device buffer. Called
    /// before every write; it is cheap when nothing changed.
    fn set_gain(&mut self, gain: f32) -> Result<()>;
    fn start(&mut self) -> Result<()>;
    /// Stops playing; what was written stays (pause).
    fn stop(&mut self) -> Result<()>;
    /// Drops what was written and not heard (stopped first); the count of
    /// heard frames starts again from 0.
    fn reset(&mut self) -> Result<()>;
    /// Frames heard since the start or the last reset.
    fn heard(&mut self) -> Result<u64>;
    /// Frames the device holds ahead of what is heard.
    fn buffer_frames(&self) -> u32;
    /// The device this plays on is no longer the one wanted: the default
    /// one changed, or the chosen one came back after the default one
    /// stood in for it.
    fn device_changed(&mut self) -> bool;
}

/// An output device as Windows lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// The endpoint's id, kept in the settings.
    pub id: String,
    /// What Windows calls it ("Speakers (Realtek(R) Audio)").
    pub name: String,
}

/// The active output devices, on a thread of their own with COM (the
/// caller's may have none); empty when they cannot be listed.
pub fn list_devices() -> Vec<Device> {
    std::thread::spawn(|| {
        let _com = super::mf::com_init_mta();
        devices().unwrap_or_else(|e| {
            log::warn!("cannot list the sound devices: {}", super::mf::describe(&e));
            Vec::new()
        })
    })
    .join()
    .unwrap_or_default()
}

fn devices() -> Result<Vec<Device>> {
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
    use windows::Win32::System::Variant::VT_LPWSTR;
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let collection = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)? };
    let count = unsafe { collection.GetCount()? };
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let Ok(device) = (unsafe { collection.Item(i) }) else {
            continue;
        };
        let id = device_id(&device);
        if id.is_empty() {
            continue;
        }
        let name = unsafe {
            device.OpenPropertyStore(STGM_READ).and_then(|store| {
                let mut value = store.GetValue(&PKEY_Device_FriendlyName)?;
                let inner = &value.Anonymous.Anonymous;
                let name = if inner.vt == VT_LPWSTR {
                    inner.Anonymous.pwszVal.to_string().unwrap_or_default()
                } else {
                    String::new()
                };
                let _ = PropVariantClear(&mut value);
                Ok(name)
            })
        }
        .unwrap_or_default();
        let name = if name.is_empty() { id.clone() } else { name };
        out.push(Device { id, name });
    }
    Ok(out)
}

const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
/// The device buffer, in 100 ns: half a second written ahead of what is
/// heard, so that the reading of tags and covers cannot starve it.
const BUFFER: i64 = 5_000_000;

pub struct Wasapi {
    client: IAudioClient,
    render: IAudioRenderClient,
    clock: IAudioClock,
    frequency: u64,
    size: u32,
    rate: u32,
    channels: usize,
    /// The id of the device this plays on, and of the one wanted (empty:
    /// the default).
    device: String,
    wanted: String,
    enumerator: IMMDeviceEnumerator,
    /// The stream's own volume, applied by the audio engine where it
    /// mixes, so within its 10 ms period; not the session's, which Windows
    /// shows in its mixer and remembers.
    volume: IAudioStreamVolume,
    /// What `volume` was last set to.
    gain: Option<f32>,
}

// The audio thread owns it; COM objects of the MTA may be used from it.
unsafe impl Send for Wasapi {}

impl Wasapi {
    /// A stream of `rate` and `channels` on the device `wanted` (its id;
    /// empty, or one not present: the default device); the audio engine
    /// converts it to the device's mix format.
    pub fn open(rate: u32, channels: u16, wanted: &str) -> Result<Wasapi> {
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        let chosen = (!wanted.is_empty())
            .then(|| unsafe { enumerator.GetDevice(&HSTRING::from(wanted)) }.ok())
            .flatten();
        if !wanted.is_empty() && chosen.is_none() {
            log::warn!("sound device {wanted} is not present: the default one plays");
        }
        let device = match chosen {
            Some(d) => d,
            None => unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole)? },
        };
        let id = device_id(&device);
        let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let format = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_IEEE_FLOAT,
            nChannels: channels,
            nSamplesPerSec: rate,
            nAvgBytesPerSec: rate * 4 * u32::from(channels),
            nBlockAlign: 4 * channels,
            wBitsPerSample: 32,
            cbSize: 0,
        };
        let flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
        unsafe { client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, BUFFER, 0, &format, None)? };
        let render: IAudioRenderClient = unsafe { client.GetService()? };
        let clock: IAudioClock = unsafe { client.GetService()? };
        let volume: IAudioStreamVolume = unsafe { client.GetService()? };
        let frequency = unsafe { clock.GetFrequency()? }.max(1);
        let size = unsafe { client.GetBufferSize()? };
        Ok(Wasapi {
            client,
            render,
            clock,
            frequency,
            size,
            rate,
            channels: usize::from(channels),
            device: id,
            wanted: wanted.to_owned(),
            enumerator,
            volume,
            gain: None,
        })
    }
}

fn device_id(device: &IMMDevice) -> String {
    unsafe {
        match device.GetId() {
            Ok(id) => {
                let text = id.to_string().unwrap_or_default();
                CoTaskMemFree(Some(id.0 as *const _));
                text
            }
            Err(_) => String::new(),
        }
    }
}

impl Output for Wasapi {
    fn room(&mut self) -> Result<u32> {
        let padding = unsafe { self.client.GetCurrentPadding()? };
        Ok(self.size.saturating_sub(padding))
    }

    fn write(&mut self, samples: &[f32]) -> Result<()> {
        let frames = (samples.len() / self.channels) as u32;
        if frames == 0 {
            return Ok(());
        }
        unsafe {
            let buffer = self.render.GetBuffer(frames)?;
            let out = std::slice::from_raw_parts_mut(
                buffer.cast::<f32>(),
                frames as usize * self.channels,
            );
            out.copy_from_slice(samples);
            self.render.ReleaseBuffer(frames, 0)?;
        }
        Ok(())
    }

    fn set_gain(&mut self, gain: f32) -> Result<()> {
        let gain = gain.clamp(0.0, 1.0);
        if self.gain == Some(gain) {
            return Ok(());
        }
        unsafe {
            let channels = self.volume.GetChannelCount()? as usize;
            self.volume.SetAllVolumes(&vec![gain; channels])?;
        }
        self.gain = Some(gain);
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        unsafe { self.client.Start() }
    }

    fn stop(&mut self) -> Result<()> {
        unsafe { self.client.Stop() }
    }

    fn reset(&mut self) -> Result<()> {
        unsafe {
            let _ = self.client.Stop();
            self.client.Reset()
        }
    }

    fn heard(&mut self) -> Result<u64> {
        let mut position = 0u64;
        unsafe { self.clock.GetPosition(&mut position, None)? };
        Ok((u128::from(position) * u128::from(self.rate) / u128::from(self.frequency)) as u64)
    }

    fn buffer_frames(&self) -> u32 {
        self.size
    }

    fn device_changed(&mut self) -> bool {
        if self.wanted.is_empty() {
            match unsafe { self.enumerator.GetDefaultAudioEndpoint(eRender, eConsole) } {
                Ok(device) => device_id(&device) != self.device,
                Err(_) => false,
            }
        } else if self.wanted == self.device {
            // The chosen device plays; its going away shows as an error.
            false
        } else {
            // The default stands in: back to the chosen one once it is
            // present again.
            unsafe {
                self.enumerator
                    .GetDevice(&HSTRING::from(self.wanted.as_str()))
            }
            .is_ok()
        }
    }
}

impl Drop for Wasapi {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}
