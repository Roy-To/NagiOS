use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Locale {
    EnUs,
    JaJp,
}

impl Locale {
    pub fn parse(value: &str) -> Self {
        match value {
            "ja-JP" | "ja" => Self::JaJp,
            _ => Self::EnUs,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::JaJp => "ja-JP",
        }
    }
}

/// Shared lookup for first-party Home and Search UI copy. Message keys stay
/// English identifiers; missing Japanese entries fall back to en-US.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalizationCatalog;

impl LocalizationCatalog {
    pub fn resolve(&self, key: &str, locale: Locale) -> String {
        let localized = match locale {
            Locale::EnUs => english(key),
            Locale::JaJp => japanese(key).or_else(|| english(key)),
        };
        localized
            .map(str::to_owned)
            .unwrap_or_else(|| match locale {
                Locale::EnUs => "Message unavailable".to_owned(),
                Locale::JaJp => "メッセージを表示できません".to_owned(),
            })
    }

    pub fn all_strings(&self, locale: Locale) -> BTreeMap<&'static str, String> {
        MESSAGE_KEYS
            .iter()
            .map(|key| (*key, self.resolve(key, locale)))
            .collect()
    }
}

pub const MESSAGE_KEYS: &[&str] = &[
    "home.title",
    "home.subtitle",
    "home.search_placeholder",
    "home.workspace_section",
    "home.continue_section",
    "home.apps_section",
    "home.open_workspace",
    "home.intent_entry",
    "home.no_workspace",
    "home.no_continuations",
    "home.no_apps",
    "home.preview_notice",
    "home.eyebrow",
    "home.workspace_id",
    "home.related_objects",
    "home.workspace_references",
    "home.show_action",
    "home.registry_label",
    "home.load_error",
    "home.load_error_help",
    "home.local_preview_only",
    "search.title",
    "search.eyebrow",
    "search.category_list_label",
    "search.search_placeholder",
    "search.empty_prompt",
    "search.no_results",
    "search.loading",
    "search.error",
    "search.result_count",
    "search.match_exact",
    "search.match_prefix",
    "search.match_substring",
    "search.match_metadata",
    "search.match_content",
    "search.match_workspace",
    "search.category.apps",
    "search.category.files",
    "search.category.notes",
    "search.category.activity",
    "search.category.actions",
    "search.category.workspaces",
    "search.category.settings",
    "search.category.everything",
    "search.open_preview",
    "search.preview_action_notice",
    "search.source_fixture",
    "search.keyboard_help",
    "search.provider_failed",
    "search.provider_timeout",
    "search.provider_denied",
    "ui.nav_primary",
    "ui.preview_language_label",
    "ui.language_english",
    "ui.language_japanese",
    "ui.close",
    "ui.done",
    "ui.language_load_error",
    "availability.host_preview",
    "availability.available",
    "availability.coming_soon",
    "availability.unavailable",
    "availability.unlaunchable",
    "availability.permission_required",
    "app.albert",
    "app.albert.description",
    "app.files",
    "app.files.description",
    "app.notes",
    "app.notes.description",
    "app.terminal",
    "app.terminal.description",
    "app.activity",
    "app.activity.description",
    "app.wayback",
    "app.wayback.description",
    "app.search",
    "app.search.description",
    "app.home",
    "app.home.description",
    "app.package.description",
];

