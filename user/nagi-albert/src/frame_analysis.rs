//! Bounded checks on rendered RGBA frames used by guest acceptance.

/// Count pixels whose color differs clearly from the frame's dominant
/// background. Text and other page content produce "ink"; a page that
/// rendered only its background color produces none.
///
/// The background is the most common color among a coarse sample of pixels.
/// A pixel counts as ink when its summed RGB distance from the background
/// exceeds `threshold`.
pub fn ink_pixels(frame: &[u8], width: u32, height: u32, threshold: u32) -> u32 {
    let pixels = (width as usize).saturating_mul(height as usize);
    if pixels == 0 || frame.len() < pixels * 4 {
        return 0;
    }
    let Some(background) = dominant_color(frame, pixels) else {
        return 0;
    };
    frame[..pixels * 4]
        .chunks_exact(4)
        .filter(|pixel| distance(pixel, background) > threshold)
        .count() as u32
}

fn distance(pixel: &[u8], color: [u8; 3]) -> u32 {
    (0..3)
        .map(|channel| u32::from(pixel[channel].abs_diff(color[channel])))
        .sum()
}

fn dominant_color(frame: &[u8], pixels: usize) -> Option<[u8; 3]> {
    const SAMPLES: usize = 256;
    let step = (pixels / SAMPLES).max(1);
    let mut colors: [([u8; 3], u32); SAMPLES] = [([0; 3], 0); SAMPLES];
    let mut used = 0;
    for index in (0..pixels).step_by(step).take(SAMPLES) {
        let offset = index * 4;
        let color = [frame[offset], frame[offset + 1], frame[offset + 2]];
        if let Some(entry) = colors[..used].iter_mut().find(|entry| entry.0 == color) {
            entry.1 += 1;
        } else if used < SAMPLES {
            colors[used] = (color, 1);
            used += 1;
        }
    }
    colors[..used]
        .iter()
        .max_by_key(|entry| entry.1)
        .map(|entry| entry.0)
}

#[cfg(test)]
mod tests {
    use super::ink_pixels;

    fn frame(width: u32, height: u32, background: [u8; 3]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..width * height {
            bytes.extend_from_slice(&[background[0], background[1], background[2], 255]);
        }
        bytes
    }

    #[test]
    fn a_background_only_frame_has_no_ink() {
        assert_eq!(
            ink_pixels(&frame(32, 16, [0xee, 0xee, 0xee]), 32, 16, 96),
            0
        );
    }

    #[test]
    fn dark_text_on_a_light_background_is_ink() {
        let mut bytes = frame(32, 16, [0xee, 0xee, 0xee]);
        for pixel in 40..70 {
            bytes[pixel * 4..pixel * 4 + 3].copy_from_slice(&[0x11, 0x11, 0x11]);
        }
        assert_eq!(ink_pixels(&bytes, 32, 16, 96), 30);
    }

    #[test]
    fn faint_antialiasing_noise_below_the_threshold_is_ignored() {
        let mut bytes = frame(8, 8, [200, 200, 200]);
        bytes[0..3].copy_from_slice(&[190, 190, 190]);
        assert_eq!(ink_pixels(&bytes, 8, 8, 96), 0);
    }

    #[test]
    fn short_or_empty_frames_report_no_ink() {
        assert_eq!(ink_pixels(&[], 0, 0, 96), 0);
        assert_eq!(ink_pixels(&[0; 8], 4, 4, 96), 0);
    }
}
