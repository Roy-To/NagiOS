# Nagi Language Architecture

Status: normative Nagi OS 0.1 common-platform architecture

This document applies to Nagi OS, Albert, Settings, Files, Terminal, the
launcher, lock screen, setup/OOBE, notifications, system dialogs, Store, and
future Nagi first-party applications. It is the detailed reference for the
normative summary in `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md` and the
short implementation rules in `AGENTS.md`.

## Canonical internal language

English is Nagi's canonical internal language. Use English for source-code
identifiers, API and service names, IPC/RPC and message names, schemas,
configuration keys, localization keys, event names, internal command names,
developer-facing diagnostics, developer-facing log identifiers, and
machine-readable error identifiers.

Choosing Japanese as the user language must not change those internal
identifiers or API structures. User-facing labels, explanations, errors,
notifications, and dialogs are resolved through the localization layer.

## Official Nagi 0.1 languages

Nagi 0.1 officially supports both of these languages as equal first-class
user languages:

- English (`en-US`)
- Japanese (`ja-JP`)

Japanese is not an experimental, community, partial, secondary, optional, or
post-hoc translation tier. The standard UI, first-party applications,
settings, notifications, errors, and initial setup must be designed so that
both supported languages can be used consistently.

The architecture must permit future language packs such as `ko-KR`, `zh-CN`,
`zh-TW`, `de-DE`, `fr-FR`, and `es-ES` without a broad rewrite of OS code.
Those languages are not Nagi 0.1 support commitments.

## Localization resources and keys

User-facing strings are externalized into shared localization resources.
Applications should use the shared localization service/framework rather than
inventing incompatible per-application language systems. Stable English-based
keys identify messages; displayed text is never used as a key.

Examples:

```text
settings.language.title
settings.region.title
common.save
common.cancel
files.rename
errors.permission_denied
```

The resource representation may follow the repository's selected tooling, but
it must support locale separation, stable keys, Unicode, validation, and
future language addition. The conceptual layout is:

```text
locales/
├── en-US.*
└── ja-JP.*
```

When a selected-locale entry is missing, resolution falls back to `en-US`.
It must not leave an empty UI, expose the key to the user, or break the whole
screen. Development tooling should detect missing translations where
practical.

## Separate language, region, input, and AI settings

These are separate concepts in both the data model and user interface:

- **System Language** controls OS and first-party application presentation.
- **Region/Locale** controls date/time formats, week start, decimal and
  thousands separators, currency, number and measurement conventions, and
  timezone presentation.
- **Input Language/Keyboard** controls layouts, IMEs, and input languages.
- **Albert/AI Conversation Language** controls AI conversation behavior and
  supports `Auto`, English, and Japanese as independent choices.

System language and region may be combined in a display label, but must not be
forced to the same setting. For example, English presentation with Japan
regional conventions is valid. Input profiles may contain both a Japanese IME
and an English (US) keyboard. Albert may converse in Japanese while the
system is in English, and `Auto` may follow mixed Japanese/English input
without rejecting or artificially splitting it.

System language should be switchable without reinstalling Nagi, ideally after
re-login or a UI restart. First-party applications inherit it by default.
Albert's conversation-language preference remains independent.

## Encoding, terminal, and errors

UTF-8 is Nagi's default encoding for internal text processing, configuration,
localization resources, and logs unless a specific external protocol requires
another encoding. ASCII-only assumptions and host-OS code-page dependencies
are not valid substitutes for Unicode handling.

Terminal command names, option names, environment variable names, shell
syntax, and developer diagnostics remain English in Nagi 0.1. Beginner help,
GUI assistance, and user-facing error explanations may be localized.

Machine-readable errors are stable English identifiers separate from
localized user-facing messages:

```text
error_id: FILE_PERMISSION_DENIED
en-US: You do not have permission to modify this file.
ja-JP: このファイルを変更する権限がありません。
```

Logs and AI diagnostics should retain the stable internal identifier regardless
of the selected presentation language.

## Current implementation boundary

`crates/nagi-localization` provides the shared user-space foundation: canonical
locale IDs, separate system-language and region inputs, versioned UTF-8 JSON
catalogs, stable message IDs, named interpolation, exact/language/English
fallback, structured diagnostics, first-class `en-US` and `ja-JP` resources,
deterministic initial formatting, and an `en-XA` pseudo-locale. The catalog
checker validates metadata, duplicate IDs, English/Japanese parity, and
placeholder parity. The resource and caller contract is documented in
`docs/guides/localization.md`.

This foundation does not implement the Settings service, input-language or IME
selection, Albert conversation-language preferences, UI components, font
resolution, or dictionary-based collation. Those remain owned by their
respective system workstreams and should consume this crate through its public
API. Nagi 0.1 formatting currently covers Gregorian dates, clock times,
numbers, percentages, USD/JPY display, and a small symbol-based unit set; it
does not provide timezone conversion, currency conversion, CLDR-wide data, or
linguistic sorting. No process-global host locale or timezone is consulted.
