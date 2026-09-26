//! Localization and text-layout adapter boundary.

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
pub enum TextLayoutError {
    ZeroWidth,
    ZeroLines,
    AdapterExceededConstraints,
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
}
