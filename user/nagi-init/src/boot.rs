use libnagi::boot::{BootMode, BootPhase, BootProgressBridge, BootStage};
use libnagi::{
    display_info, display_present, DisplayInfo, PIXEL_FORMAT_RGBA8888, SURFACE_BYTES,
    SURFACE_HEIGHT, SURFACE_WIDTH,
};

use crate::ui::rgba;

const BACKGROUND: u32 = rgba(3, 8, 19);
const FOREGROUND: u32 = rgba(220, 236, 255);
const CYAN: u32 = rgba(125, 213, 255);
const LAVENDER: u32 = rgba(143, 162, 255);

const CENTER_X: i32 = SURFACE_WIDTH as i32 / 2;
const RING_CENTER_Y: i32 = 73;
const COLLAPSE_FRAME_COUNT: u32 = 4;
const DIGIT_SCALE: i32 = 2;
const DIGIT_CELL_WIDTH: i32 = 12;

const CIRCLE_POINTS: [(i16, i16); 32] = [
    (0, -1000),
    (195, -981),
    (383, -924),
    (556, -831),
    (707, -707),
    (831, -556),
    (924, -383),
    (981, -195),
    (1000, 0),
    (981, 195),
    (924, 383),
    (831, 556),
    (707, 707),
    (556, 831),
    (383, 924),
    (195, 981),
    (0, 1000),
    (-195, 981),
    (-383, 924),
    (-556, 831),
    (-707, 707),
    (-831, 556),
    (-924, 383),
    (-981, 195),
    (-1000, 0),
    (-981, -195),
    (-924, -383),
    (-831, -556),
    (-707, -707),
    (-556, -831),
    (-383, -924),
    (-195, -981),
];

const DIGIT_GLYPHS: [[u8; 7]; 10] = [
    [0x1c, 0x22, 0x26, 0x2a, 0x32, 0x22, 0x1c],
    [0x08, 0x18, 0x08, 0x08, 0x08, 0x08, 0x1c],
    [0x1c, 0x22, 0x02, 0x04, 0x08, 0x10, 0x3e],
    [0x3c, 0x02, 0x02, 0x1c, 0x02, 0x02, 0x3c],
    [0x04, 0x0c, 0x14, 0x24, 0x3e, 0x04, 0x04],
    [0x3e, 0x20, 0x20, 0x3c, 0x02, 0x02, 0x3c],
    [0x1c, 0x20, 0x20, 0x3c, 0x22, 0x22, 0x1c],
    [0x3e, 0x02, 0x04, 0x08, 0x10, 0x10, 0x10],
    [0x1c, 0x22, 0x22, 0x1c, 0x22, 0x22, 0x1c],
    [0x1c, 0x22, 0x22, 0x1e, 0x02, 0x02, 0x1c],
];

