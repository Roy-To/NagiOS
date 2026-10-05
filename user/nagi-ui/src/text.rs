//! Localization and text-layout adapter boundary.

use crate::layout::LogicalSize;
use crate::tokens::TypeRole;

/// Stable English-based localization key; displayed text is never used as identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MessageKey(&'static str);

impl MessageKey {
    pub const fn new(key: &'static str) -> Self {
        Self(key)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// Text returned by an injected localization service for a stable message key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedText<'a> {
    pub key: MessageKey,
    pub value: &'a str,
}

impl<'a> ResolvedText<'a> {
    pub const fn new(key: MessageKey, value: &'a str) -> Self {
        Self { key, value }
    }
}

/// Apps adapt the shared localization service through this small interface.
pub trait TextResolver {
    type Error;

    fn resolve(&self, key: MessageKey) -> Result<&str, Self::Error>;
}

pub fn resolve_text<R: TextResolver>(
    resolver: &R,
    key: MessageKey,
) -> Result<ResolvedText<'_>, R::Error> {
    resolver
        .resolve(key)
        .map(|value| ResolvedText { key, value })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextWrap {
    None,
    AtAvailableLineBreaks,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextOverflow {
    Clip,
    Ellipsis,
    Reject,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextConstraints {
    pub max_width: u16,
    pub max_lines: u16,
    pub wrap: TextWrap,
    pub overflow: TextOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextLayoutMetrics {
    pub line_count: u16,
    pub max_line_width: u16,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextSizeConstraints {
    pub min_width: u16,
    pub max_width: u16,
    pub min_height: u16,
    pub max_height: u16,
}

impl TextSizeConstraints {
    pub const fn new(min_width: u16, max_width: u16, min_height: u16, max_height: u16) -> Self {
        Self {
            min_width,
            max_width,
            min_height,
            max_height,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextSizeError {
    InvalidWidthRange,
    InvalidHeightRange,
    ContentExceedsMaximum,
}

/// Resolve a box size from metrics for the actual selected locale. The caller
/// can reflow or report overflow rather than clipping text to a maximum.
pub const fn resolve_text_size(
    metrics: TextLayoutMetrics,
    line_height_px: u8,
    constraints: TextSizeConstraints,
) -> Result<LogicalSize, TextSizeError> {
    if constraints.min_width > constraints.max_width {
        return Err(TextSizeError::InvalidWidthRange);
    }
    if constraints.min_height > constraints.max_height {
        return Err(TextSizeError::InvalidHeightRange);
    }
    let measured_height = metrics.line_count as u32 * line_height_px as u32;
    if metrics.max_line_width > constraints.max_width
        || measured_height > constraints.max_height as u32
        || measured_height > u16::MAX as u32
    {
        return Err(TextSizeError::ContentExceedsMaximum);
    }
    Ok(LogicalSize {
        width: if metrics.max_line_width < constraints.min_width {
            constraints.min_width
        } else {
            metrics.max_line_width
        },
        height: if measured_height < constraints.min_height as u32 {
            constraints.min_height
        } else {
            measured_height as u16
        },
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextLayoutError {
    ZeroWidth,
    ZeroLines,
    AdapterExceededConstraints,
    TruncationRejected,
}

/// Renderer/font adapter shapes the resolved locale and applies the requested policy.
pub trait TextLayoutAdapter {
    fn layout(&self, text: &str, role: TypeRole, constraints: TextConstraints)
        -> TextLayoutMetrics;
}

/// Layout always receives resolved locale text, never a key or English surrogate.
pub fn layout_resolved_text<A: TextLayoutAdapter>(
    adapter: &A,
    text: ResolvedText<'_>,
    role: TypeRole,
    constraints: TextConstraints,
) -> Result<TextLayoutMetrics, TextLayoutError> {
    if constraints.max_width == 0 {
        return Err(TextLayoutError::ZeroWidth);
    }
    if constraints.max_lines == 0 {
        return Err(TextLayoutError::ZeroLines);
    }
    let metrics = adapter.layout(text.value, role, constraints);
    if metrics.line_count > constraints.max_lines || metrics.max_line_width > constraints.max_width
    {
        return Err(TextLayoutError::AdapterExceededConstraints);
    }
    if metrics.truncated && constraints.overflow == TextOverflow::Reject {
        return Err(TextLayoutError::TruncationRejected);
    }
    Ok(metrics)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CharacterCellLayout;

    impl TextLayoutAdapter for CharacterCellLayout {
        fn layout(
            &self,
            text: &str,
            _role: TypeRole,
            constraints: TextConstraints,
        ) -> TextLayoutMetrics {
            let line_count = if constraints.wrap == TextWrap::None {
                1
            } else {
                let cells_per_line = usize::from(constraints.max_width / 8).max(1);
                let chars = text.chars().count();
                chars.div_ceil(cells_per_line).max(1)
            };
            let line_count = line_count.min(usize::from(constraints.max_lines));
            TextLayoutMetrics {
                line_count: line_count as u16,
                max_line_width: constraints.max_width.min(40),
                truncated: text.chars().count() > line_count * 5,
            }
        }
    }

    struct JapaneseResolver;

    impl TextResolver for JapaneseResolver {
        type Error = ();

        fn resolve(&self, key: MessageKey) -> Result<&str, Self::Error> {
            match key.as_str() {
                "settings.display.title" => Ok("画面とアクセシビリティの設定"),
                _ => Err(()),
            }
        }
    }

    struct LocaleWidthLayout;

    impl TextLayoutAdapter for LocaleWidthLayout {
        fn layout(
            &self,
            text: &str,
            _role: TypeRole,
            constraints: TextConstraints,
        ) -> TextLayoutMetrics {
            let cells = text
                .chars()
                .map(|character| if character.is_ascii() { 1 } else { 2 })
                .sum::<usize>();
            let cells_per_line = usize::from(constraints.max_width / 8).max(1);
            let line_count = cells.div_ceil(cells_per_line).max(1);
            TextLayoutMetrics {
                line_count: line_count as u16,
                max_line_width: constraints.max_width.min((cells_per_line * 8) as u16),
                truncated: line_count > usize::from(constraints.max_lines),
            }
        }
    }

    struct TruncatingLayout;

    impl TextLayoutAdapter for TruncatingLayout {
        fn layout(
            &self,
            _text: &str,
            _role: TypeRole,
            constraints: TextConstraints,
        ) -> TextLayoutMetrics {
            TextLayoutMetrics {
                line_count: 1,
                max_line_width: constraints.max_width,
                truncated: true,
            }
        }
    }

    #[test]
    fn localization_injects_japanese_text_before_measured_layout() {
        let key = MessageKey::new("settings.display.title");
        let resolved = resolve_text(&JapaneseResolver, key).unwrap();
        assert_eq!(resolved.key, key);
        assert!(resolved.value.contains("日本語") || resolved.value.contains('設'));
        let metrics = layout_resolved_text(
            &CharacterCellLayout,
            resolved,
            TypeRole::Body,
            TextConstraints {
                max_width: 40,
                max_lines: 3,
                wrap: TextWrap::AtAvailableLineBreaks,
                overflow: TextOverflow::Ellipsis,
            },
        )
        .unwrap();
        assert_eq!(metrics.line_count, 3);
        assert!(!metrics.truncated);
    }

    #[test]
    fn layout_rejects_invalid_constraints_and_adapter_overflow() {
        let resolved = ResolvedText::new(MessageKey::new("common.title"), "Nagi");
        let no_width = TextConstraints {
            max_width: 0,
            max_lines: 1,
            wrap: TextWrap::None,
            overflow: TextOverflow::Clip,
        };
        assert_eq!(
            layout_resolved_text(&CharacterCellLayout, resolved, TypeRole::Body, no_width),
            Err(TextLayoutError::ZeroWidth)
        );
        assert_eq!(
            layout_resolved_text(
                &CharacterCellLayout,
                resolved,
                TypeRole::Body,
                TextConstraints {
                    max_width: 40,
                    max_lines: 0,
                    wrap: TextWrap::None,
                    overflow: TextOverflow::Clip,
                }
            ),
            Err(TextLayoutError::ZeroLines)
        );
    }

    #[test]
    fn english_and_japanese_layout_use_resolved_text_and_bounded_sizes() {
        let constraints = TextConstraints {
            max_width: 64,
            max_lines: 4,
            wrap: TextWrap::AtAvailableLineBreaks,
            overflow: TextOverflow::Reject,
        };
        let english = layout_resolved_text(
            &LocaleWidthLayout,
            ResolvedText::new(MessageKey::new("display.title"), "Change display settings"),
            TypeRole::Body,
            constraints,
        )
        .unwrap();
        let japanese = layout_resolved_text(
            &LocaleWidthLayout,
            ResolvedText::new(MessageKey::new("display.title"), "設定の表示を調整"),
            TypeRole::Body,
            constraints,
        )
        .unwrap();
        assert!(english.line_count > japanese.line_count);

        let limits = TextSizeConstraints::new(80, 120, 40, 100);
        let english_size = resolve_text_size(english, 22, limits).unwrap();
        let japanese_size = resolve_text_size(japanese, 22, limits).unwrap();
        assert_eq!(english_size.width, 80);
        assert_eq!(japanese_size.width, 80);
        assert_eq!(english_size.height, english.line_count * 22);
        assert_eq!(japanese_size.height, japanese.line_count * 22);
    }

    #[test]
    fn reject_overflow_refuses_truncated_critical_text() {
        let text = ResolvedText::new(MessageKey::new("permission.denied"), "Access denied");
        assert_eq!(
            layout_resolved_text(
                &TruncatingLayout,
                text,
                TypeRole::Body,
                TextConstraints {
                    max_width: 80,
                    max_lines: 1,
                    wrap: TextWrap::None,
                    overflow: TextOverflow::Reject,
                }
            ),
            Err(TextLayoutError::TruncationRejected)
        );
    }

    #[test]
    fn localized_size_constraints_reject_invalid_ranges_and_overflow() {
        let metrics = TextLayoutMetrics {
            line_count: 2,
            max_line_width: 96,
            truncated: false,
        };
        assert_eq!(
            resolve_text_size(metrics, 22, TextSizeConstraints::new(120, 160, 40, 60)),
            Ok(crate::layout::LogicalSize {
                width: 120,
                height: 44
            })
        );
        assert_eq!(
            resolve_text_size(metrics, 22, TextSizeConstraints::new(161, 160, 40, 60)),
            Err(TextSizeError::InvalidWidthRange)
        );
        assert_eq!(
            resolve_text_size(metrics, 22, TextSizeConstraints::new(0, 160, 61, 60)),
            Err(TextSizeError::InvalidHeightRange)
        );
        assert_eq!(
            resolve_text_size(metrics, 22, TextSizeConstraints::new(0, 80, 40, 60)),
            Err(TextSizeError::ContentExceedsMaximum)
        );
    }
}
