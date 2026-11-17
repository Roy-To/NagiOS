//! Character error rate and process memory readings for the evaluation.

/// Normalizes Japanese/ASCII text for character error rate: drops whitespace
/// and punctuation, folds full-width ASCII to half-width, and lowercases ASCII.
/// Kana/kanji are compared as written (no reading conversion).
pub fn normalize(text: &str) -> Vec<char> {
    text.chars()
        .map(|character| match character {
            '\u{FF01}'..='\u{FF5E}' => {
                char::from_u32(character as u32 - 0xFEE0).unwrap_or(character)
            }
            '\u{3000}' => ' ',
            other => other,
        })
        .map(|character| character.to_ascii_lowercase())
        .filter(|character| !character.is_whitespace() && !is_punctuation(*character))
        .collect()
}

fn is_punctuation(character: char) -> bool {
    character.is_ascii_punctuation()
        || matches!(
            character,
            '、' | '。'
                | '・'
                | '「'
                | '」'
                | '『'
                | '』'
                | '（'
                | '）'
                | '〈'
                | '〉'
                | '《'
                | '》'
                | '【'
                | '】'
                | '〔'
                | '〕'
                | '…'
                | '‥'
                | '〜'
                | '～'
                | '―'
                | '‐'
                | '“'
                | '”'
                | '‘'
                | '’'
                | '，'
                | '．'
                | '！'
                | '？'
                | '：'
                | '；'
        )
}

/// Levenshtein distance over characters.
pub fn edit_distance(reference: &[char], hypothesis: &[char]) -> usize {
    let mut previous: Vec<usize> = (0..=hypothesis.len()).collect();
    let mut current = vec![0; hypothesis.len() + 1];
    for (row, reference_char) in reference.iter().enumerate() {
        current[0] = row + 1;
        for (column, hypothesis_char) in hypothesis.iter().enumerate() {
            let substitution = previous[column] + usize::from(reference_char != hypothesis_char);
            current[column + 1] = substitution
                .min(previous[column + 1] + 1)
                .min(current[column] + 1);
        }
        core::mem::swap(&mut previous, &mut current);
    }
    previous[hypothesis.len()]
}

/// Returns (edits, reference length) after normalization.
pub fn character_errors(reference: &str, hypothesis: &str) -> (usize, usize) {
    let reference = normalize(reference);
    let hypothesis = normalize(hypothesis);
    (edit_distance(&reference, &hypothesis), reference.len())
}

/// Resident set size and its peak (`VmRSS`, `VmHWM`) in KiB, Linux only.
pub fn rss_kib() -> Option<(u64, u64)> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let field = |name: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|value| value.parse::<u64>().ok())
    };
    Some((field("VmRSS:")?, field("VmHWM:")?))
}

/// Resets the kernel's peak-RSS counter so the next `VmHWM` reading covers
/// only the following phase. Returns false where unsupported.
pub fn reset_peak_rss() -> bool {
    std::fs::write("/proc/self/clear_refs", b"5").is_ok()
}
