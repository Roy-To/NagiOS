//! Logical-pixel layout, surface, and passive content contracts.

use crate::text::MessageKey;
use crate::tokens::{spacing, ColorRole, CornerRadius, ElevationRole, SpacingRole};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogicalSize {
    pub width: u16,
    pub height: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Insets {
    pub top: u8,
    pub right: u8,
    pub bottom: u8,
    pub left: u8,
}

impl Insets {
    pub const fn all(value: SpacingRole) -> Self {
        let value = spacing(value);
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }

    pub const fn symmetric(vertical: SpacingRole, horizontal: SpacingRole) -> Self {
        Self {
            top: spacing(vertical),
            right: spacing(horizontal),
            bottom: spacing(vertical),
            left: spacing(horizontal),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextAlignment {
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutKind {
    Row {
        gap: SpacingRole,
        wrap: bool,
    },
    Column {
        gap: SpacingRole,
    },
    Stack {
        alignment: TextAlignment,
    },
    Grid {
        columns: u8,
        column_gap: SpacingRole,
        row_gap: SpacingRole,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LayoutSpec {
    pub kind: LayoutKind,
    pub padding: Insets,
}

impl LayoutSpec {
    pub const fn new(kind: LayoutKind, padding: Insets) -> Self {
        Self { kind, padding }
    }

    pub const fn validate(self) -> Result<Self, LayoutError> {
        match self.kind {
            LayoutKind::Grid { columns: 0, .. } => Err(LayoutError::GridNeedsColumn),
            _ => Ok(self),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutError {
    GridNeedsColumn,
}

pub const fn layout_spec(kind: LayoutKind, padding: Insets) -> LayoutSpec {
    LayoutSpec::new(kind, padding)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceKind {
    WindowContent,
    Dialog,
    Sheet,
    Popover,
    SettingsPage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurfaceFrame {
    pub background: ColorRole,
    pub border: ColorRole,
    pub elevation: ElevationRole,
    pub radius: CornerRadius,
    pub padding: Insets,
}

pub const fn surface_frame(kind: SurfaceKind) -> SurfaceFrame {
    match kind {
        SurfaceKind::WindowContent => SurfaceFrame {
            background: ColorRole::Surface,
            border: ColorRole::Border,
            elevation: ElevationRole::Raised,
            radius: CornerRadius::Medium,
            padding: Insets::all(SpacingRole::Large),
        },
        SurfaceKind::Dialog => SurfaceFrame {
            background: ColorRole::SurfaceRaised,
            border: ColorRole::BorderStrong,
            elevation: ElevationRole::Overlay,
            radius: CornerRadius::Large,
            padding: Insets::all(SpacingRole::XLarge),
        },
        SurfaceKind::Sheet => SurfaceFrame {
            background: ColorRole::SurfaceRaised,
            border: ColorRole::Border,
            elevation: ElevationRole::Overlay,
            radius: CornerRadius::Large,
            padding: Insets::all(SpacingRole::Large),
        },
        SurfaceKind::Popover => SurfaceFrame {
            background: ColorRole::SurfaceRaised,
            border: ColorRole::BorderStrong,
            elevation: ElevationRole::Overlay,
            radius: CornerRadius::Medium,
            padding: Insets::all(SpacingRole::Small),
        },
        SurfaceKind::SettingsPage => SurfaceFrame {
            background: ColorRole::Canvas,
            border: ColorRole::Border,
            elevation: ElevationRole::Flat,
            radius: CornerRadius::Medium,
            padding: Insets::all(SpacingRole::XLarge),
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScrollAxis {
    Vertical,
    Horizontal,
    Both,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScrollContract {
    pub axis: ScrollAxis,
    pub focusable: bool,
    pub line_step: SpacingRole,
    pub page_step: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SettingsRowSpec {
    pub label: MessageKey,
    pub description: Option<MessageKey>,
    pub control: SettingsControl,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsControl {
    None,
    Toggle,
    Picker,
    Navigation,
    Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmptyStateSpec {
    pub title: MessageKey,
    pub description: MessageKey,
    pub action_label: Option<MessageKey>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TooltipTrigger {
    HoverOrFocus,
    FocusOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TooltipSpec {
    pub label: MessageKey,
    pub trigger: TooltipTrigger,
    pub show_delay_ms: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgressState {
    Indeterminate,
    Determinate { value: u32, maximum: u32 },
}

impl ProgressState {
    pub const fn determinate(value: u32, maximum: u32) -> Option<Self> {
        if maximum == 0 || value > maximum {
            None
        } else {
            Some(Self::Determinate { value, maximum })
        }
    }
}

pub const fn progress_percent(progress: ProgressState) -> Option<u8> {
    match progress {
        ProgressState::Indeterminate => None,
        ProgressState::Determinate { value, maximum } => {
            if maximum == 0 {
                None
            } else {
                Some((((value as u64) * 100) / maximum as u64) as u8)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusTone {
    Neutral,
    Accent,
    Danger,
    Success,
    Warning,
}

pub const fn status_color_role(tone: StatusTone) -> ColorRole {
    match tone {
        StatusTone::Neutral => ColorRole::TextSecondary,
        StatusTone::Accent => ColorRole::Accent,
        StatusTone::Danger => ColorRole::Danger,
        StatusTone::Success => ColorRole::Success,
        StatusTone::Warning => ColorRole::Warning,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DividerOrientation {
    Horizontal,
    Vertical,
}

pub const fn divider_orientation(vertical: bool) -> DividerOrientation {
    if vertical {
        DividerOrientation::Vertical
    } else {
        DividerOrientation::Horizontal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_column_stack_and_grid_layout_contracts_are_explicit() {
        let padding = Insets::symmetric(SpacingRole::Medium, SpacingRole::Large);
        let layouts = [
            LayoutSpec::new(
                LayoutKind::Row {
                    gap: SpacingRole::Small,
                    wrap: true,
                },
                padding,
            ),
            LayoutSpec::new(
                LayoutKind::Column {
                    gap: SpacingRole::Medium,
                },
                padding,
            ),
            LayoutSpec::new(
                LayoutKind::Stack {
                    alignment: TextAlignment::Center,
                },
                padding,
            ),
            LayoutSpec::new(
                LayoutKind::Grid {
                    columns: 2,
                    column_gap: SpacingRole::Large,
                    row_gap: SpacingRole::Medium,
                },
                padding,
            ),
        ];
        assert!(layouts.iter().all(|layout| layout.validate().is_ok()));
        assert_eq!(padding.left, 16);
        assert_eq!(padding.top, 12);
        assert_eq!(
            LayoutSpec::new(
                LayoutKind::Grid {
                    columns: 0,
                    column_gap: SpacingRole::Zero,
                    row_gap: SpacingRole::Zero,
                },
                Insets::all(SpacingRole::Zero)
            )
            .validate(),
            Err(LayoutError::GridNeedsColumn)
        );
    }

    #[test]
    fn windows_dialogs_sheets_and_settings_share_surface_roles() {
        let dialog = surface_frame(SurfaceKind::Dialog);
        let popover = surface_frame(SurfaceKind::Popover);
        assert_eq!(dialog.elevation, ElevationRole::Overlay);
        assert_eq!(dialog.background, ColorRole::SurfaceRaised);
        assert!(dialog.padding.left > popover.padding.left);
        assert_eq!(
            surface_frame(SurfaceKind::SettingsPage).background,
            ColorRole::Canvas
        );
    }

    #[test]
    fn progress_and_status_semantics_are_bounded() {
        let progress = ProgressState::determinate(3, 4).unwrap();
        assert_eq!(progress_percent(progress), Some(75));
        assert_eq!(ProgressState::determinate(1, 0), None);
        assert_eq!(ProgressState::determinate(5, 4), None);
        assert_eq!(progress_percent(ProgressState::Indeterminate), None);
        assert_eq!(status_color_role(StatusTone::Danger), ColorRole::Danger);
        assert_eq!(divider_orientation(false), DividerOrientation::Horizontal);
    }
}
