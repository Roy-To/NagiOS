//! Incremental romaji-to-hiragana composition.

use alloc::string::String;

/// Romaji sequences accepted by the composer, using common Japanese IME
/// conventions (Hepburn and Kunrei spellings, `x`/`l` small kana, `nn`/`n'`).
const TABLE: &[(&str, &str)] = &[
    ("a", "あ"),
    ("i", "い"),
    ("u", "う"),
    ("e", "え"),
    ("o", "お"),
    ("ka", "か"),
    ("ki", "き"),
    ("ku", "く"),
    ("ke", "け"),
    ("ko", "こ"),
    ("ga", "が"),
    ("gi", "ぎ"),
    ("gu", "ぐ"),
    ("ge", "げ"),
    ("go", "ご"),
    ("sa", "さ"),
    ("si", "し"),
    ("shi", "し"),
    ("su", "す"),
    ("se", "せ"),
    ("so", "そ"),
    ("za", "ざ"),
    ("zi", "じ"),
    ("ji", "じ"),
    ("zu", "ず"),
    ("ze", "ぜ"),
    ("zo", "ぞ"),
    ("ta", "た"),
    ("ti", "ち"),
    ("chi", "ち"),
    ("tu", "つ"),
    ("tsu", "つ"),
    ("te", "て"),
    ("to", "と"),
    ("da", "だ"),
    ("di", "ぢ"),
    ("du", "づ"),
    ("de", "で"),
    ("do", "ど"),
    ("na", "な"),
    ("ni", "に"),
    ("nu", "ぬ"),
    ("ne", "ね"),
    ("no", "の"),
    ("ha", "は"),
    ("hi", "ひ"),
    ("hu", "ふ"),
    ("fu", "ふ"),
    ("he", "へ"),
    ("ho", "ほ"),
    ("ba", "ば"),
    ("bi", "び"),
    ("bu", "ぶ"),
    ("be", "べ"),
    ("bo", "ぼ"),
    ("pa", "ぱ"),
    ("pi", "ぴ"),
    ("pu", "ぷ"),
    ("pe", "ぺ"),
    ("po", "ぽ"),
    ("ma", "ま"),
    ("mi", "み"),
    ("mu", "む"),
    ("me", "め"),
    ("mo", "も"),
    ("ya", "や"),
    ("yu", "ゆ"),
    ("ye", "いぇ"),
    ("yo", "よ"),
    ("ra", "ら"),
    ("ri", "り"),
    ("ru", "る"),
    ("re", "れ"),
    ("ro", "ろ"),
    ("wa", "わ"),
    ("wi", "うぃ"),
    ("we", "うぇ"),
    ("wo", "を"),
    ("nn", "ん"),
    ("n'", "ん"),
    ("va", "ゔぁ"),
    ("vi", "ゔぃ"),
    ("vu", "ゔ"),
    ("ve", "ゔぇ"),
    ("vo", "ゔぉ"),
    ("fa", "ふぁ"),
    ("fi", "ふぃ"),
    ("fe", "ふぇ"),
    ("fo", "ふぉ"),
    ("kya", "きゃ"),
    ("kyu", "きゅ"),
    ("kyo", "きょ"),
    ("gya", "ぎゃ"),
    ("gyu", "ぎゅ"),
    ("gyo", "ぎょ"),
    ("sya", "しゃ"),
    ("syu", "しゅ"),
    ("syo", "しょ"),
    ("sha", "しゃ"),
    ("shu", "しゅ"),
    ("she", "しぇ"),
    ("sho", "しょ"),
    ("zya", "じゃ"),
    ("zyu", "じゅ"),
    ("zyo", "じょ"),
    ("ja", "じゃ"),
    ("ju", "じゅ"),
    ("je", "じぇ"),
    ("jo", "じょ"),
    ("jya", "じゃ"),
    ("jyu", "じゅ"),
    ("jyo", "じょ"),
    ("tya", "ちゃ"),
    ("tyu", "ちゅ"),
    ("tyo", "ちょ"),
    ("cya", "ちゃ"),
    ("cyu", "ちゅ"),
    ("cyo", "ちょ"),
    ("cha", "ちゃ"),
    ("chu", "ちゅ"),
    ("che", "ちぇ"),
    ("cho", "ちょ"),
    ("dya", "ぢゃ"),
    ("dyu", "ぢゅ"),
    ("dyo", "ぢょ"),
    ("thi", "てぃ"),
    ("dhi", "でぃ"),
    ("nya", "にゃ"),
    ("nyu", "にゅ"),
    ("nyo", "にょ"),
    ("hya", "ひゃ"),
    ("hyu", "ひゅ"),
    ("hyo", "ひょ"),
    ("bya", "びゃ"),
    ("byu", "びゅ"),
    ("byo", "びょ"),
    ("pya", "ぴゃ"),
    ("pyu", "ぴゅ"),
    ("pyo", "ぴょ"),
    ("mya", "みゃ"),
    ("myu", "みゅ"),
    ("myo", "みょ"),
    ("rya", "りゃ"),
    ("ryu", "りゅ"),
    ("ryo", "りょ"),
    ("xa", "ぁ"),
    ("xi", "ぃ"),
    ("xu", "ぅ"),
    ("xe", "ぇ"),
    ("xo", "ぉ"),
    ("la", "ぁ"),
    ("li", "ぃ"),
    ("lu", "ぅ"),
    ("le", "ぇ"),
    ("lo", "ぉ"),
    ("xya", "ゃ"),
    ("xyu", "ゅ"),
    ("xyo", "ょ"),
    ("lya", "ゃ"),
    ("lyu", "ゅ"),
    ("lyo", "ょ"),
    ("xtu", "っ"),
    ("ltu", "っ"),
    ("xtsu", "っ"),
    ("ltsu", "っ"),
    ("xwa", "ゎ"),
    ("lwa", "ゎ"),
];

