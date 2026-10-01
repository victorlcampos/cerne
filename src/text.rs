//! Matching what a person types against names, regardless of case, accents
//! and punctuation.

/// Lowercase ASCII words separated by single spaces: "Rosé Pine" and
/// "rose-pine" both become "rose pine", "Saint-Étienne" "saint etienne".
/// Apostrophes join words ("N'Djamena" → "ndjamena").
pub fn fold(text: &str) -> String {
    let ascii = deunicode::deunicode(text).replace('\'', "");
    ascii
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_accents_case_and_punctuation() {
        assert_eq!(fold("Rosé Pine"), "rose pine");
        assert_eq!(fold("SynthWave '84"), "synthwave 84");
        assert_eq!(fold("  São Paulo, Brazil "), "sao paulo brazil");
        assert_eq!(fold("  Saint-Étienne "), "saint etienne");
        assert_eq!(fold("N'Djamena"), "ndjamena");
        assert_eq!(fold("Zürich"), "zurich");
        assert_eq!(fold("Tōkyō"), "tokyo");
        assert_eq!(fold("--"), "");
    }
}