// This is a bounded polygonal approximation of the supplied formal mark.
// Its points preserve the source SVG's 36..178 by 23..81 proportions.
const MARK_PATH: &[(i32, i32)] = &[
    (36, 50),
    (43, 58),
    (51, 66),
    (58, 51),
    (65, 23),
    (76, 22),
    (86, 23),
    (96, 35),
    (105, 57),
    (114, 67),
    (124, 68),
    (134, 67),
    (145, 61),
    (156, 50),
    (166, 42),
    (178, 37),
    (173, 49),
    (169, 59),
    (163, 67),
    (156, 75),
    (149, 77),
    (141, 77),
    (132, 78),
    (123, 77),
    (115, 81),
    (111, 70),
    (107, 57),
    (102, 45),
    (97, 37),
    (92, 35),
    (88, 36),
    (82, 45),
    (77, 58),
    (72, 70),
    (69, 81),
    (62, 82),
    (56, 80),
    (50, 78),
    (44, 76),
    (39, 73),
    (35, 68),
    (33, 63),
    (36, 50),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootError {
    DisplayInfo,
    Surface,
    Present,
    InvalidState,
}

pub struct BootScreen {
    display_capability: u64,
    surface_address: u64,
    bridge: BootProgressBridge,
}

impl BootScreen {
    pub fn new(display_capability: u64) -> Result<Self, BootError> {
        let mut info = DisplayInfo::default();
        if !display_info(&mut info) {
            return Err(BootError::DisplayInfo);
        }
        if info.surface_address == 0
            || info.surface_bytes as usize != SURFACE_BYTES
            || info.width != SURFACE_WIDTH
            || info.height != SURFACE_HEIGHT
            || info.stride != SURFACE_WIDTH * core::mem::size_of::<u32>() as u32
            || info.pixel_format != PIXEL_FORMAT_RGBA8888
            || !(info.surface_address as usize).is_multiple_of(core::mem::align_of::<u32>())
        {
            return Err(BootError::Surface);
        }
        Ok(Self {
            display_capability,
            surface_address: info.surface_address,
            bridge: BootProgressBridge::new(BootMode::External),
        })
    }

    pub fn present_stage(&mut self, stage: BootStage) -> Result<u64, BootError> {
        if !self.bridge.advance(stage) {
            return Err(BootError::InvalidState);
        }
        let frame_count = if self.bridge.state().reduced_motion() {
            1
        } else {
            4
        };
        for frame in 0..frame_count {
            self.render_boot_frame(frame, frame_count)?;
            self.present()?;
            delay_frame(frame_count);
        }
        self.log_stage(stage, self.bridge.state().progress());
        Ok(self.checksum())
    }

    pub fn finish_to_desktop(&mut self) -> Result<(u64, u64), BootError> {
        if !self.bridge.mark_lock_ready() {
            return Err(BootError::InvalidState);
        }
        self.render_boot_frame(0, 1)?;
        self.present()?;
        let boot_checksum = self.checksum();
        if !self.bridge.complete() {
            return Err(BootError::InvalidState);
        }
        self.render_collapse_frames()?;
        self.render_lock_frame();
        self.present()?;
        let lock_checksum = self.checksum();
        self.log_lock_transition(boot_checksum, lock_checksum);
        Ok((boot_checksum, lock_checksum))
    }

    pub fn present_failure(&mut self) -> Result<u64, BootError> {
        self.bridge.fail();
        self.render_failure_frame()?;
        self.present()?;
        let checksum = self.checksum();
        libnagi::console_write(b"Nagi boot failure PRESENTED\r\n");
        Ok(checksum)
    }

    fn render_boot_frame(&mut self, frame: u32, frame_count: u32) -> Result<(), BootError> {
        let state = self.bridge.state();
        let progress = state.progress();
        let surface = self.surface_mut()?;
        clear_surface(surface, BACKGROUND);

        let stage_shrink = i32::from(progress / 20);
        let frame_shift = if frame_count > 1 && frame & 1 == 1 {
            1
        } else {
            0
        };
        let ring_alpha = (72 + u16::from(progress) * 120 / 100).min(208) as u8;
        draw_ring_set(
            surface,
            CENTER_X,
            RING_CENTER_Y,
            stage_shrink + frame_shift,
            ring_alpha,
        );
        draw_progress_arc(
            surface,
            CENTER_X,
            RING_CENTER_Y,
            62 - stage_shrink,
            progress,
            frame,
            frame_count,
        );
        draw_formal_mark(surface, 0, 20, FOREGROUND);
        draw_wordmark(surface, 142, 137, FOREGROUND);
        draw_percentage(surface, 144, 151, progress, FOREGROUND);
        draw_centered_text(surface, 178, phase_label(state.phase()), FOREGROUND);
        Ok(())
    }

    fn render_collapse_frames(&mut self) -> Result<(), BootError> {
        let mut frame = 0;
        while frame < COLLAPSE_FRAME_COUNT {
            let surface = self.surface_mut()?;
            clear_surface(surface, BACKGROUND);
            let shrink = frame as i32 * 10;
            let alpha = 190_u8.saturating_sub(frame as u8 * 42);
            draw_ring_set(surface, CENTER_X, RING_CENTER_Y, shrink, alpha);
            draw_formal_mark(surface, 0, 20, blend(BACKGROUND, FOREGROUND, alpha));
            draw_wordmark(surface, 142, 137, blend(BACKGROUND, FOREGROUND, alpha));
            self.present()?;
            delay_frame(COLLAPSE_FRAME_COUNT);
            frame += 1;
        }
        Ok(())
    }

    fn render_lock_frame(&mut self) {
        let surface = self
            .surface_mut()
            .expect("validated display surface must remain available");
        clear_surface(surface, BACKGROUND);
        draw_formal_mark(surface, 0, 28, FOREGROUND);
        draw_wordmark(surface, 142, 145, FOREGROUND);
        draw_centered_text(surface, 177, b"LOCK STATE READY", CYAN);
    }

    fn render_failure_frame(&mut self) -> Result<(), BootError> {
        let surface = self.surface_mut()?;
        clear_surface(surface, BACKGROUND);
        draw_formal_mark(surface, 0, 28, LAVENDER);
        draw_wordmark(surface, 142, 145, FOREGROUND);
        draw_centered_text(surface, 177, b"BOOT FAILED", LAVENDER);
        Ok(())
    }

    fn surface_mut(&mut self) -> Result<&mut [u32], BootError> {
        if self.surface_address == 0
            || !(self.surface_address as usize).is_multiple_of(core::mem::align_of::<u32>())
        {
            return Err(BootError::Surface);
        }
        let length = SURFACE_BYTES / core::mem::size_of::<u32>();
        let pointer = self.surface_address as *mut u32;
        Ok(unsafe { core::slice::from_raw_parts_mut(pointer, length) })
    }

    fn present(&self) -> Result<(), BootError> {
        if display_present(self.display_capability) {
            Ok(())
        } else {
            Err(BootError::Present)
        }
    }

    fn checksum(&self) -> u64 {
        let length = SURFACE_BYTES / core::mem::size_of::<u32>();
        let surface =
            unsafe { core::slice::from_raw_parts(self.surface_address as *const u32, length) };
        let mut value = 0xcbf2_9ce4_8422_2325_u64;
        for pixel in surface.iter().step_by(17) {
            value ^= u64::from(*pixel);
            value = value.wrapping_mul(0x1000_0000_01b3);
        }
        value
    }

    fn log_stage(&self, stage: BootStage, progress: u8) {
        let label = stage_label(stage);
        let mut output = [0_u8; 64];
        let mut length = 0;
        length = append(&mut output, length, b"Nagi boot stage ");
        length = append(&mut output, length, label);
        length = append(&mut output, length, b" ");
        length = append_decimal(&mut output, length, u64::from(progress));
        length = append(&mut output, length, b"\r\n");
        libnagi::console_write(&output[..length]);
    }

    fn log_lock_transition(&self, boot_checksum: u64, lock_checksum: u64) {
        let mut output = [0_u8; 160];
        let mut length = 0;
        length = append(&mut output, length, b"Nagi boot lock READY\r\n");
        length = append(&mut output, length, b"Nagi boot collapse COMPLETE\r\n");
        length = append(&mut output, length, b"Nagi boot frame checksum=");
        length = append_decimal(&mut output, length, boot_checksum);
        length = append(&mut output, length, b"\r\nNagi boot lock checksum=");
        length = append_decimal(&mut output, length, lock_checksum);
        length = append(&mut output, length, b"\r\n");
        libnagi::console_write(&output[..length]);
    }
}

fn stage_label(stage: BootStage) -> &'static [u8] {
    match stage {
        BootStage::Platform => b"PLATFORM",
        BootStage::CoreServices => b"CORE_SERVICES",
        BootStage::Storage => b"STORAGE",
        BootStage::Graphics => b"GRAPHICS",
        BootStage::Session => b"SESSION",
    }
}

