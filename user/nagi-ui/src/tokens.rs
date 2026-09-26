//! Stable semantic values shared by first-party UI adapters.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThemeMode {
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl Color {
    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha: u8::MAX,
        }
    }

    /// Pack RGBA bytes in the same little-endian pixel convention as M10's painter.
    pub const fn to_pixel(self) -> u32 {
        self.red as u32
            | ((self.green as u32) << 8)
            | ((self.blue as u32) << 16)
            | ((self.alpha as u32) << 24)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorRole {
    Canvas,
    Surface,
    SurfaceRaised,
    SurfaceSunken,
    TextPrimary,
    TextSecondary,
    TextDisabled,
    TextOnAccent,
    Accent,
    AccentHover,
    AccentPressed,
    Border,
    BorderStrong,
    Focus,
    Selection,
    Danger,
    Success,
    Warning,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Palette {
    pub mode: ThemeMode,
    pub canvas: Color,
    pub surface: Color,
    pub surface_raised: Color,
    pub surface_sunken: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_disabled: Color,
    pub text_on_accent: Color,
    pub accent: Color,
    pub accent_hover: Color,
    pub accent_pressed: Color,
    pub border: Color,
    pub border_strong: Color,
    pub focus: Color,
    pub selection: Color,
    pub danger: Color,
    pub success: Color,
    pub warning: Color,
}

impl Palette {
    pub const fn color(self, role: ColorRole) -> Color {
        match role {
            ColorRole::Canvas => self.canvas,
            ColorRole::Surface => self.surface,
            ColorRole::SurfaceRaised => self.surface_raised,
            ColorRole::SurfaceSunken => self.surface_sunken,
            ColorRole::TextPrimary => self.text_primary,
            ColorRole::TextSecondary => self.text_secondary,
            ColorRole::TextDisabled => self.text_disabled,
            ColorRole::TextOnAccent => self.text_on_accent,
            ColorRole::Accent => self.accent,
            ColorRole::AccentHover => self.accent_hover,
            ColorRole::AccentPressed => self.accent_pressed,
            ColorRole::Border => self.border,
            ColorRole::BorderStrong => self.border_strong,
            ColorRole::Focus => self.focus,
            ColorRole::Selection => self.selection,
            ColorRole::Danger => self.danger,
            ColorRole::Success => self.success,
            ColorRole::Warning => self.warning,
        }
    }
}

const fn light_palette() -> Palette {
    Palette {
        mode: ThemeMode::Light,
        canvas: Color::rgb(243, 247, 247),
        surface: Color::rgb(255, 255, 255),
        surface_raised: Color::rgb(255, 255, 255),
        surface_sunken: Color::rgb(232, 239, 239),
        text_primary: Color::rgb(25, 37, 43),
        text_secondary: Color::rgb(77, 94, 101),
        text_disabled: Color::rgb(125, 140, 145),
        text_on_accent: Color::rgb(255, 255, 255),
        accent: Color::rgb(8, 119, 110),
        accent_hover: Color::rgb(7, 103, 96),
        accent_pressed: Color::rgb(6, 86, 81),
        border: Color::rgb(207, 219, 220),
        border_strong: Color::rgb(151, 169, 171),
        focus: Color::rgb(0, 116, 108),
        selection: Color::rgb(199, 229, 224),
        danger: Color::rgb(174, 48, 58),
        success: Color::rgb(25, 112, 75),
        warning: Color::rgb(129, 84, 0),
    }
}

const fn dark_palette() -> Palette {
    Palette {
        mode: ThemeMode::Dark,
        canvas: Color::rgb(17, 25, 36),
        surface: Color::rgb(28, 39, 52),
        surface_raised: Color::rgb(37, 51, 67),
        surface_sunken: Color::rgb(11, 17, 26),
        text_primary: Color::rgb(237, 243, 246),
        text_secondary: Color::rgb(184, 197, 204),
        text_disabled: Color::rgb(125, 140, 151),
        text_on_accent: Color::rgb(255, 255, 255),
        accent: Color::rgb(48, 157, 145),
        accent_hover: Color::rgb(57, 177, 163),
        accent_pressed: Color::rgb(37, 127, 118),
        border: Color::rgb(57, 73, 88),
        border_strong: Color::rgb(91, 111, 126),
        focus: Color::rgb(102, 207, 192),
        selection: Color::rgb(37, 83, 76),
        danger: Color::rgb(236, 127, 132),
        success: Color::rgb(115, 207, 163),
        warning: Color::rgb(241, 194, 105),
    }
}

/// Resolve a complete semantic palette for the selected system appearance.
pub const fn palette(mode: ThemeMode) -> Palette {
    match mode {
        ThemeMode::Light => light_palette(),
        ThemeMode::Dark => dark_palette(),
    }
}

pub const fn color(mode: ThemeMode, role: ColorRole) -> Color {
    palette(mode).color(role)
}

/// A four-point logical-pixel spacing scale: 0, 4, 8, 12, 16, 24, 32, 48.
pub const SPACING: [u8; 8] = [0, 4, 8, 12, 16, 24, 32, 48];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpacingRole {
    Zero,
    XSmall,
    Small,
    Medium,
    Large,
    XLarge,
    XXLarge,
    XXXLarge,
}

pub const fn spacing(role: SpacingRole) -> u8 {
    SPACING[match role {
        SpacingRole::Zero => 0,
        SpacingRole::XSmall => 1,
        SpacingRole::Small => 2,
        SpacingRole::Medium => 3,
        SpacingRole::Large => 4,
        SpacingRole::XLarge => 5,
        SpacingRole::XXLarge => 6,
        SpacingRole::XXXLarge => 7,
    }]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TypeRole {
    Caption,
    Body,
    BodyStrong,
    Heading,
    Title,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontWeight {
    Regular,
    Medium,
    Semibold,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TypeTokens {
    pub size_px: u8,
    pub line_height_px: u8,
    pub weight: FontWeight,
}

pub const fn typography(role: TypeRole) -> TypeTokens {
    match role {
        TypeRole::Caption => TypeTokens {
            size_px: 12,
            line_height_px: 16,
            weight: FontWeight::Regular,
        },
        TypeRole::Body => TypeTokens {
            size_px: 15,
            line_height_px: 22,
            weight: FontWeight::Regular,
        },
        TypeRole::BodyStrong => TypeTokens {
            size_px: 15,
            line_height_px: 22,
            weight: FontWeight::Semibold,
        },
        TypeRole::Heading => TypeTokens {
            size_px: 20,
            line_height_px: 28,
            weight: FontWeight::Semibold,
        },
        TypeRole::Title => TypeTokens {
            size_px: 26,
            line_height_px: 34,
            weight: FontWeight::Medium,
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CornerRadius {
    Small,
    Medium,
    Large,
    Pill,
}

pub const fn corner_radius(role: CornerRadius) -> u8 {
    match role {
        CornerRadius::Small => 4,
        CornerRadius::Medium => 8,
        CornerRadius::Large => 12,
        CornerRadius::Pill => u8::MAX,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Density {
    Comfortable,
    Compact,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DensityTokens {
    pub control_min_height: u8,
    pub row_min_height: u8,
    pub large_row_min_height: u8,
    pub inline_padding: u8,
    pub item_gap: u8,
    pub icon_size: u8,
}

pub const fn density_tokens(density: Density) -> DensityTokens {
    match density {
        Density::Comfortable => DensityTokens {
            control_min_height: 36,
            row_min_height: 40,
            large_row_min_height: 48,
            inline_padding: 12,
            item_gap: 8,
            icon_size: 20,
        },
        Density::Compact => DensityTokens {
            control_min_height: 28,
            row_min_height: 32,
            large_row_min_height: 40,
            inline_padding: 8,
            item_gap: 6,
            icon_size: 16,
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IconSize {
    Small,
    Medium,
    Large,
}

pub const fn icon_size_px(size: IconSize) -> u8 {
    match size {
        IconSize::Small => 16,
        IconSize::Medium => 20,
        IconSize::Large => 24,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ElevationRole {
    Flat,
    Raised,
    Overlay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ElevationTokens {
    pub surface: ColorRole,
    pub border: ColorRole,
    /// Relative shadow step for the renderer; zero means no shadow.
    pub shadow_step: u8,
}

pub const fn elevation_tokens(role: ElevationRole) -> ElevationTokens {
    match role {
        ElevationRole::Flat => ElevationTokens {
            surface: ColorRole::Surface,
            border: ColorRole::Border,
            shadow_step: 0,
        },
        ElevationRole::Raised => ElevationTokens {
            surface: ColorRole::SurfaceRaised,
            border: ColorRole::Border,
            shadow_step: 1,
        },
        ElevationRole::Overlay => ElevationTokens {
            surface: ColorRole::SurfaceRaised,
            border: ColorRole::BorderStrong,
            shadow_step: 2,
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MotionRole {
    Fast,
    Normal,
    Slow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MotionPreference {
    Full,
    Reduced,
}

/// State transitions are 120/180/260 ms; reduced motion always resolves to 0.
pub const fn motion_duration_ms(role: MotionRole, preference: MotionPreference) -> u16 {
    if matches!(preference, MotionPreference::Reduced) {
        return 0;
    }
    match role {
        MotionRole::Fast => 120,
        MotionRole::Normal => 180,
        MotionRole::Slow => 260,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_palettes_resolve_every_semantic_role() {
        let roles = [
            ColorRole::Canvas,
            ColorRole::Surface,
            ColorRole::SurfaceRaised,
            ColorRole::SurfaceSunken,
            ColorRole::TextPrimary,
            ColorRole::TextSecondary,
            ColorRole::TextDisabled,
            ColorRole::TextOnAccent,
            ColorRole::Accent,
            ColorRole::AccentHover,
            ColorRole::AccentPressed,
            ColorRole::Border,
            ColorRole::BorderStrong,
            ColorRole::Focus,
            ColorRole::Selection,
            ColorRole::Danger,
            ColorRole::Success,
            ColorRole::Warning,
        ];
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let resolved = palette(mode);
            assert_eq!(resolved.mode, mode);
            for role in roles {
                assert_eq!(resolved.color(role).alpha, u8::MAX, "{mode:?} {role:?}");
            }
        }
        assert_ne!(
            color(ThemeMode::Light, ColorRole::Canvas),
            color(ThemeMode::Dark, ColorRole::Canvas)
        );
    }

    #[test]
    fn scale_roles_are_defined_in_code() {
        assert_eq!(SPACING, [0, 4, 8, 12, 16, 24, 32, 48]);
        assert_eq!(spacing(SpacingRole::Large), 16);
        assert_eq!(typography(TypeRole::Body).line_height_px, 22);
        assert_eq!(corner_radius(CornerRadius::Medium), 8);
        assert!(
            density_tokens(Density::Comfortable).row_min_height
                > density_tokens(Density::Compact).row_min_height
        );
        assert_eq!(icon_size_px(IconSize::Large), 24);
    }

    #[test]
    fn reduced_motion_suppresses_every_transition() {
        for role in [MotionRole::Fast, MotionRole::Normal, MotionRole::Slow] {
            assert_eq!(motion_duration_ms(role, MotionPreference::Reduced), 0);
            assert!(motion_duration_ms(role, MotionPreference::Full) > 0);
        }
    }
}