fn english(key: &str) -> Option<&'static str> {
    Some(match key {
        "home.title" => "Home",
        "home.subtitle" => "Pick up where you left off.",
        "home.search_placeholder" => "Search apps, files, notes, activity, and actions",
        "home.workspace_section" => "Current workspace",
        "home.continue_section" => "Continue",
        "home.apps_section" => "Apps",
        "home.open_workspace" => "Open workspace",
        "home.intent_entry" => "Describe what you want to do",
        "home.no_workspace" => "No workspace is available yet.",
        "home.no_continuations" => "Nothing to continue yet.",
        "home.no_apps" => "No apps are registered.",
        "home.preview_notice" => "Host preview. Demo fixtures are in-memory and are not Nagi user data.",
        "home.eyebrow" => "NAGI OS · HOME",
        "home.workspace_id" => "Workspace",
        "home.related_objects" => "related objects",
        "home.workspace_references" => "WORKSPACE REFERENCES",
        "home.show_action" => "Show action",
        "home.registry_label" => "FIRST-PARTY REGISTRY",
        "home.load_error" => "Home preview is unavailable.",
        "home.load_error_help" => "Start the local host preview server and reload.",
        "home.local_preview_only" => "Local host preview only. No Nagi or host user data is read.",
        "search.title" => "Search",
        "search.eyebrow" => "NAGI OS · QUICK SEARCH",
        "search.category_list_label" => "Search categories",
        "search.search_placeholder" => "Search Nagi",
        "search.empty_prompt" => "Type to search across available providers.",
        "search.no_results" => "No matching results.",
        "search.loading" => "Searching…",
        "search.error" => "Search could not finish. Try again.",
        "search.result_count" => "results",
        "search.match_exact" => "Exact match",
        "search.match_prefix" => "Starts with your query",
        "search.match_substring" => "Contains your query",
        "search.match_metadata" => "Matched metadata",
        "search.match_content" => "Matched content",
        "search.match_workspace" => "Related to this workspace",
        "search.category.apps" => "Apps",
        "search.category.files" => "Files",
        "search.category.notes" => "Notes",
        "search.category.activity" => "Activity",
        "search.category.actions" => "Actions",
        "search.category.workspaces" => "Workspaces",
        "search.category.settings" => "Settings",
        "search.category.everything" => "Everything",
        "search.open_preview" => "Open preview",
        "search.preview_action_notice" => "This preview shows the typed action target. It does not launch a Nagi app or open Nagi data.",
        "search.source_fixture" => "Demo provider · in-memory fixture",
        "search.keyboard_help" => "Use ↑ and ↓ to move, Enter to preview, and Escape to go back.",
        "search.provider_failed" => "Provider could not return results.",
        "search.provider_timeout" => "Provider took too long and was skipped.",
        "search.provider_denied" => "Provider permission is required.",
        "ui.nav_primary" => "Primary navigation",
        "ui.preview_language_label" => "Preview language",
        "ui.language_english" => "English",
        "ui.language_japanese" => "Japanese",
        "ui.close" => "Close",
        "ui.done" => "Done",
        "ui.language_load_error" => "Could not load the selected language.",
        "availability.host_preview" => "Host preview",
        "availability.available" => "Available",
        "availability.coming_soon" => "Coming soon",
        "availability.unavailable" => "Unavailable",
        "availability.unlaunchable" => "Cannot be launched yet",
        "availability.permission_required" => "Permission required",
        "app.albert" => "Albert",
        "app.albert.description" => "The Nagi browser app is not launchable in this preview.",
        "app.files" => "Files",
        "app.files.description" => "Browse and organize Nagi resources.",
        "app.notes" => "Notes",
        "app.notes.description" => "Capture and connect notes with other work.",
        "app.terminal" => "Terminal",
        "app.terminal.description" => "Run commands through the Nagi user-space terminal.",
        "app.activity" => "Activity",
        "app.activity.description" => "Review meaningful actions and recent changes.",
        "app.wayback" => "Wayback",
        "app.wayback.description" => "Inspect and restore prior object states.",
        "app.search" => "Search",
        "app.search.description" => "Find permitted apps, objects, and actions.",
        "app.home" => "Home",
        "app.home.description" => "Continue work from shared workspace context.",
        "app.package.description" => "Installed package. Launch support is not available in this preview.",
        _ => return None,
    })
}