/// Punctuation converted when no romaji is pending.
fn punctuation(character: char) -> Option<char> {
    Some(match character {
        '-' => 'ー',
        ',' => '、',
        '.' => '。',
        '[' => '「',
        ']' => '」',
        '/' => '・',
        '?' => '？',
        '!' => '！',
        '~' => '〜',
        _ => return None,
    })
}

fn is_vowel(character: char) -> bool {
    matches!(character, 'a' | 'i' | 'u' | 'e' | 'o')
}

fn lookup(pending: &str) -> Option<&'static str> {
    TABLE
        .iter()
        .find(|(romaji, _)| *romaji == pending)
        .map(|(_, kana)| *kana)
}

fn is_prefix(pending: &str) -> bool {
    TABLE.iter().any(|(romaji, _)| romaji.starts_with(pending))
}

/// Composition buffer: converted kana followed by not-yet-converted romaji.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RomajiComposer {
    kana: String,
    pending: String,
}

impl RomajiComposer {
    pub fn is_empty(&self) -> bool {
        self.kana.is_empty() && self.pending.is_empty()
    }

    /// The text to show while composing.
    pub fn preedit(&self) -> String {
        let mut text = self.kana.clone();
        text.push_str(&self.pending);
        text
    }

    pub fn char_count(&self) -> usize {
        self.kana.chars().count() + self.pending.chars().count()
    }

    /// Feed one typed ASCII character. Returns `false` if it is not
    /// composable (the caller should pass it through).
    pub fn push(&mut self, character: char) -> bool {
        let character = character.to_ascii_lowercase();
        if self.pending.is_empty() {
            if let Some(mark) = punctuation(character) {
                self.kana.push(mark);
                return true;
            }
        }
        if !(character.is_ascii_lowercase() || (character == '\'' && self.pending == "n")) {
            if let Some(mark) = punctuation(character) {
                self.flush_pending();
                self.kana.push(mark);
                return true;
            }
            return false;
        }
        self.pending.push(character);
        self.resolve();
        true
    }

