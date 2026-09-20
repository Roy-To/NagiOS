# ADR 0015: Common Language Architecture

- Status: Accepted
- Scope: Nagi OS 0.1 common platform and first-party user space
- Date: 2026-09-19

## Decision

Nagi uses English as its canonical internal language, while English (`en-US`)
and Japanese (`ja-JP`) are equally supported first-class user languages.

Internal identifiers, APIs, IPC/RPC names, schemas, configuration keys,
localization keys, event names, commands, developer diagnostics, log
identifiers, and machine-readable error identifiers remain English-based.
User-facing strings are resolved through shared localization resources with
stable English-based keys and an `en-US` fallback.

System language, region/locale, input language/keyboard, and Albert/AI
conversation language are independent settings. UTF-8 is the default internal
text encoding. The design must allow future language packs without requiring
an OS-wide code rewrite.

## Rationale

This preserves stable interfaces and diagnostics while making Japanese a
supported Nagi user language rather than a secondary translation layer. It
also prevents display language, regional formatting, keyboard/IME behavior,
and AI conversation behavior from becoming inseparably coupled.

## Consequences

- Nagi first-party applications share one localization architecture.
- Missing selected-locale entries resolve to English and are diagnosable in
  development.
- Terminal command syntax and machine-facing identifiers remain stable.
- A complete localization resource catalog and settings service remain future
  implementation work; this ADR does not claim those deliverables exist.
- Existing Japanese UTF-8 UI/input behavior remains valid and is not replaced.