fn phase_label(phase: BootPhase) -> &'static [u8] {
    match phase {
        BootPhase::SystemInit => b"SYSTEM INIT",
        BootPhase::CoreServices => b"CORE SERVICES",
        BootPhase::StorageMount => b"STORAGE MOUNT",
        BootPhase::GraphicsReady => b"GRAPHICS READY",
        BootPhase::SessionReady => b"SESSION READY",
        BootPhase::Ready => b"READY",
        BootPhase::Failed => b"BOOT FAILED",
    }
}

fn clear_surface(surface: &mut [u32], color: u32) {
    let mut index = 0;
    while index < surface.len() {
        unsafe { core::ptr::write(surface.as_mut_ptr().add(index), color) };
        index += 1;
    }
}

fn draw_ring_set(surface: &mut [u32], center_x: i32, center_y: i32, shrink: i32, alpha: u8) {
    let radii = [38 - shrink, 50 - shrink, 62 - shrink];
    let mut index = 0;
    while index < radii.len() {
        let radius = radii[index].max(8);
        let ring_alpha = alpha.saturating_sub(index as u8 * 18);
        let source = if index == 1 { CYAN } else { LAVENDER };
        draw_ring(
            surface,
            center_x,
            center_y,
            radius,
            blend(BACKGROUND, source, ring_alpha),
        );
        index += 1;
    }
}

