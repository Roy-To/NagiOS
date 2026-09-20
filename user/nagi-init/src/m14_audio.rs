use nagi_audio::{AudioError, AudioService, PcmFormat, MAX_SESSIONS, MAX_SESSION_SAMPLES};

const PCM_BYTES: usize = 4096;

const fn build_pcm() -> [u8; PCM_BYTES] {
    let mut pcm = [0_u8; PCM_BYTES];
    let mut frame = 0;
    while frame < PCM_BYTES / 4 {
        let sample: i16 = if frame % 64 < 32 { 4_000 } else { -4_000 };
        let raw = sample as u16;
        let offset = frame * 4;
        pcm[offset] = raw as u8;
        pcm[offset + 1] = (raw >> 8) as u8;
        pcm[offset + 2] = raw as u8;
        pcm[offset + 3] = (raw >> 8) as u8;
        frame += 1;
    }
    pcm
}

static PCM: [u8; PCM_BYTES] = build_pcm();

fn marker(message: &[u8]) {
    libnagi::console_write(message);
}

pub fn run(capability: u64) -> bool {
    let mut service = AudioService::new(capability);
    if !service.play(0, &PCM) {
        marker(b"Nagi M14 playback FAIL\r\n");
        return false;
    }
    marker(b"Nagi M14 playback PASS\r\n");

    let mut capture = [0_u8; PCM_BYTES];
    if service.capture(1, &mut capture) != PCM_BYTES {
        marker(b"Nagi M14 capture FAIL\r\n");
        return false;
    }
    if !capture.iter().any(|byte| *byte != 0) {
        marker(b"Nagi M14 capture signal FAIL\r\n");
        return false;
    }
    marker(b"Nagi M14 capture signal PASS\r\n");
    marker(b"Nagi M14 capture PASS\r\n");

    let mut denied_capture = [0_u8; 4];
    if libnagi::audio_play(capability ^ 1, 0, &PCM)
        || libnagi::audio_capture(capability ^ 1, 1, &mut denied_capture) != 0
    {
        marker(b"Nagi M14 capability denial FAIL\r\n");
        return false;
    }
    marker(b"Nagi M14 capability denial PASS\r\n");

    let format = PcmFormat::new(48_000, 2);
    if format != Ok(PcmFormat::stereo_48khz()) {
        marker(b"Nagi M14 mixer FAIL\r\n");
        return false;
    }
    let mixer = service.mixer_mut();
    let first = match mixer.open_session(0x10) {
        Ok(token) => token,
        Err(_) => return false,
    };
    let second = match mixer.open_session(0x20) {
        Ok(token) => token,
        Err(_) => return false,
    };
    if mixer.set_volume(first, 50).is_err()
        || mixer.submit(first, &[1_000; MAX_SESSION_SAMPLES]).is_err()
        || mixer.submit(second, &[2_000; MAX_SESSION_SAMPLES]).is_err()
    {
        marker(b"Nagi M14 volume/mute FAIL\r\n");
        return false;
    }
    let mut output = [0_i16; MAX_SESSION_SAMPLES];
    if mixer.mix_into(&mut output).is_err() || output[0] != 2_500 {
        marker(b"Nagi M14 mixer FAIL\r\n");
        return false;
    }
    if mixer.set_muted(first, true).is_err()
        || mixer.request_focus(second).is_err()
        || mixer.submit(first, &[3_000; MAX_SESSION_SAMPLES]).is_err()
        || mixer.submit(second, &[700; MAX_SESSION_SAMPLES]).is_err()
        || mixer.mix_into(&mut output).is_err()
        || output[0] != 700
        || mixer.release_focus(second).is_err()
    {
        marker(b"Nagi M14 volume/mute FAIL\r\n");
        return false;
    }
    marker(b"Nagi M14 mixer PASS\r\n");
    marker(b"Nagi M14 volume/mute PASS\r\n");

    let third = mixer.open_session(0x30);
    let fourth = mixer.open_session(0x40);
    let fifth = mixer.open_session(0x50);
    if !third.is_ok() || !fourth.is_ok() || fifth != Err(AudioError::NoSession) {
        marker(b"Nagi M14 sessions FAIL\r\n");
        return false;
    }
    if MAX_SESSIONS != 4 {
        marker(b"Nagi M14 sessions FAIL\r\n");
        return false;
    }
    marker(b"Nagi M14 sessions PASS\r\n");
    marker(b"Nagi M14 audio service PASS\r\n");
    true
}
