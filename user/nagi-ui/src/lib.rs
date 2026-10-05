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
    role_for_component, AccessibleField, AccessibleFieldError, AccessibleNode, AccessibleRole,
    AccessibleState, CheckedState, KeyboardOperation,
};
pub use command_palette::{
    CommandId, CommandPalette, CommandPaletteAction, CommandResult, PaletteStatus, ShortcutHint,
};
pub use dialog::{ActionId, DialogAction, DialogModel};
pub use focus::{
    FocusDirection, FocusManager, FocusScope, FocusTarget, NavigationAction, NavigationBehavior,
    NavigationModel, SelectAction, SelectModel,
};
pub use interaction::{
    component_family, ButtonEvent, ButtonModel, ButtonOutcome, ComponentFamily, ComponentKind,
    ComponentState, FeedbackState, InteractionState, KeyCode, TextFieldAction, TextFieldModel,
    ToggleKind, ToggleModel, COMPONENT_KINDS,
};
pub use layout::{
    divider_orientation, layout_spec, progress_percent, status_color_role, ApplicationShellSpec,
    DividerOrientation, EmptyStateSpec, ErrorStateSpec, Insets, LayoutError, LayoutKind,
    LayoutSpec, LogicalSize, ProgressState, ScrollAxis, ScrollContract, SettingsControl,
    SettingsRowSpec, ShellRegionSpec, StatusTone, SurfaceFrame, SurfaceKind, TextAlignment,
    TooltipSpec, TooltipTrigger,
};
pub use text::{
    layout_resolved_text, resolve_text_size, MessageKey, ResolvedText, TextConstraints,
    TextLayoutAdapter, TextLayoutError, TextLayoutMetrics, TextOverflow, TextResolver,
    TextSizeConstraints, TextSizeError, TextWrap,
};
pub use tokens::{
    border_width_px, color, control_tokens, corner_radius, density_tokens, elevation_tokens,
    focus_treatment, icon_size_px, motion_duration_ms, palette, scaled_typography, spacing,
    typography, BorderRole, Color, ColorRole, ControlSize, ControlTokens, CornerRadius, Density,
    DensityTokens, ElevationRole, ElevationTokens, FocusTreatment, FontFamily, FontWeight,
    IconSize, MotionPreference, MotionRole, Palette, SpacingRole, TextScale, ThemeMode, TypeRole,
    TypeTokens, SPACING,
};
