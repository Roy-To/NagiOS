use nagi_m25_whisper_tests::metrics::{character_errors, edit_distance, normalize};

#[test]
fn normalization_drops_punctuation_and_folds_width() {
    assert_eq!(normalize("ＡＢ、c。 d！"), vec!['a', 'b', 'c', 'd']);
    assert_eq!(
        normalize("「東京」・大阪…"),
        "東京大阪".chars().collect::<Vec<_>>()
    );
}

#[test]
fn edit_distance_counts_substitutions_insertions_deletions() {
    let chars = |text: &str| text.chars().collect::<Vec<_>>();
    assert_eq!(edit_distance(&chars("今日は晴れ"), &chars("今日は晴れ")), 0);
    assert_eq!(edit_distance(&chars("今日は晴れ"), &chars("今日も晴れ")), 1);
    assert_eq!(edit_distance(&chars("abc"), &chars("")), 3);
    assert_eq!(edit_distance(&chars(""), &chars("ab")), 2);
    assert_eq!(
        character_errors("多くの場合、海外", "多くの場合海外です"),
        (2, 7)
    );
}