fn japanese(key: &str) -> Option<&'static str> {
    Some(match key {
        "home.title" => "ホーム",
        "home.subtitle" => "前回の作業を続けましょう。",
        "home.search_placeholder" => "アプリ、ファイル、ノート、履歴、アクションを検索",
        "home.workspace_section" => "現在のワークスペース",
        "home.continue_section" => "続きから",
        "home.apps_section" => "アプリ",
        "home.open_workspace" => "ワークスペースを開く",
        "home.intent_entry" => "やりたいことを入力",
        "home.no_workspace" => "利用できるワークスペースはありません。",
        "home.no_continuations" => "続きから再開できる項目はありません。",
        "home.no_apps" => "登録されたアプリはありません。",
        "home.preview_notice" => "ホストプレビューです。デモ用データはメモリ内の fixture で、Nagi のユーザーデータではありません。",
        "home.eyebrow" => "NAGI OS · ホーム",
        "home.workspace_id" => "ワークスペース",
        "home.related_objects" => "関連オブジェクト",
        "home.workspace_references" => "ワークスペースの参照",
        "home.show_action" => "アクションを表示",
        "home.registry_label" => "標準アプリのレジストリ",
        "home.load_error" => "ホームプレビューを利用できません。",
        "home.load_error_help" => "ローカルのホストプレビューを起動して再読み込みしてください。",
        "home.local_preview_only" => "ローカルのホストプレビューです。Nagi やホストのユーザーデータは読み込みません。",
        "search.title" => "検索",
        "search.eyebrow" => "NAGI OS · クイック検索",
        "search.category_list_label" => "検索カテゴリ",
        "search.search_placeholder" => "Nagi を検索",
        "search.empty_prompt" => "入力すると利用可能な provider を横断検索します。",
        "search.no_results" => "一致する結果はありません。",
        "search.loading" => "検索中…",
        "search.error" => "検索を完了できませんでした。もう一度お試しください。",
        "search.result_count" => "件",
        "search.match_exact" => "完全一致",
        "search.match_prefix" => "先頭が一致",
        "search.match_substring" => "一部が一致",
        "search.match_metadata" => "メタデータが一致",
        "search.match_content" => "内容が一致",
        "search.match_workspace" => "このワークスペースに関連",
        "search.category.apps" => "アプリ",
        "search.category.files" => "ファイル",
        "search.category.notes" => "ノート",
        "search.category.activity" => "アクティビティ",
        "search.category.actions" => "アクション",
        "search.category.workspaces" => "ワークスペース",
        "search.category.settings" => "設定",
        "search.category.everything" => "すべて",
        "search.open_preview" => "プレビューを開く",
        "search.preview_action_notice" => "typed action の対象を表示します。Nagi アプリの起動や Nagi データの読み込みは行いません。",
        "search.source_fixture" => "デモ provider · メモリ内 fixture",
        "search.keyboard_help" => "↑↓で移動、Enterでプレビュー、Escapeで戻ります。",
        "search.provider_failed" => "provider から結果を取得できませんでした。",
        "search.provider_timeout" => "provider の応答が遅いため検索対象から除外しました。",
        "search.provider_denied" => "provider の利用権限が必要です。",
        "ui.nav_primary" => "メインナビゲーション",
        "ui.preview_language_label" => "プレビューの言語",
        "ui.language_english" => "英語",
        "ui.language_japanese" => "日本語",
        "ui.close" => "閉じる",
        "ui.done" => "完了",
        "ui.language_load_error" => "選択した言語を読み込めませんでした。",
        "availability.host_preview" => "ホストプレビュー",
        "availability.available" => "利用可能",
        "availability.coming_soon" => "開発予定",
        "availability.unavailable" => "利用できません",
        "availability.unlaunchable" => "まだ起動できません",
        "availability.permission_required" => "権限が必要です",
        "app.albert" => "Albert",
        "app.albert.description" => "このプレビューでは Nagi ブラウザーを起動できません。",
        "app.files" => "ファイル",
        "app.files.description" => "Nagi のリソースを参照・整理します。",
        "app.notes" => "ノート",
        "app.notes.description" => "ノートを記録し、他の作業と関連付けます。",
        "app.terminal" => "ターミナル",
        "app.terminal.description" => "Nagi のユーザー空間ターミナルでコマンドを実行します。",
        "app.activity" => "アクティビティ",
        "app.activity.description" => "意味のある操作と最近の変更を確認します。",
        "app.wayback" => "Wayback",
        "app.wayback.description" => "オブジェクトの過去の状態を確認・復元します。",
        "app.search" => "検索",
        "app.search.description" => "権限のあるアプリ、オブジェクト、アクションを検索します。",
        "app.home" => "ホーム",
        "app.home.description" => "共有ワークスペースの情報から作業を再開します。",
        "app.package.description" => "インストール済みパッケージです。このプレビューでは起動できません。",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{Locale, LocalizationCatalog, MESSAGE_KEYS};

    #[test]
    fn english_and_japanese_catalogs_cover_every_shared_key() {
        let catalog = LocalizationCatalog;
        for key in MESSAGE_KEYS {
            assert!(!catalog.resolve(key, Locale::EnUs).is_empty(), "{key}");
            assert!(!catalog.resolve(key, Locale::JaJp).is_empty(), "{key}");
        }
    }

    #[test]
    fn missing_japanese_translation_falls_back_to_english_without_exposing_key() {
        let catalog = LocalizationCatalog;
        assert_eq!(catalog.resolve("search.result_count", Locale::JaJp), "件");
        assert_eq!(
            catalog.resolve("unregistered.message", Locale::JaJp),
            "メッセージを表示できません"
        );
    }

    #[test]
    fn unsupported_locale_falls_back_to_en_us() {
        assert_eq!(Locale::parse("fr-FR"), Locale::EnUs);
    }
}