    fn resolve(&mut self) {
        while !self.pending.is_empty() {
            if let Some(kana) = lookup(&self.pending) {
                self.kana.push_str(kana);
                self.pending.clear();
                return;
            }
            let mut chars = self.pending.chars();
            let first = chars.next().unwrap_or_default();
            let second = chars.next();
            match second {
                // "kk" -> っ + "k"
                Some(second) if second == first && first != 'n' && !is_vowel(first) => {
                    self.kana.push('っ');
                    self.pending.remove(0);
                    continue;
                }
                // "tch" -> っ + "ch"
                Some('c') if first == 't' && self.pending.as_bytes().get(2) == Some(&b'h') => {
                    self.kana.push('っ');
                    self.pending.remove(0);
                    continue;
                }
                // "nk" -> ん + "k"
                Some(second) if first == 'n' && !is_vowel(second) && second != 'y' => {
                    self.kana.push('ん');
                    self.pending.remove(0);
                    continue;
                }
                _ => {}
            }
            // "tc" may still become "tch" (っち).
            if is_prefix(&self.pending) || self.pending == "tc" {
                return;
            }
            // Not a valid romaji start: keep the first letter literally.
            self.kana.push(first);
            self.pending.remove(0);
        }
    }

    /// Convert a trailing lone `n` to ん and keep other pending letters as
    /// typed. Used before committing or switching to punctuation.
    pub fn flush_pending(&mut self) {
        if self.pending == "n" {
            self.kana.push('ん');
        } else {
            let pending = core::mem::take(&mut self.pending);
            self.kana.push_str(&pending);
        }
        self.pending.clear();
    }

    /// Remove the last pending letter, or else the last kana.
    pub fn backspace(&mut self) -> bool {
        self.pending.pop().is_some() || self.kana.pop().is_some()
    }

    /// Finish composition, returning the hiragana reading.
    pub fn finish(&mut self) -> String {
        self.flush_pending();
        core::mem::take(&mut self.kana)
    }

    pub fn clear(&mut self) {
        self.kana.clear();
        self.pending.clear();
    }

    /// Replace the buffer with an already converted reading.
    pub fn set_reading(&mut self, reading: &str) {
        self.kana.clear();
        self.kana.push_str(reading);
        self.pending.clear();
    }
}

/// Convert hiragana in `text` to katakana; other characters are unchanged.
pub fn to_katakana(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\u{3041}'..='\u{3096}' => char::from_u32(character as u32 + 0x60).unwrap_or(character),
            _ => character,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compose(input: &str) -> String {
        let mut composer = RomajiComposer::default();
        for character in input.chars() {
            assert!(composer.push(character), "{character}");
        }
        composer.finish()
    }

    #[test]
    fn converts_common_words() {
        assert_eq!(compose("nihongo"), "にほんご");
        assert_eq!(compose("konnnichiha"), "こんにちは");
        assert_eq!(compose("arigatou"), "ありがとう");
        assert_eq!(compose("toukyou"), "とうきょう");
        assert_eq!(compose("shinbun"), "しんぶん");
        assert_eq!(compose("kitte"), "きって");
        assert_eq!(compose("matcha"), "まっちゃ");
        assert_eq!(compose("kan'i"), "かんい");
        assert_eq!(compose("jisho"), "じしょ");
        assert_eq!(compose("fairu"), "ふぁいる");
        assert_eq!(compose("ra-men"), "らーめん");
        assert_eq!(compose("hai."), "はい。");
        assert_eq!(compose("xtsu"), "っ");
    }

    #[test]
    fn preedit_shows_pending_romaji() {
        let mut composer = RomajiComposer::default();
        for character in "nihonk".chars() {
            composer.push(character);
        }
        assert_eq!(composer.preedit(), "にほんk");
        assert!(composer.backspace());
        assert_eq!(composer.preedit(), "にほん");
        assert!(composer.backspace());
        assert_eq!(composer.preedit(), "にほ");
    }

    #[test]
    fn invalid_sequences_keep_their_letters() {
        assert_eq!(compose("q"), "q");
        assert_eq!(compose("qa"), "qあ");
        assert_eq!(compose("kq"), "kq");
    }

    #[test]
    fn non_composable_characters_are_rejected() {
        let mut composer = RomajiComposer::default();
        assert!(!composer.push('1'));
        assert!(!composer.push(' '));
        assert!(composer.is_empty());
    }

    #[test]
    fn katakana_conversion_maps_hiragana_only() {
        assert_eq!(to_katakana("にほんご"), "ニホンゴ");
        assert_eq!(to_katakana("らーめんA"), "ラーメンA");
        assert_eq!(to_katakana("ゔ"), "ヴ");
    }
}
