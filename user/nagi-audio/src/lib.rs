#![no_std]

pub const MAX_SESSIONS: usize = 4;
pub const MAX_SESSION_SAMPLES: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioError {
    UnsupportedFormat,
    InvalidSession,
    InvalidVolume,
    BufferTooLarge,
    OutputTooSmall,
    NoSession,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcmFormat {
    sample_rate: u32,
    channels: u16,
}

impl PcmFormat {
    pub const fn stereo_48khz() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 2,
        }
    }

    pub const fn new(sample_rate: u32, channels: u16) -> Result<Self, AudioError> {
        if sample_rate == 48_000 && channels == 2 {
            Ok(Self {
                sample_rate,
                channels,
            })
        } else {
            Err(AudioError::UnsupportedFormat)
        }
    }

    pub const fn sample_rate(self) -> u32 {
        self.sample_rate
    }

    pub const fn channels(self) -> u16 {
        self.channels
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionToken {
    slot: u8,
    owner: u64,
}

impl SessionToken {
    pub const fn invalid() -> Self {
        Self {
            slot: u8::MAX,
            owner: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct Session {
    owner: u64,
    volume: u8,
    muted: bool,
    samples: [i16; MAX_SESSION_SAMPLES],
    sample_count: usize,
}

impl Session {
    const EMPTY: Self = Self {
        owner: 0,
        volume: 100,
        muted: false,
        samples: [0; MAX_SESSION_SAMPLES],
        sample_count: 0,
    };
}

pub struct Mixer {
    format: PcmFormat,
    sessions: [Option<Session>; MAX_SESSIONS],
    focus: Option<SessionToken>,
}

/// User-space audio service boundary. Applications use this bounded service
/// object; only the service owns the device capability and mixer state.
pub struct AudioService {
    capability: u64,
    mixer: Mixer,
}

impl AudioService {
    pub fn new(capability: u64) -> Self {
        Self {
            capability,
            mixer: Mixer::new(PcmFormat::stereo_48khz()),
        }
    }

    pub fn play(&self, stream_id: u32, data: &[u8]) -> bool {
        libnagi::audio_play(self.capability, u64::from(stream_id), data)
    }

    pub fn capture(&self, stream_id: u32, destination: &mut [u8]) -> usize {
        libnagi::audio_capture(self.capability, u64::from(stream_id), destination)
    }

    pub fn mixer_mut(&mut self) -> &mut Mixer {
        &mut self.mixer
    }
}

impl Mixer {
    pub const fn new(format: PcmFormat) -> Self {
        Self {
            format,
            sessions: [None; MAX_SESSIONS],
            focus: None,
        }
    }

    pub const fn format(&self) -> PcmFormat {
        self.format
    }

    pub fn open_session(&mut self, owner: u64) -> Result<SessionToken, AudioError> {
        for (slot, session) in self.sessions.iter_mut().enumerate() {
            if session.is_none() {
                let token = SessionToken {
                    slot: slot as u8,
                    owner,
                };
                *session = Some(Session {
                    owner,
                    ..Session::EMPTY
                });
                return Ok(token);
            }
        }
        Err(AudioError::NoSession)
    }

    pub fn set_volume(&mut self, token: SessionToken, volume: u8) -> Result<(), AudioError> {
        if volume > 100 {
            return Err(AudioError::InvalidVolume);
        }
        let session = self.session_mut(token)?;
        session.volume = volume;
        Ok(())
    }

    pub fn set_muted(&mut self, token: SessionToken, muted: bool) -> Result<(), AudioError> {
        self.session_mut(token)?.muted = muted;
        Ok(())
    }

    pub fn request_focus(&mut self, token: SessionToken) -> Result<(), AudioError> {
        self.session(token)?;
        self.focus = Some(token);
        Ok(())
    }

    pub fn release_focus(&mut self, token: SessionToken) -> Result<(), AudioError> {
        self.session(token)?;
        if self.focus == Some(token) {
            self.focus = None;
        }
        Ok(())
    }

    pub fn submit(&mut self, token: SessionToken, samples: &[i16]) -> Result<(), AudioError> {
        if samples.len() > MAX_SESSION_SAMPLES {
            return Err(AudioError::BufferTooLarge);
        }
        let session = self.session_mut(token)?;
        session.samples[..samples.len()].copy_from_slice(samples);
        session.sample_count = samples.len();
        Ok(())
    }

    pub fn mix_into(&mut self, output: &mut [i16]) -> Result<usize, AudioError> {
        if output.len() > MAX_SESSION_SAMPLES {
            return Err(AudioError::OutputTooSmall);
        }
        output.fill(0);
        for (slot, session) in self.sessions.iter_mut().enumerate() {
            let Some(session) = session.as_mut() else {
                continue;
            };
            let token = SessionToken {
                slot: slot as u8,
                owner: session.owner,
            };
            if self.focus.is_some_and(|focus| focus != token) || session.muted {
                session.sample_count = 0;
                continue;
            }
            let count = output.len().min(session.sample_count);
            for (index, mixed) in output.iter_mut().enumerate().take(count) {
                let scaled = i32::from(session.samples[index]) * i32::from(session.volume) / 100;
                let value = i32::from(*mixed) + scaled;
                *mixed = value.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
            }
            session.sample_count = 0;
        }
        Ok(output.len())
    }

    fn session(&self, token: SessionToken) -> Result<&Session, AudioError> {
        let Some(session) = self
            .sessions
            .get(usize::from(token.slot))
            .and_then(Option::as_ref)
        else {
            return Err(AudioError::InvalidSession);
        };
        if session.owner != token.owner {
            return Err(AudioError::InvalidSession);
        }
        Ok(session)
    }

    fn session_mut(&mut self, token: SessionToken) -> Result<&mut Session, AudioError> {
        let Some(session) = self
            .sessions
            .get_mut(usize::from(token.slot))
            .and_then(Option::as_mut)
        else {
            return Err(AudioError::InvalidSession);
        };
        if session.owner != token.owner {
            return Err(AudioError::InvalidSession);
        }
        Ok(session)
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioError, Mixer, PcmFormat};

    #[test]
    fn rejects_an_unsupported_pcm_format() {
        assert_eq!(PcmFormat::new(48_000, 2).unwrap().sample_rate(), 48_000);
        assert_eq!(
            PcmFormat::new(48_000, 1),
            Err(AudioError::UnsupportedFormat)
        );
        assert_eq!(
            PcmFormat::new(44_100, 2),
            Err(AudioError::UnsupportedFormat)
        );
    }

    #[test]
    fn mixes_two_sessions_with_independent_volume() {
        let mut mixer = Mixer::new(PcmFormat::stereo_48khz());
        let first = mixer.open_session(10).expect("first session");
        let second = mixer.open_session(20).expect("second session");
        mixer.set_volume(first, 50).expect("first volume");
        mixer
            .submit(first, &[1_000, -1_000])
            .expect("first samples");
        mixer
            .submit(second, &[2_000, -2_000])
            .expect("second samples");

        let mut output = [0_i16; 2];
        assert_eq!(mixer.mix_into(&mut output).expect("mixed output"), 2);
        assert_eq!(output, [2_500, -2_500]);
    }

    #[test]
    fn mute_and_focus_prevent_other_session_audio() {
        let mut mixer = Mixer::new(PcmFormat::stereo_48khz());
        let first = mixer.open_session(10).expect("first session");
        let second = mixer.open_session(20).expect("second session");
        mixer.set_muted(first, true).expect("mute first");
        mixer.request_focus(second).expect("focus second");
        mixer.submit(first, &[3_000, 3_000]).expect("first samples");
        mixer.submit(second, &[700, 700]).expect("second samples");

        let mut output = [0_i16; 2];
        mixer.mix_into(&mut output).expect("mixed output");
        assert_eq!(output, [700, 700]);
    }

    #[test]
    fn rejects_invalid_tokens_and_bounded_buffers() {
        let mut mixer = Mixer::new(PcmFormat::stereo_48khz());
        let session = mixer.open_session(10).expect("session");
        assert_eq!(
            mixer.set_volume(super::SessionToken::invalid(), 1),
            Err(AudioError::InvalidSession)
        );
        assert_eq!(
            mixer.submit(session, &[0; super::MAX_SESSION_SAMPLES + 1]),
            Err(AudioError::BufferTooLarge)
        );
        assert!(mixer.open_session(20).is_ok());
        assert!(mixer.open_session(30).is_ok());
        assert!(mixer.open_session(40).is_ok());
        assert!(mixer.open_session(50).is_err());
    }
}
