use core::fmt;
use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::{Catalog, CatalogSet, MessageId, MISSING_MESSAGE_ID};
use crate::locale::{LocaleContext, LocaleId};
use crate::template::{parse_template, valid_argument_name, TemplatePart};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiagnosticKind {
    FallbackUsed,
    MissingMessage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalizationDiagnostic {
    pub kind: DiagnosticKind,
    pub requested_locale: LocaleId,
    pub resolved_locale: Option<LocaleId>,
    pub message_id: MessageId,
}

pub trait DiagnosticSink: Send + Sync {
    fn report(&self, diagnostic: LocalizationDiagnostic);
}

#[derive(Default)]
pub struct NoopDiagnosticSink;

impl DiagnosticSink for NoopDiagnosticSink {
    fn report(&self, _diagnostic: LocalizationDiagnostic) {}
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MessageArgs(BTreeMap<String, String>);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArgumentError(String);

impl MessageArgs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(
        &mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), ArgumentError> {
        let name = name.into();
        if !valid_argument_name(&name) {
            return Err(ArgumentError(format!("invalid message argument `{name}`")));
        }
        self.0.insert(name, value.into());
        Ok(())
    }
}

impl fmt::Display for ArgumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ArgumentError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LookupError {
    MissingFallbackMessage,
    MissingArgument {
        message_id: MessageId,
        name: String,
    },
    UnexpectedArgument {
        message_id: MessageId,
        name: String,
    },
    MalformedCatalogTemplate {
        message_id: MessageId,
        detail: String,
    },
}

impl fmt::Display for LookupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingFallbackMessage => {
                formatter.write_str("the required system.message_unavailable message is missing")
            }
            Self::MissingArgument { message_id, name } => {
                write!(
                    formatter,
                    "message `{message_id}` requires argument `{name}`"
                )
            }
            Self::UnexpectedArgument { message_id, name } => {
                write!(
                    formatter,
                    "message `{message_id}` does not use argument `{name}`"
                )
            }
            Self::MalformedCatalogTemplate { message_id, detail } => {
                write!(
                    formatter,
                    "message `{message_id}` has malformed template: {detail}"
                )
            }
        }
    }
}

impl std::error::Error for LookupError {}

pub struct Localizer {
    catalogs: CatalogSet,
    context: LocaleContext,
    diagnostics: Box<dyn DiagnosticSink>,
}

impl Localizer {
    pub fn new(
        catalogs: CatalogSet,
        context: LocaleContext,
        diagnostics: impl DiagnosticSink + 'static,
    ) -> Self {
        Self {
            catalogs,
            context,
            diagnostics: Box::new(diagnostics),
        }
    }

    pub fn bundled(
        context: LocaleContext,
        diagnostics: impl DiagnosticSink + 'static,
    ) -> Result<Self, crate::catalog::CatalogError> {
        Ok(Self::new(CatalogSet::bundled()?, context, diagnostics))
    }

    pub fn context(&self) -> &LocaleContext {
        &self.context
    }

    /// Resolve a stable ID and interpolate named arguments. An absent key is
    /// logged and replaced with the catalog's localized generic fallback so
    /// callers never display a raw key or an empty string.
    pub fn render(
        &self,
        message_id: &MessageId,
        arguments: &MessageArgs,
    ) -> Result<String, LookupError> {
        if let Some((catalog, template)) = self.find_message(message_id) {
            self.report_fallback_if_needed(message_id, catalog);
            return render_template(template, message_id, arguments);
        }

        self.diagnostics.report(LocalizationDiagnostic {
            kind: DiagnosticKind::MissingMessage,
            requested_locale: self.context.system_language.clone(),
            resolved_locale: None,
            message_id: message_id.clone(),
        });

        let fallback_id = MessageId::new(MISSING_MESSAGE_ID).expect("stable built-in message ID");
        let Some((catalog, template)) = self.find_message(&fallback_id) else {
            return Err(LookupError::MissingFallbackMessage);
        };
        self.report_fallback_if_needed(&fallback_id, catalog);
        render_template(template, &fallback_id, &MessageArgs::new())
    }

    fn find_message(&self, message_id: &MessageId) -> Option<(&Catalog, &str)> {
        for locale in lookup_candidates(&self.context.system_language) {
            let Some(catalog) = self.catalogs.get(&locale) else {
                continue;
            };
            if let Some(template) = catalog.message(message_id) {
                return Some((catalog, template));
            }
        }
        None
    }

    fn report_fallback_if_needed(&self, message_id: &MessageId, catalog: &Catalog) {
        if catalog.locale() != &self.context.system_language {
            self.diagnostics.report(LocalizationDiagnostic {
                kind: DiagnosticKind::FallbackUsed,
                requested_locale: self.context.system_language.clone(),
                resolved_locale: Some(catalog.locale().clone()),
                message_id: message_id.clone(),
            });
        }
    }
}