fn draw_ring(surface: &mut [u32], center_x: i32, center_y: i32, radius: i32, color: u32) {
    let outer = (radius + 1) * (radius + 1);
    let inner = (radius - 1).max(0) * (radius - 1).max(0);
    let minimum_x = (center_x - radius - 1).max(0);
    let maximum_x = (center_x + radius + 1).min(SURFACE_WIDTH as i32 - 1);
    let minimum_y = (center_y - radius - 1).max(0);
    let maximum_y = (center_y + radius + 1).min(SURFACE_HEIGHT as i32 - 1);
    let mut y = minimum_y;
    while y <= maximum_y {
        let mut x = minimum_x;
        while x <= maximum_x {
            let delta_x = x - center_x;
            let delta_y = y - center_y;
            let distance = delta_x * delta_x + delta_y * delta_y;
            if distance >= inner && distance <= outer {
                put_pixel(surface, x, y, color);
            }
            x += 1;
        }
        y += 1;
    }
}

fn draw_progress_arc(
    surface: &mut [u32],
    center_x: i32,
    center_y: i32,
    radius: i32,
    progress: u8,
    frame: u32,
    frame_count: u32,
) {
    let active =
        (usize::from(progress) * (CIRCLE_POINTS.len() - 1) / 100).min(CIRCLE_POINTS.len() - 1);
    let offset = if frame_count > 1 {
        (frame as usize * 2) % CIRCLE_POINTS.len()
    } else {
        0
    };
    let mut previous = None;
    let mut step = 0;
    while step <= active {
        let point = CIRCLE_POINTS[(step + offset) % CIRCLE_POINTS.len()];
        let x = center_x + i32::from(point.0) * radius / 1000;
        let y = center_y + i32::from(point.1) * radius / 1000;
        if let Some((previous_x, previous_y)) = previous {
            draw_line(surface, previous_x, previous_y, x, y, CYAN);
        }
        draw_disc(surface, x, y, 1, CYAN);
        previous = Some((x, y));
        step += 1;
    }
    if let Some((marker_x, marker_y)) = previous {
        draw_disc(surface, marker_x, marker_y, 3, FOREGROUND);
        draw_disc(surface, marker_x, marker_y, 1, CYAN);
    }
}

fn draw_formal_mark(surface: &mut [u32], origin_x: i32, origin_y: i32, color: u32) {
    fill_polygon(surface, origin_x, origin_y, MARK_PATH);
    let mut index = 1;
    while index < MARK_PATH.len() {
        let start = MARK_PATH[index - 1];
        let end = MARK_PATH[index];
        let midpoint = origin_x + (start.0 + end.0) / 2;
        let edge_color = if color == FOREGROUND {
            gradient_color(midpoint)
        } else {
            color
        };
        draw_line(
            surface,
            origin_x + start.0,
            origin_y + start.1,
            origin_x + end.0,
            origin_y + end.1,
            edge_color,
        );
        index += 1;
    }
}

fn fill_polygon(surface: &mut [u32], origin_x: i32, origin_y: i32, path: &[(i32, i32)]) {
    let mut minimum_y = i32::MAX;
    let mut maximum_y = i32::MIN;
    for point in path {
        minimum_y = minimum_y.min(origin_y + point.1);
        maximum_y = maximum_y.max(origin_y + point.1);
    }
    let mut y = minimum_y.max(0);
    let last_y = maximum_y.min(SURFACE_HEIGHT as i32 - 1);
    let mut intersections = [0_i32; 48];
    while y <= last_y {
        let mut count = 0;
        let mut index = 1;
        while index < path.len() {
            let first = path[index - 1];
            let second = path[index];
            let first_y = origin_y + first.1;
            let second_y = origin_y + second.1;
            if ((first_y <= y && second_y > y) || (second_y <= y && first_y > y))
                && count < intersections.len()
            {
                let first_x = origin_x + first.0;
                let second_x = origin_x + second.0;
                intersections[count] =
                    first_x + (y - first_y) * (second_x - first_x) / (second_y - first_y);
                count += 1;
            }
            index += 1;
        }
        sort_intersections(&mut intersections[..count]);
        let mut pair = 0;
        while pair + 1 < count {
            let left = intersections[pair].max(0);
            let right = intersections[pair + 1].min(SURFACE_WIDTH as i32 - 1);
            let mut x = left;
            while x <= right {
                put_pixel(surface, x, y, gradient_color(x));
                x += 1;
            }
            pair += 2;
        }
        y += 1;
    }
}

