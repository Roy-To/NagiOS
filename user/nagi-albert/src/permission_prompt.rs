//! Bounded first-party permission prompt content and input hit testing.

use nagi_localization::{text, Locale};

use crate::permissions::{PermissionKind, UserDecision};

pub const MAX_PROMPT_ORIGIN_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionPromptAction {
    Allow,
    Deny,
    Cancel,
}

impl PermissionPromptAction {
    pub const fn decision(self) -> UserDecision {
        match self {
            Self::Allow => UserDecision::Allow,
            Self::Deny => UserDecision::Deny,
            Self::Cancel => UserDecision::Dismiss,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PermissionPromptLabels {
    pub title: &'static str,
    pub origin: &'static str,
    pub feature: &'static str,
    pub allow: &'static str,
    pub deny: &'static str,
    pub cancel: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionPromptView<'a> {
    pub origin: &'a str,
    pub feature: &'a str,
    pub labels: PermissionPromptLabels,
}

impl PermissionPromptLabels {
    pub fn for_locale(locale: Locale) -> Self {
        Self {
            title: text(locale, "albert.permission.title"),
            origin: text(locale, "albert.permission.origin"),
            feature: text(locale, "albert.permission.feature"),
            allow: text(locale, "albert.permission.allow"),
            deny: text(locale, "albert.permission.deny"),
            cancel: text(locale, "albert.permission.cancel"),
        }
    }

    pub fn feature_name(locale: Locale, kind: PermissionKind) -> &'static str {
        let key = match kind {
            PermissionKind::ClipboardRead => "albert.permission.feature.clipboard_read",
            PermissionKind::ClipboardWrite => "albert.permission.feature.clipboard_write",
            PermissionKind::FileUpload => "albert.permission.feature.file_upload",
            PermissionKind::Download => "albert.permission.feature.download",
            PermissionKind::Location => "albert.permission.feature.location",
            PermissionKind::Notifications => "albert.permission.feature.notifications",
            PermissionKind::Push => "albert.permission.feature.push",
            PermissionKind::Midi => "albert.permission.feature.midi",
            PermissionKind::Camera => "albert.permission.feature.camera",
            PermissionKind::Microphone => "albert.permission.feature.microphone",
            PermissionKind::Speaker => "albert.permission.feature.speaker",
            PermissionKind::DeviceInfo => "albert.permission.feature.device_info",
            PermissionKind::BackgroundSync => "albert.permission.feature.background_sync",
            PermissionKind::Bluetooth => "albert.permission.feature.bluetooth",
            PermissionKind::PersistentStorage => "albert.permission.feature.persistent_storage",
            PermissionKind::ScreenWakeLock => "albert.permission.feature.screen_wake_lock",
            PermissionKind::Gamepad => "albert.permission.feature.gamepad",
        };
        text(locale, key)
    }
}

/// Return a bounded display copy; the original origin remains the authority key.
pub fn bounded_origin(origin: &str) -> String {
    let mut end = origin.len().min(MAX_PROMPT_ORIGIN_BYTES);
    while !origin.is_char_boundary(end) {
        end -= 1;
    }
    origin[..end].to_owned()
}

/// Hit-test the three visible controls. Only a primary-button press should be
/// passed here; coordinates outside the controls cannot resolve a request.
pub fn action_at(width: u32, height: u32, x: u32, y: u32) -> Option<PermissionPromptAction> {
    let layout = PromptLayout::new(width, height)?;
    if x < layout.buttons_x
        || x >= layout.buttons_right
        || y < layout.buttons_y
        || y >= layout.buttons_bottom
    {
        return None;
    }
    let relative_x = x - layout.buttons_x;
    let slot_width = layout.button_width + layout.button_gap;
    let slot = relative_x / slot_width;
    if relative_x % slot_width >= layout.button_width {
        return None;
    }
    if slot > 2 {
        return None;
    }
    match slot {
        0 => Some(PermissionPromptAction::Cancel),
        1 => Some(PermissionPromptAction::Deny),
        2 => Some(PermissionPromptAction::Allow),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PromptLayout {
    pub card: (u32, u32, u32, u32),
    pub buttons_x: u32,
    pub buttons_y: u32,
    pub buttons_right: u32,
    pub buttons_bottom: u32,
    pub button_width: u32,
    pub button_gap: u32,
}

impl PromptLayout {
    pub(crate) fn new(width: u32, height: u32) -> Option<Self> {
        const MIN_WIDTH: u32 = 208;
        const MIN_HEIGHT: u32 = 128;
        if width < MIN_WIDTH || height < MIN_HEIGHT {
            return None;
        }
        let card_width = width.saturating_sub(24).min(360);
        let card_height = 128.min(height.saturating_sub(16));
        let card_x = width.saturating_sub(card_width) / 2;
        let card_y = height.saturating_sub(card_height) / 2;
        let buttons_x = card_x + 8;
        let button_gap = 4;
        let button_width = card_width.saturating_sub(16 + button_gap * 2) / 3;
        let buttons_right = buttons_x + button_width * 3 + button_gap * 2;
        let buttons_y = card_y + card_height - 30;
        let buttons_bottom = buttons_y + 22;
        Some(Self {
            card: (card_x, card_y, card_width, card_height),
            buttons_x,
            buttons_y,
            buttons_right,
            buttons_bottom,
            button_width,
            button_gap,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_prompt_choices_require_a_click_inside_their_own_button() {
        let layout = PromptLayout::new(320, 200).unwrap();
        let center_y = layout.buttons_y + 10;
        let first_center = layout.buttons_x + layout.button_width / 2;
        let second_center =
            layout.buttons_x + layout.button_width + layout.button_gap + layout.button_width / 2;
        let third_center = layout.buttons_x
            + (layout.button_width + layout.button_gap) * 2
            + layout.button_width / 2;

        assert_eq!(
            action_at(320, 200, first_center, center_y),
            Some(PermissionPromptAction::Cancel)
        );
        assert_eq!(
            action_at(320, 200, second_center, center_y),
            Some(PermissionPromptAction::Deny)
        );
        assert_eq!(
            action_at(320, 200, third_center, center_y),
            Some(PermissionPromptAction::Allow)
        );
        assert_eq!(
            action_at(320, 200, layout.buttons_x + layout.button_width, center_y,),
            None
        );
        assert_eq!(action_at(320, 200, 200, 100), None);
        assert_eq!(
            action_at(320, 200, first_center, layout.buttons_y - 1),
            None
        );
    }

    #[test]
    fn prompt_fails_closed_when_the_viewport_cannot_fit_the_controls() {
        assert_eq!(PromptLayout::new(207, 200), None);
        assert_eq!(PromptLayout::new(320, 127), None);
        assert_eq!(action_at(320, 200, 200, 100), None);
    }

    #[test]
    fn prompt_labels_and_common_features_are_localized_in_both_first_party_locales() {
        let english = PermissionPromptLabels::for_locale(Locale::EnUs);
        let japanese = PermissionPromptLabels::for_locale(Locale::JaJp);
        assert_eq!(english.title, "Site permission");
        assert_eq!(japanese.title, "権限");
        assert_eq!(english.origin, "Site");
        assert_eq!(japanese.origin, "サイト");
        assert_eq!(english.feature, "Feature");
        assert_eq!(japanese.feature, "機能");
        assert_eq!(english.allow, "Allow");
        assert_eq!(japanese.allow, "許可");
        assert_eq!(english.deny, "Deny");
        assert_eq!(japanese.deny, "拒否");
        assert_eq!(english.cancel, "Cancel");
        assert_eq!(japanese.cancel, "取消");
        assert_eq!(
            PermissionPromptLabels::feature_name(Locale::JaJp, PermissionKind::Camera),
            "カメラ"
        );
        assert_eq!(
            PermissionPromptLabels::feature_name(Locale::JaJp, PermissionKind::Location),
            "位置情報"
        );
    }

    #[test]
    fn every_broker_permission_kind_has_localized_feature_copy() {
        let kinds = [
            PermissionKind::ClipboardRead,
            PermissionKind::ClipboardWrite,
            PermissionKind::FileUpload,
            PermissionKind::Download,
            PermissionKind::Location,
            PermissionKind::Notifications,
            PermissionKind::Push,
            PermissionKind::Midi,
            PermissionKind::Camera,
            PermissionKind::Microphone,
            PermissionKind::Speaker,
            PermissionKind::DeviceInfo,
            PermissionKind::BackgroundSync,
            PermissionKind::Bluetooth,
            PermissionKind::PersistentStorage,
            PermissionKind::ScreenWakeLock,
            PermissionKind::Gamepad,
        ];
        for locale in [Locale::EnUs, Locale::JaJp] {
            for kind in kinds {
                let feature = PermissionPromptLabels::feature_name(locale, kind);
                assert!(!feature.is_empty());
                assert_ne!(feature, "Text unavailable.");
            }
        }
    }

    #[test]
    fn displayed_origin_is_utf8_bounded_without_changing_the_authority_value() {
        let origin = format!("https://{}.example", "あ".repeat(100));
        let bounded = bounded_origin(&origin);
        assert!(bounded.len() <= MAX_PROMPT_ORIGIN_BYTES);
        assert!(origin.starts_with(&bounded));
        assert!(bounded.is_char_boundary(bounded.len()));
    }

    #[test]
    fn cancel_and_deny_both_fail_closed_but_remain_distinct_user_choices() {
        assert_eq!(
            PermissionPromptAction::Cancel.decision(),
            UserDecision::Dismiss
        );
        assert_eq!(PermissionPromptAction::Deny.decision(), UserDecision::Deny);
        assert_eq!(
            PermissionPromptAction::Allow.decision(),
            UserDecision::Allow
        );
    }

    #[test]
    fn m18_harness_allow_click_lands_on_allow() {
        // tools/nagi-cli M18_PERMISSION_ALLOW_EVENTS clicks here on the
        // 320x200 surface; keep both in step.
        assert_eq!(
            super::action_at(320, 200, 253, 145),
            Some(PermissionPromptAction::Allow)
        );
    }
}
