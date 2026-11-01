//! Three-word passwords sampled with OS randomness from a curated 2,048-word list.
use crate::storage::StorageError;
use rand::{RngCore, rngs::OsRng};
use std::sync::OnceLock;

const SOURCE: &str = include_str!("../resources/wordlists/words.txt");
const SYMBOLS: &[u8] = b"!@#$%^&*+=?";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PassphraseOptions {
    pub capital: bool,
    pub special: bool,
    pub number: bool,
}

fn words() -> &'static [&'static str] {
    static WORDS: OnceLock<Vec<&'static str>> = OnceLock::new();
    WORDS.get_or_init(|| {
        SOURCE
            .lines()
            .filter_map(|line| {
                let word = line.split_whitespace().last()?;
                // Keep the displayed pattern exactly word-word-word, even if a future
                // dictionary edit includes punctuation, whitespace, or hyphenated terms.
                (!word.is_empty() && word.bytes().all(|c| c.is_ascii_lowercase())).then_some(word)
            })
            .collect()
    })
}

pub fn generate_passphrase(options: PassphraseOptions) -> Result<String, StorageError> {
    let words = words();
    if words.len() < 2048 {
        return Err(StorageError::InvalidWordlist);
    }
    let chosen = [
        words[random_index(words.len())?],
        words[random_index(words.len())?],
        words[random_index(words.len())?],
    ];
    let capital_word = if options.capital {
        Some(random_index(3)?)
    } else {
        None
    };
    let length = chosen.iter().map(|word| word.len()).sum::<usize>() + 4;
    let mut result = String::with_capacity(length);
    for (index, word) in chosen.into_iter().enumerate() {
        if index > 0 {
            result.push('-');
        }
        if capital_word == Some(index) {
            result.push(char::from(word.as_bytes()[0].to_ascii_uppercase()));
            result.push_str(&word[1..]);
        } else {
            result.push_str(word);
        }
    }
    if options.special {
        result.push(char::from(SYMBOLS[random_index(SYMBOLS.len())?]));
    }
    if options.number {
        result.push(char::from(b'0' + random_index(10)? as u8));
    }
    Ok(result)
}

fn random_index(limit: usize) -> Result<usize, StorageError> {
    let range = 1u64 << 32;
    let ceiling = range - range % limit as u64;
    loop {
        let mut bytes = [0; 4];
        OsRng
            .try_fill_bytes(&mut bytes)
            .map_err(|_| StorageError::Randomness)?;
        let value = u64::from(u32::from_le_bytes(bytes));
        if value < ceiling {
            return Ok((value % limit as u64) as usize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn bundled_dictionary_is_large_unique_and_lowercase() {
        assert_eq!(SOURCE.lines().count(), 2048);
        assert_eq!(words().len(), 2048);
        assert!(
            ["rainy", "october", "simply"]
                .into_iter()
                .all(|word| words().contains(&word))
        );
        assert_eq!(
            words().iter().copied().collect::<HashSet<_>>().len(),
            words().len()
        );
        assert!(
            words()
                .iter()
                .all(|word| word.bytes().all(|c| c.is_ascii_lowercase()))
        );
    }

    #[test]
    fn default_is_three_dictionary_words_separated_by_hyphens() {
        let dictionary = words().iter().copied().collect::<HashSet<_>>();
        let mut samples = HashSet::new();
        for _ in 0..32 {
            let password = generate_passphrase(PassphraseOptions::default()).unwrap();
            let parts = password.split('-').collect::<Vec<_>>();
            assert_eq!(parts.len(), 3);
            assert!(parts.into_iter().all(|part| dictionary.contains(part)));
            samples.insert(password);
        }
        assert!(
            samples.len() > 1,
            "generation must not return a fixed example"
        );
    }

    #[test]
    fn every_option_combination_matches_its_character_requirements() {
        for capital in [false, true] {
            for special in [false, true] {
                for number in [false, true] {
                    let password = generate_passphrase(PassphraseOptions {
                        capital,
                        special,
                        number,
                    })
                    .unwrap();
                    assert_eq!(
                        password.bytes().filter(|c| c.is_ascii_uppercase()).count(),
                        usize::from(capital)
                    );
                    assert_eq!(
                        password.bytes().filter(|c| SYMBOLS.contains(c)).count(),
                        usize::from(special)
                    );
                    assert_eq!(
                        password.bytes().filter(|c| c.is_ascii_digit()).count(),
                        usize::from(number)
                    );
                    assert_eq!(password.matches('-').count(), 2);
                    if number {
                        assert!(password.as_bytes().last().unwrap().is_ascii_digit());
                    }
                    let letters = password.trim_end_matches(|c: char| !c.is_ascii_alphabetic());
                    for word in letters.split('-') {
                        assert!(words().contains(&word.to_ascii_lowercase().as_str()));
                    }
                }
            }
        }
    }
}