fn sort_intersections(values: &mut [i32]) {
    let mut index = 1;
    while index < values.len() {
        let value = values[index];
        let mut position = index;
        while position > 0 && values[position - 1] > value {
            values[position] = values[position - 1];
            position -= 1;
        }
        values[position] = value;
        index += 1;
    }
}

fn gradient_color(x: i32) -> u32 {
    let position = ((x - 32).clamp(0, 146) * 255 / 146) as u8;
    if position < 128 {
        blend(CYAN, FOREGROUND, position.saturating_mul(2))
    } else {
        blend(FOREGROUND, LAVENDER, (position - 128).saturating_mul(2))
    }
}

fn draw_wordmark(surface: &mut [u32], x: i32, y: i32, color: u32) {
    let _ = crate::font::draw_utf8(surface, x, y, b"NAGI", color);
}

fn draw_centered_text(surface: &mut [u32], y: i32, text: &[u8], color: u32) {
    let width = text.len() as i32 * 6;
    let x = (SURFACE_WIDTH as i32 - width) / 2;
    let _ = crate::font::draw_utf8(surface, x, y, text, color);
}

fn draw_percentage(surface: &mut [u32], x: i32, y: i32, value: u8, color: u32) {
    let value = u16::from(value).min(100);
    let mut cursor = x;
    if value >= 100 {
        draw_digit(surface, cursor, y, 1, color);
        cursor += DIGIT_CELL_WIDTH;
        draw_digit(surface, cursor, y, 0, color);
        cursor += DIGIT_CELL_WIDTH;
        draw_digit(surface, cursor, y, 0, color);
        cursor += DIGIT_CELL_WIDTH;
    } else if value >= 10 {
        draw_digit(surface, cursor, y, (value / 10) as u8, color);
        cursor += DIGIT_CELL_WIDTH;
        draw_digit(surface, cursor, y, (value % 10) as u8, color);
        cursor += DIGIT_CELL_WIDTH;
    } else {
        draw_digit(surface, cursor, y, value as u8, color);
        cursor += DIGIT_CELL_WIDTH;
    }
    let _ = crate::font::draw_utf8(surface, cursor + 1, y + 3, b"%", color);
}

fn draw_digit(surface: &mut [u32], x: i32, y: i32, digit: u8, color: u32) {
    let rows = DIGIT_GLYPHS[digit.min(9) as usize];
    let mut row = 0;
    while row < rows.len() {
        let mut column = 0;
        while column < 5 {
            if rows[row] & (1 << (4 - column)) != 0 {
                let mut pixel_y = 0;
                while pixel_y < DIGIT_SCALE {
                    let mut pixel_x = 0;
                    while pixel_x < DIGIT_SCALE {
                        put_pixel(
                            surface,
                            x + column * DIGIT_SCALE + pixel_x,
                            y + row as i32 * DIGIT_SCALE + pixel_y,
                            color,
                        );
                        pixel_x += 1;
                    }
                    pixel_y += 1;
                }
            }
            column += 1;
        }
        row += 1;
    }
}

fn draw_line(surface: &mut [u32], start_x: i32, start_y: i32, end_x: i32, end_y: i32, color: u32) {
    let mut x = start_x;
    let mut y = start_y;
    let delta_x = (end_x - start_x).abs();
    let step_x = if start_x < end_x { 1 } else { -1 };
    let delta_y = -(end_y - start_y).abs();
    let step_y = if start_y < end_y { 1 } else { -1 };
    let mut error = delta_x + delta_y;
    loop {
        draw_disc(surface, x, y, 1, color);
        if x == end_x && y == end_y {
            break;
        }
        let double_error = error * 2;
        if double_error >= delta_y {
            error += delta_y;
            x += step_x;
        }
        if double_error <= delta_x {
            error += delta_x;
            y += step_y;
        }
    }
}

