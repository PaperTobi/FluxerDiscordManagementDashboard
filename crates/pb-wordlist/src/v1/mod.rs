//! Version 1.
//!
//! A text is cut into tokens (runs of letters, digits and the look-alikes `@ $ !`); each token is normalised (NFKC,
//! lower case, invisible characters dropped, look-alikes folded when the token has a letter) and kept as runs of equal
//! characters. A listed word matches a token when the characters are the same and every run is as long as in the word,
//! or stretched (three or more); `*` at either end of a word leaves that end open. A phrase matches its words in
//! consecutive tokens. Single letters written apart (`f u c k`) are also read as one token.

use std::ops::Range;

use unicode_normalization::UnicodeNormalization;

/// A listed word or phrase in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// The entry of the list, as written there.
    pub pattern: String,
    /// Where in the text (bytes).
    pub range: Range<usize>,
}

/// Compiled word lists.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WordList {
    patterns: Vec<Pattern>,
}

#[derive(Debug, Clone, PartialEq)]
struct Pattern {
    source: String,
    words: Vec<Word>,
}

#[derive(Debug, Clone, PartialEq)]
struct Word {
    runs: Vec<(char, usize)>,
    open_start: bool,
    open_end: bool,
}

/// A token of the text: its normalised runs and where it is.
#[derive(Debug, Clone)]
struct Token {
    runs: Vec<(char, usize)>,
    range: Range<usize>,
}

/// Characters that are never seen (they could split a word without showing).
fn invisible(c: char) -> bool {
    matches!(c, '\u{00AD}' | '\u{034F}' | '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}')
}

/// Characters that are part of a token.
fn in_token(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '@' | '$' | '!') || invisible(c)
}

/// The letter a look-alike stands for.
fn fold(c: char) -> char {
    match c {
        '0' => 'o',
        '1' | '!' => 'i',
        '3' => 'e',
        '4' | '@' => 'a',
        '5' | '$' => 's',
        '7' => 't',
        // Cyrillic and Greek letters that look Latin.
        'а' | 'α' => 'a',
        'е' | 'ε' => 'e',
        'о' | 'ο' => 'o',
        'р' | 'ρ' => 'p',
        'с' => 'c',
        'у' => 'y',
        'х' | 'χ' => 'x',
        'і' | 'ι' => 'i',
        'ѕ' => 's',
        'ј' => 'j',
        'к' | 'κ' => 'k',
        'н' => 'h',
        'т' | 'τ' => 't',
        'в' | 'β' => 'b',
        'м' => 'm',
        'ν' => 'v',
        other => other,
    }
}

/// A word normalised: NFKC, lower case, invisible characters dropped, and look-alikes folded when it has a letter or
/// one of `@ $ !` (`100` stays a number, `@$$` is a word).
pub fn normalize(word: &str) -> String {
    let lower: String = word
        .nfkc()
        .flat_map(char::to_lowercase)
        .filter(|c| !invisible(*c))
        .collect();
    if lower.chars().any(|c| c.is_alphabetic() || matches!(c, '@' | '$' | '!')) {
        lower.chars().map(fold).collect()
    } else {
        lower
    }
}

/// Runs of equal characters.
fn runs(s: &str) -> Vec<(char, usize)> {
    let mut out: Vec<(char, usize)> = Vec::new();
    for c in s.chars() {
        match out.last_mut() {
            Some((last, n)) if *last == c => *n += 1,
            _ => out.push((c, 1)),
        }
    }
    out
}

/// The text's tokens; `!` at a token's ends is punctuation, not an `i`.
fn tokens(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let push = |s: usize, e: usize, out: &mut Vec<Token>| {
        let raw = &text[s..e];
        let trimmed = raw.trim_matches('!');
        if trimmed.is_empty() {
            return;
        }
        let s = s + (raw.len() - raw.trim_start_matches('!').len());
        let e = s + trimmed.len();
        let runs = runs(&normalize(trimmed));
        if !runs.is_empty() {
            out.push(Token { runs, range: s..e });
        }
    };
    for (i, c) in text.char_indices() {
        match (in_token(c), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                push(s, i, &mut out);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        push(s, text.len(), &mut out);
    }
    out
}

/// Single letters written apart (three or more: `f u c k`, `f.u.c.k`) as one token each run; other tokens stay.
fn joined(tokens: &[Token]) -> Vec<Token> {
    let single = |t: &Token| t.runs.len() == 1 && t.runs[0].1 == 1 && t.runs[0].0.is_alphabetic();
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let mut j = i;
        while j < tokens.len() && single(&tokens[j]) {
            j += 1;
        }
        if j - i >= 3 {
            let letters: String = tokens[i..j].iter().map(|t| t.runs[0].0).collect();
            out.push(Token {
                runs: runs(&letters),
                range: tokens[i].range.start..tokens[j - 1].range.end,
            });
            i = j;
        } else {
            out.push(tokens[i].clone());
            i += 1;
        }
    }
    out
}

impl Word {
    fn parse(s: &str) -> Option<Word> {
        let open_start = s.starts_with('*');
        let open_end = s.ends_with('*');
        let core = s.trim_matches('*');
        let runs = runs(&normalize(core));
        (!runs.is_empty()).then_some(Word {
            runs,
            open_start,
            open_end,
        })
    }

