# Localization integration guide

Nagi's internal language is English. User-visible text is addressed with
stable English message IDs and resolved through `nagi-localization`; English
(`en-US`) and Japanese (`ja-JP`) catalogs are equal first-class resources.

## Calling the runtime

System language and regional formatting are separate inputs. Settings may
therefore select English presentation with Japanese date and number formats.
Keyboard/input language and Albert's conversation language remain independent
settings and are not fields in `LocaleContext`.

```rust
use nagi_localization::{
    LocaleContext, LocaleId, Localizer, MessageArgs, MessageId, NoopDiagnosticSink,
};

let context = LocaleContext::new(
    LocaleId::parse("ja-JP")?, // system language
    LocaleId::parse("en-US")?, // region and formatting conventions
);
let localizer = Localizer::bundled(context, NoopDiagnosticSink)?;
let id = MessageId::new("settings.language.title")?;
let label = localizer.render(&id, &MessageArgs::new())?;
assert_eq!(label, "システムの言語");
```

Use typed `MessageId` values rather than English display text as keys. Pass
interpolation values as plain text; UI clients must insert the result and
arguments through their text APIs rather than treating localized content as
markup.

## Catalog schema and validation

Catalogs are UTF-8 JSON with schema version `1`. The normalized locale in the
resource must be canonical (`en-US`, not `en_us`). `script` and
`font_fallback_scripts` use four-letter ISO 15924 codes; `direction` is `ltr` or
`rtl`. `messages` maps stable IDs such as `common.cancel` and
`settings.language.title` to translated strings. Message IDs use lowercase
dot-separated segments and do not encode the displayed English text. Locale
identifiers normalize case and underscore separators and accept language,
optional script/region, and variant subtags; extensions and private-use
subtags are not supported in schema v1.

Schema v1 supports named string arguments such as `Hello, {name}!`; doubled
braces (`{{` and `}}`) represent literal braces. Every translation must use
the same argument names as its English reference. Plural/select messages are
not inferred from translated prose. When needed, add them as typed message
nodes under a new catalog schema version so each locale keeps the same
message structure.

Validate the bundled reference and Japanese catalogs with:

```sh
cargo run --locked -p nagi-localization --bin localization-check
```

The checker rejects malformed JSON and metadata, duplicate locale/message
IDs, missing or unreferenced Japanese messages, and interpolation mismatches.
Application packages should declare their translation namespace, resource
schema version, supported locale tags, and locale resource files in their
package manifest. Prefix message IDs with that namespace, for example
`app.files.rename`. Parse each package's locale resources and run
`Catalog::validate_namespace` against the declared prefix. Build a package-local
`CatalogSet` and validate its declared locales before packaging. Then merge
the parsed catalogs into the system set with `with_additional_catalogs`; the
merge rejects duplicate IDs and metadata conflicts. An application that
supports only English may validate and merge only its `en-US` resource. The
current App SDK does not yet define those manifest fields.

## Lookup and diagnostics

Lookup tries an exact tag, then progressively broader script/language tags.
`en` and `ja` also resolve to their official Nagi 0.1 catalogs. Other missing
locales fall back to `en-US`. If a key is absent from the selected catalog,
that key falls back to English and emits a structured `FallbackUsed` event. If
the key is absent everywhere, the runtime emits `MissingMessage` and renders
the localized `system.message_unavailable` message; it never displays the raw
ID or an empty value. `DiagnosticSink` is the adapter point for Nagi
Diagnostics and does not require that workstream to be complete.

The runtime does not own persistent preferences. A future Settings service
can replace `LocaleContext.system_language` or `.region` and construct a new
localizer without changing catalog IDs or resource format. A font service may
use catalog script and fallback-script metadata, while the UI Design System
continues to own actual font selection and shaping.

## Formatting limits

`Formatter` consumes an explicit region `LocaleId`; it does not read host
locale, timezone, or process environment. The current deterministic subset is:

- Gregorian date, time, and date-time formatting for the Japanese profile and
  the `en-US` fallback profile;
- grouped integers, fixed-precision decimals, and ratio-based percentages;
- USD and JPY display without conversion;
- byte, meter, kilometer, kilogram, and Celsius symbols;
- a `Collator` interface for a future locale-specific implementation.

Japanese language tags use the Japanese profile; other tags currently use the
`en-US` profile. Timezone conversion, relative dates, currency conversion,
additional currency rules, CLDR-complete formatting, and dictionary-based
Japanese collation are outside this subset. A missing Japanese collator must
not be replaced with an assumption that Unicode codepoint order is linguistic
order.

## Pseudo-localization

`CatalogSet::with_pseudo_locale()` derives `en-XA` from the English catalog.
It marks and expands literal text while preserving named placeholders. Select
`en-XA` in a development build to reveal clipping and fallback paths. Hardcoded
UI strings do not receive pseudo transformation; visual inspection can expose
those remaining call sites.
