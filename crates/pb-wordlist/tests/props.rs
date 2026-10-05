//! Properties of normalising and matching, on any text.
#![allow(clippy::unwrap_used)] // a panic is how a test fails

use pb_wordlist::{WordList, normalize};
use proptest::prelude::*;

proptest! {
    #[test]
    fn normalising_twice_is_normalising_once(s in "\\PC{0,24}") {
        let once = normalize(&s);
        prop_assert_eq!(normalize(&once), once);
    }

    #[test]
    fn a_listed_word_is_found_where_it_stands(
        before in "[a-z ,.]{0,12}",
        word in "[a-zäöüß]{2,10}",
        after in "[a-z ,.]{0,12}",
    ) {
        let text = format!("{before} {word} {after}");
        let found = WordList::new(&[word.as_str()]).find(&text);
        let at = before.len() + 1;
        prop_assert!(found.iter().any(|m| m.range == (at..at + word.len())), "{:?}", found);
    }

    #[test]
    fn matches_are_in_the_text_in_order_and_apart(text in "\\PC{0,64}", list in prop::collection::vec("[a-z*]{1,6}( [a-z]{1,4})?", 0..6)) {
        let found = WordList::new(&list).find(&text);
        for m in &found {
            prop_assert!(m.range.start < m.range.end && m.range.end <= text.len());
            prop_assert!(text.is_char_boundary(m.range.start) && text.is_char_boundary(m.range.end));
        }
        prop_assert!(found.windows(2).all(|w| w[0].range.end <= w[1].range.start));
    }
}
