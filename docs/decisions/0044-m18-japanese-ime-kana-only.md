# ADR 0044: M18 Japanese IME without kanji conversion in Nagi 0.1

Status: accepted for M18 (user decision, 2026-10-05)
Date: 2026-10-05
Milestone: M18 — Albert Browser

## Context

M18 lists IME as a browser deliverable, and the specification requires a
Japanese IME as part of Nagi's first-class Japanese support. Nagi had no input
method: Servo's input-method requests were logged as unavailable. Kanji
(kana-kanji) conversion needs a dictionary. The practical candidates are the
Mozc OSS dictionary, which is large and carries combined licence terms
including IPADIC-derived conditions, and SKK dictionaries, whose main
dictionaries are GPL. Choosing and bundling one is a product and licensing
decision.

## Decision

- Add `user/nagi-ime`, a `no_std` + `alloc` user-space input method with no
  device, display, or OS authority. The client that owns trusted keyboard
  input (Albert) feeds it layout-translated keys and forwards its results.
- Compose hiragana from romaji using common IME conventions (Hepburn and
  Kunrei spellings, doubled consonants, `nn`/`n'`, small kana, long-vowel mark
  and Japanese punctuation). Space cycles candidates, Enter commits, Escape
  cancels, Backspace edits, F6/F7 select hiragana/katakana. Ctrl+Space,
  Zenkaku/Hankaku, Henkan and Muhenkan switch the input mode. The preedit is
  bounded to 64 characters and candidates to 16.
- Candidates come from a `CandidateSource`. Nagi 0.1 ships only
  `KanaCandidates` (hiragana and katakana). **Nagi 0.1 does not include kanji
  conversion**; a dictionary-backed source is deferred to a later release by
  explicit user decision.
- Albert routes page keys through the IME only while Servo reports a focused
  text field (`EmbedderControl::InputMethod`), sends composition
  start/update/end events to Servo, and swallows both press and release of
  keys the IME consumed. Mode keys work without a focused field.

## Consequences

- Japanese text can be entered as hiragana or katakana in web page fields.
  Kanji must be pasted or come from page content until a dictionary source is
  added; that source can be added without changing the engine or Albert.
- The address bar does not use the IME in 0.1: Albert has no search provider,
  addresses are ASCII, and the chrome bitmap font lacks the kana repertoire.
- Input language remains independent of the System language.
- `./nagi m18` verifies a QMP-typed `nihongo` composition committed as
  `にほんご` into a page field.