    /// Whether the word matches the token.
    fn matches(&self, t: &Token) -> bool {
        let (p, tk) = (&self.runs, &t.runs);
        let m = p.len();
        if m > tk.len() {
            return false;
        }
        let first = if self.open_start { 0..=tk.len() - m } else { 0..=0 };
        first.into_iter().any(|i| {
            if !self.open_end && i + m != tk.len() {
                return false;
            }
            p.iter()
                .zip(&tk[i..i + m])
                .enumerate()
                .all(|(j, ((pc, pn), (tc, tn)))| {
                    // An open end may continue the run (`shit*` in `shitty`); a run may be stretched (`fuuuck`).
                    let open = (j == 0 && self.open_start) || (j == m - 1 && self.open_end);
                    pc == tc && (tn == pn || (open && tn >= pn) || (*tn >= 3 && tn >= pn))
                })
        })
    }
}

impl WordList {
    /// Compiles a list (entries without letters or digits are left out).
    pub fn new<S: AsRef<str>>(entries: &[S]) -> WordList {
        WordList {
            patterns: entries
                .iter()
                .filter_map(|e| {
                    let source = e.as_ref().trim();
                    let words: Vec<Word> = source.split_whitespace().filter_map(Word::parse).collect();
                    (!words.is_empty()).then(|| Pattern {
                        source: source.to_owned(),
                        words,
                    })
                })
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Every listed word or phrase in `text`, in the order they appear (one match per place).
    pub fn find(&self, text: &str) -> Vec<Match> {
        if self.patterns.is_empty() {
            return Vec::new();
        }
        let plain = tokens(text);
        let joined = joined(&plain);
        let mut out: Vec<Match> = Vec::new();
        for toks in [&plain, &joined] {
            for p in &self.patterns {
                let n = p.words.len();
                for i in 0..toks.len().saturating_sub(n - 1) {
                    if p.words.iter().zip(&toks[i..i + n]).all(|(w, t)| w.matches(t)) {
                        let range = toks[i].range.start..toks[i + n - 1].range.end;
                        if !out
                            .iter()
                            .any(|m| m.range.start < range.end && range.start < m.range.end)
                        {
                            out.push(Match {
                                pattern: p.source.clone(),
                                range,
                            });
                        }
                    }
                }
            }
        }
        out.sort_by_key(|m| m.range.start);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(list: &[&str], text: &str) -> Vec<String> {
        WordList::new(list)
            .find(text)
            .into_iter()
            .map(|m| text[m.range].to_owned())
            .collect()
    }

    #[test]
    fn whole_words_only_unless_open() {
        assert_eq!(found(&["ass"], "Kiss my ass, class."), ["ass"]);
        assert!(found(&["ass"], "a classic passage").is_empty());
        assert_eq!(found(&["fuck*"], "fucking hell"), ["fucking"]);
        assert_eq!(found(&["*loch"], "du Arschloch"), ["Arschloch"]);
        assert_eq!(found(&["*schei*"], "verscheißert"), ["verscheißert"]);
        assert_eq!(found(&["shit*"], "that is shitty"), ["shitty"]);
    }

    #[test]
    fn disguises_do_not_hide_a_word() {
        let list = ["fuck", "shit"];
        assert_eq!(found(&list, "F@CK this"), Vec::<String>::new());
        assert_eq!(found(&["ass"], "kiss my @$$"), ["@$$"]);
        assert_eq!(found(&list, "FUCK this"), ["FUCK"]);
        assert_eq!(found(&list, "5h1t!"), ["5h1t"]);
        assert_eq!(found(&list, "sh!t happens"), ["sh!t"]);
        assert_eq!(found(&list, "fuuuuuck"), ["fuuuuuck"]);
        assert_eq!(found(&list, "f u c k you"), ["f u c k"]);
        assert_eq!(found(&list, "f.u.c.k"), ["f.u.c.k"]);
        assert_eq!(found(&list, "fu\u{200B}ck"), ["fu\u{200B}ck"]);
        // Cyrillic letters and full-width forms.
        assert_eq!(found(&list, "shіt"), ["shіt"]);
        assert_eq!(found(&list, "ｆｕｃｋ"), ["ｆｕｃｋ"]);
    }

    #[test]
    fn letters_written_twice_are_not_stretched() {
        // `god` is not in `good`; three or more is stretching.
        assert!(found(&["god"], "good morning").is_empty());
        assert_eq!(found(&["god"], "oh gooood"), ["gooood"]);
        assert!(found(&["ass"], "as I said").is_empty());
    }

    #[test]
    fn numbers_stay_numbers() {
        assert!(found(&["ioo"], "I have 100 of them").is_empty());
        assert_eq!(found(&["b4"], "see you b4"), ["b4"]);
    }

    #[test]
    fn phrases_match_words_in_a_row() {
        let list = ["shut up", "halt die fresse"];
        assert_eq!(found(&list, "Oh SHUT... up!"), ["SHUT... up"]);
        assert_eq!(found(&list, "Halt  die Fresse, bitte"), ["Halt  die Fresse"]);
        assert!(found(&list, "shut the door, up there").is_empty());
    }

    #[test]
    fn every_place_once_in_order() {
        let list = WordList::new(&["fuck", "fuck*", "", "***"]);
        assert_eq!(list.patterns.len(), 2);
        let m = list.find("fuck it, fucking fuck");
        assert_eq!(m.len(), 3);
        assert!(m.windows(2).all(|w| w[0].range.end <= w[1].range.start));
        assert!(WordList::new::<&str>(&[]).find("fuck").is_empty());
    }
}
