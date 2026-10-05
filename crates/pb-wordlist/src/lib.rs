//! Word lists: whether a chat message (or a transcript) contains a listed word or phrase. Case, look-alike characters
//! (`f@ck`, `5hit`, Cyrillic letters), invisible characters, stretched letters (`fuuuuck`) and spaced-out letters
//! (`f u c k`) do not hide a word; a word inside another (`ass` in `class`) is not one. Pure: no I/O.

pub mod v1;

pub use v1::*;