fn lookup_candidates(requested: &LocaleId) -> Vec<LocaleId> {
    let mut candidates = requested.fallback_candidates();
    let official_fallback = match requested.language() {
        "en" => Some("en-US"),
        "ja" => Some("ja-JP"),
        _ => None,
    };
    if let Some(official_fallback) = official_fallback {
        candidates.push(LocaleId::parse(official_fallback).expect("valid official locale"));
    }
    candidates.push(LocaleId::parse("en-US").expect("valid reference locale"));
    let mut unique = Vec::new();
    for candidate in candidates {
        if !unique.contains(&candidate) {
            unique.push(candidate);
        }
    }
    unique
}

fn render_template(
    template: &str,
    message_id: &MessageId,
    arguments: &MessageArgs,
) -> Result<String, LookupError> {
    let parts =
        parse_template(template).map_err(|error| LookupError::MalformedCatalogTemplate {
            message_id: message_id.clone(),
            detail: error.to_string(),
        })?;
    let mut output = String::new();
    let mut used_arguments = BTreeSet::new();
    for part in parts {
        match part {
            TemplatePart::Literal(text) => output.push_str(&text),
            TemplatePart::Argument(name) => {
                let value = arguments
                    .0
                    .get(&name)
                    .ok_or_else(|| LookupError::MissingArgument {
                        message_id: message_id.clone(),
                        name: name.clone(),
                    })?;
                used_arguments.insert(name);
                output.push_str(value);
            }
        }
    }
    if let Some(name) = arguments
        .0
        .keys()
        .find(|name| !used_arguments.contains(*name))
    {
        return Err(LookupError::UnexpectedArgument {
            message_id: message_id.clone(),
            name: name.clone(),
        });
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::{DiagnosticKind, DiagnosticSink, LocalizationDiagnostic, Localizer, MessageArgs};
    use crate::catalog::{Catalog, CatalogSet, MessageId};
    use crate::locale::{LocaleContext, LocaleId};

    #[derive(Clone, Default)]
    struct RecordingSink(Arc<Mutex<Vec<LocalizationDiagnostic>>>);

    impl DiagnosticSink for RecordingSink {
        fn report(&self, diagnostic: LocalizationDiagnostic) {
            self.0.lock().unwrap().push(diagnostic);
        }
    }

    fn localizer(language: &str, region: &str, sink: RecordingSink) -> Localizer {
        Localizer::new(
            CatalogSet::bundled().unwrap(),
            LocaleContext::new(
                LocaleId::parse(language).unwrap(),
                LocaleId::parse(region).unwrap(),
            ),
            sink,
        )
    }

    #[test]
    fn resolves_first_class_english_and_japanese_messages() {
        let save = MessageId::new("common.save").unwrap();
        let english = localizer("en-US", "ja-JP", RecordingSink::default());
        let japanese = localizer("ja-JP", "en-US", RecordingSink::default());
        assert_eq!(english.render(&save, &MessageArgs::new()).unwrap(), "Save");
        assert_eq!(japanese.render(&save, &MessageArgs::new()).unwrap(), "保存");
    }

    #[test]
    fn pseudo_catalog_runs_through_the_same_runtime_lookup_contract() {
        let catalogs = CatalogSet::bundled().unwrap().with_pseudo_locale().unwrap();
        let localizer = Localizer::new(
            catalogs,
            LocaleContext::new(
                LocaleId::parse("en-XA").unwrap(),
                LocaleId::parse("en-US").unwrap(),
            ),
            super::super::NoopDiagnosticSink,
        );
        let save = MessageId::new("common.save").unwrap();
        assert_eq!(
            localizer.render(&save, &MessageArgs::new()).unwrap(),
            "⟦Sáavée⟧"
        );
    }

    #[test]
    fn merges_namespaced_application_resources_into_shared_locale_catalogs() {
        let app_english = Catalog::parse(
            r#"{"schema_version":1,"locale":"en-US","script":"Latn","direction":"ltr","font_fallback_scripts":["Latn"],"messages":{"app.files.root_title":"Files"}}"#,
        )
        .unwrap();
        let app_japanese = Catalog::parse(
            r#"{"schema_version":1,"locale":"ja-JP","script":"Jpan","direction":"ltr","font_fallback_scripts":["Jpan"],"messages":{"app.files.root_title":"ファイル"}}"#,
        )
        .unwrap();
        let catalogs = CatalogSet::bundled()
            .unwrap()
            .with_additional_catalogs([app_english, app_japanese])
            .unwrap();
        let id = MessageId::new("app.files.root_title").unwrap();
        let japanese = Localizer::new(
            catalogs,
            LocaleContext::new(
                LocaleId::parse("ja-JP").unwrap(),
                LocaleId::parse("ja-JP").unwrap(),
            ),
            super::super::NoopDiagnosticSink,
        );
        assert_eq!(
            japanese.render(&id, &MessageArgs::new()).unwrap(),
            "ファイル"
        );
    }

    #[test]
    fn uses_same_language_and_then_english_fallbacks_deterministically() {
        let save = MessageId::new("common.save").unwrap();
        let fallback_sink = RecordingSink::default();
        let japanese_variant = localizer("ja-HR", "ja-JP", fallback_sink.clone());
        let unsupported = localizer("fr-CA", "fr-CA", RecordingSink::default());
        assert_eq!(
            japanese_variant.render(&save, &MessageArgs::new()).unwrap(),
            "保存"
        );
        assert_eq!(
            unsupported.render(&save, &MessageArgs::new()).unwrap(),
            "Save"
        );
        let diagnostics = fallback_sink.0.lock().unwrap();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::FallbackUsed);
        assert_eq!(
            diagnostics[0].resolved_locale.as_ref().unwrap().as_str(),
            "ja-JP"
        );
    }

    #[test]
    fn missing_key_is_diagnosed_and_never_exposes_the_raw_key() {
        let sink = RecordingSink::default();
        let localizer = localizer("ja-JP", "ja-JP", sink.clone());
        let absent = MessageId::new("settings.not_yet_added").unwrap();
        let rendered = localizer.render(&absent, &MessageArgs::new()).unwrap();
        assert_eq!(rendered, "このメッセージは利用できません。");
        assert!(!rendered.contains(absent.as_str()));
        assert_eq!(
            sink.0.lock().unwrap()[0].kind,
            DiagnosticKind::MissingMessage
        );
    }

    #[test]
    fn falls_back_per_message_and_reports_the_actual_locale() {
        let english = Catalog::parse(
            r#"{"schema_version":1,"locale":"en-US","script":"Latn","direction":"ltr","font_fallback_scripts":["Latn"],"messages":{"common.save":"Save","common.cancel":"Cancel","system.message_unavailable":"Unavailable"}}"#,
        )
        .unwrap();
        let japanese = Catalog::parse(
            r#"{"schema_version":1,"locale":"ja-JP","script":"Jpan","direction":"ltr","font_fallback_scripts":["Jpan"],"messages":{"common.cancel":"キャンセル","system.message_unavailable":"利用できません"}}"#,
        )
        .unwrap();
        let sink = RecordingSink::default();
        let localizer = Localizer::new(
            CatalogSet::new([english, japanese]).unwrap(),
            LocaleContext::new(
                LocaleId::parse("ja-JP").unwrap(),
                LocaleId::parse("ja-JP").unwrap(),
            ),
            sink.clone(),
        );
        let save = MessageId::new("common.save").unwrap();
        assert_eq!(
            localizer.render(&save, &MessageArgs::new()).unwrap(),
            "Save"
        );
        let diagnostics = sink.0.lock().unwrap();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::FallbackUsed);
        assert_eq!(
            diagnostics[0].resolved_locale.as_ref().unwrap().as_str(),
            "en-US"
        );
    }

    #[test]
    fn interpolates_named_arguments_and_reports_missing_arguments() {
        let english = Catalog::parse(
            r#"{"schema_version":1,"locale":"en-US","script":"Latn","direction":"ltr","font_fallback_scripts":["Latn"],"messages":{"common.greeting":"Hello, {name}!","system.message_unavailable":"Unavailable"}}"#,
        )
        .unwrap();
        let localizer = Localizer::new(
            CatalogSet::new([english]).unwrap(),
            LocaleContext::new(
                LocaleId::parse("en-US").unwrap(),
                LocaleId::parse("en-US").unwrap(),
            ),
            super::super::NoopDiagnosticSink,
        );
        let greeting = MessageId::new("common.greeting").unwrap();
        let mut args = MessageArgs::new();
        args.insert("name", "Nagi").unwrap();
        assert_eq!(localizer.render(&greeting, &args).unwrap(), "Hello, Nagi!");
        assert!(localizer
            .render(&greeting, &MessageArgs::new())
            .unwrap_err()
            .to_string()
            .contains("argument `name`"));
        assert!(args.insert("invalid name", "x").is_err());
        let mut extra = MessageArgs::new();
        extra.insert("name", "Nagi").unwrap();
        extra.insert("unused", "value").unwrap();
        assert!(localizer
            .render(&greeting, &extra)
            .unwrap_err()
            .to_string()
            .contains("does not use argument `unused`"));
    }

    #[test]
    fn display_language_and_region_are_independent_inputs() {
        let context = LocaleContext::new(
            LocaleId::parse("en-US").unwrap(),
            LocaleId::parse("ja-JP").unwrap(),
        );
        assert_eq!(context.system_language.as_str(), "en-US");
        assert_eq!(context.region.as_str(), "ja-JP");
    }
}
