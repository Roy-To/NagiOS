#![no_std]
#![forbid(unsafe_code)]

//! Backend-neutral design tokens and deterministic component contracts for Nagi.
//!
//! This crate owns presentation vocabulary and local interaction state only.
//! Applications inject resolved text, theme preference, text layout, input,
//! and rendering adapters. It does not call Nagi services or execute app
//! commands.

pub mod accessibility;
pub mod command_palette;
pub mod dialog;
pub mod focus;
pub mod interaction;
pub mod layout;
pub mod text;
pub mod tokens;

pub use accessibility::{
    role_for_component, AccessibleNode, AccessibleRole, AccessibleState, CheckedState,
    KeyboardOperation,
};
pub use command_palette::{
    CommandId, CommandPalette, CommandPaletteAction, CommandResult, PaletteStatus, ShortcutHint,
};
pub use dialog::{ActionId, DialogAction, DialogModel};
pub use focus::{
    FocusDirection, FocusManager, FocusTarget, NavigationAction, NavigationBehavior,
    NavigationModel,
};
pub use interaction::{
    component_family, ButtonEvent, ButtonModel, ButtonOutcome, ComponentFamily, ComponentKind,
    InteractionState, KeyCode, TextFieldAction, TextFieldModel, ToggleKind, ToggleModel,
    COMPONENT_KINDS,
};
pub use layout::{
    divider_orientation, layout_spec, progress_percent, status_color_role, DividerOrientation,
    EmptyStateSpec, Insets, LayoutError, LayoutKind, LayoutSpec, LogicalSize, ProgressState,
    ScrollAxis, ScrollContract, SettingsControl, SettingsRowSpec, StatusTone, SurfaceFrame,
    SurfaceKind, TextAlignment, TooltipSpec, TooltipTrigger,
};
pub use text::{
    layout_resolved_text, MessageKey, ResolvedText, TextConstraints, TextLayoutAdapter,
    TextLayoutError, TextLayoutMetrics, TextOverflow, TextResolver, TextWrap,
};
pub use tokens::{
    color, corner_radius, density_tokens, elevation_tokens, icon_size_px, motion_duration_ms,
    palette, spacing, typography, Color, ColorRole, CornerRadius, Density, DensityTokens,
    ElevationRole, ElevationTokens, FontWeight, IconSize, MotionPreference, MotionRole, Palette,
    SpacingRole, ThemeMode, TypeRole, TypeTokens, SPACING,
};
