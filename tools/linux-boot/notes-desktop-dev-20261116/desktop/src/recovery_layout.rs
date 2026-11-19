/// Place the recovery cause line below the hint when it fits inside both the
/// Notes window and the visible framebuffer. The bottom window border is
/// excluded from the text area. Returning `None` means this viewport cannot
/// show a complete cause glyph row without clipping or overlap.
pub(crate) fn cause_line_y(
    window_y: i32,
    window_height: i32,
    viewport_height: i32,
    title_height: i32,
    hint_offset: i32,
    preferred_offset: i32,
    glyph_height: i32,
) -> Option<i32> {
    if window_height <= 1
        || viewport_height <= 0
        || title_height < 0
        || hint_offset < 0
        || preferred_offset < 0
        || glyph_height <= 0
    {
        return None;
    }

    let title_y = window_y.checked_add(title_height)?;
    let hint_y = title_y.checked_add(hint_offset)?;
    let first_cause_y = hint_y.checked_add(glyph_height)?.max(0);
    let preferred_y = title_y.checked_add(preferred_offset)?;
    let window_bottom_border_y = window_y.checked_add(window_height)?.checked_sub(1)?;
    let visible_bottom = window_bottom_border_y.min(viewport_height);
    let last_cause_y = visible_bottom.checked_sub(glyph_height)?;
    let cause_y = preferred_y.min(last_cause_y);

    (cause_y >= first_cause_y).then_some(cause_y)
}