fn draw_disc(surface: &mut [u32], center_x: i32, center_y: i32, radius: i32, color: u32) {
    let mut y = -radius;
    while y <= radius {
        let mut x = -radius;
        while x <= radius {
            if x * x + y * y <= radius * radius {
                put_pixel(surface, center_x + x, center_y + y, color);
            }
            x += 1;
        }
        y += 1;
    }
}

fn put_pixel(surface: &mut [u32], x: i32, y: i32, color: u32) {
    if x < 0 || y < 0 || x >= SURFACE_WIDTH as i32 || y >= SURFACE_HEIGHT as i32 {
        return;
    }
    let index = y as usize * SURFACE_WIDTH as usize + x as usize;
    if index < surface.len() {
        unsafe { core::ptr::write(surface.as_mut_ptr().add(index), color) };
    }
}

fn blend(background: u32, foreground: u32, alpha: u8) -> u32 {
    let alpha = u32::from(alpha);
    let inverse = 255 - alpha;
    let red = (((background & 0xff) * inverse) + ((foreground & 0xff) * alpha)) / 255;
    let green =
        ((((background >> 8) & 0xff) * inverse) + (((foreground >> 8) & 0xff) * alpha)) / 255;
    let blue =
        ((((background >> 16) & 0xff) * inverse) + (((foreground >> 16) & 0xff) * alpha)) / 255;
    rgba(red as u8, green as u8, blue as u8)
}

fn delay_frame(frame_count: u32) {
    let iterations = frame_count.min(4) as usize * 1024;
    let mut iteration = 0;
    while iteration < iterations {
        core::hint::spin_loop();
        iteration += 1;
    }
}

fn append(destination: &mut [u8], offset: usize, source: &[u8]) -> usize {
    let mut index = 0;
    while index < source.len() && offset + index < destination.len() {
        destination[offset + index] = source[index];
        index += 1;
    }
    offset + index
}

fn append_decimal(destination: &mut [u8], mut offset: usize, mut value: u64) -> usize {
    let start = offset;
    loop {
        if offset == destination.len() {
            return offset;
        }
        destination[offset] = b'0' + (value % 10) as u8;
        offset += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    let mut left = start;
    let mut right = offset - 1;
    while left < right {
        destination.swap(left, right);
        left += 1;
        right -= 1;
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNTOUCHED_PIXEL: u32 = 0x1357_9bdf;

    fn test_screen(surface: &mut [u32]) -> BootScreen {
        BootScreen {
            display_capability: 0,
            surface_address: surface.as_mut_ptr() as u64,
            bridge: BootProgressBridge::new(BootMode::External),
        }
    }

    #[test]
    fn rejected_boot_stages_leave_the_frame_and_state_untouched() {
        let mut surface = vec![UNTOUCHED_PIXEL; SURFACE_BYTES / core::mem::size_of::<u32>()];
        let mut screen = test_screen(&mut surface);
        assert!(screen.bridge.advance(BootStage::Graphics));
        let expected_state = screen.bridge.state();

        assert_eq!(
            screen.present_stage(BootStage::Graphics),
            Err(BootError::InvalidState)
        );
        assert_eq!(screen.bridge.state(), expected_state);
        assert!(surface.iter().all(|pixel| *pixel == UNTOUCHED_PIXEL));

        assert_eq!(
            screen.present_stage(BootStage::Storage),
            Err(BootError::InvalidState)
        );
        assert_eq!(screen.bridge.state(), expected_state);
        assert!(surface.iter().all(|pixel| *pixel == UNTOUCHED_PIXEL));
    }

    #[test]
    fn rejected_finish_leaves_the_frame_and_failed_state_untouched() {
        let mut surface = vec![UNTOUCHED_PIXEL; SURFACE_BYTES / core::mem::size_of::<u32>()];
        let mut screen = test_screen(&mut surface);
        assert!(screen.bridge.advance(BootStage::Session));
        assert!(screen.bridge.fail());
        let expected_state = screen.bridge.state();

        assert_eq!(screen.finish_to_desktop(), Err(BootError::InvalidState));
        assert_eq!(screen.bridge.state(), expected_state);
        assert!(surface.iter().all(|pixel| *pixel == UNTOUCHED_PIXEL));
    }
}
