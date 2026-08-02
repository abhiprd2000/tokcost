//! Hand-rolled byte-pair-encoding tokenizer, dependency-free.
//!
//! Two things live here:
//!
//! 1. `split`: a pretokenizer that produces the same *final token IDs* as
//!    tiktoken's `cl100k_base` / `o200k_base` splitting regexes, without a
//!    regex engine. It does **not** claim to reproduce those regexes'
//!    exact boundary placement — it's deliberately coarser in one specific
//!    way: `o200k_base`'s real word alternatives (`UP*LO+` / `UP+LO*`) only
//!    ever match a run of *homogeneous case*, so the real splitter cuts at
//!    every lowercase-to-uppercase transition (`myVariable` -> `my` |
//!    `Variable`). This splitter instead greedily consumes the whole
//!    alphabetic run regardless of case, so `myVariable` stays one piece.
//!
//!    That's safe rather than approximate, by construction: tiktoken (and
//!    BPE tokenizers generally) are trained by pretokenizing the training
//!    corpus with this same regex *first*, then learning merge rules
//!    independently within each resulting piece — the trainer never
//!    observes two pretokens' bytes adjacent to each other, for any input,
//!    because the regex structurally forbids it. So no rank-table entry
//!    can ever bridge a boundary the real splitter always draws, at *any*
//!    case transition, in *any* text. `byte_pair_merge` only merges a pair
//!    when `ranks` has an entry for it; since no such entry can exist
//!    across that boundary, feeding it a coarser piece that merely omits
//!    the boundary yields the identical sequence of merges as feeding it
//!    the two real pieces separately — the algorithm never gets a chance
//!    to satisfy a merge there, because there's provably nothing to find.
//!
//!    The converse does *not* hold: adding a boundary the real splitter
//!    does not have (a *finer* split) can block a merge the real tokenizer
//!    would have made, which does change the output. So coarser is safe,
//!    finer is not — every rule in this file errs only on the coarse side.
//!
//!    Contractions: both vocabularies match contraction suffixes
//!    case-insensitively — cl100k_base's group is `'(?i:[sdmt]|ll|ve|re)`
//!    and o200k_base's is `(?i:'s|'t|'re|'ve|'m|'ll|'d)` (verified against
//!    tiktoken 0.13.0's pattern strings and its actual split output on
//!    inputs like `x'Sy` and `DON'T`, both pinned by golden fixtures).
//!    `match_contraction` still takes an explicit case-insensitivity flag
//!    so each call site *states* which regex group it's implementing
//!    rather than baking the sameness in silently — if a future vocab ever
//!    differs, the knob already exists.
//!
//! 2. `byte_pair_merge`: the reference O(n^2) BPE merge tiktoken itself ships
//!    as its "educational" implementation (repeatedly merge the lowest-rank
//!    adjacent pair until none remain). It's not the fastest possible
//!    algorithm, but it's ~20 lines, obviously correct, and fast enough since
//!    pretokenization already keeps individual pieces short.
//!
//! Known approximation (a real one, unlike the case-run coarsening above):
//! `o200k_base`'s split pattern distinguishes fine Unicode letter
//! subcategories (Lu/Ll/Lt/Lm/Lo) and combining marks (M). Rust's `char`
//! gives us `is_alphabetic`/`is_uppercase`/`is_lowercase` for free (no
//! crate needed, these ship in libcore), which covers the letter side
//! exactly. Combining marks have no libcore accessor, so `is_mark` below
//! recognizes only the common combining-diacritic Unicode blocks
//! (Latin-adjacent). Rare scripts whose combining marks fall outside those
//! blocks may split — and in this case may actually *encode* —
//! differently than real o200k_base. This is disclosed in the README and
//! pinned down by golden tests.

use std::collections::HashMap;

/// A token id, as tiktoken calls it a "rank".
pub type Rank = u32;

/// Maps a byte sequence (a merged token, or a lone byte) to its rank.
pub type Ranks = HashMap<Vec<u8>, Rank>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    Cl100kBase,
    O200kBase,
}

// ---------------------------------------------------------------------
// BPE merge
// ---------------------------------------------------------------------

/// Merge `piece` (raw bytes of one pretokenized chunk) into ranks by
/// repeatedly combining the adjacent pair whose merged form has the lowest
/// rank, until no adjacent pair has a known merged rank.
///
/// Requires every single byte 0..=255 to be present in `ranks` (true for
/// both tiktoken vocabularies), so the loop always terminates with every
/// remaining part resolvable to a rank.
pub fn byte_pair_merge(piece: &[u8], ranks: &Ranks) -> Vec<Rank> {
    if piece.is_empty() {
        return Vec::new();
    }

    let mut parts: Vec<Vec<u8>> = piece.iter().map(|&b| vec![b]).collect();

    loop {
        let mut best: Option<(usize, Rank)> = None;
        for i in 0..parts.len() - 1 {
            let mut pair = parts[i].clone();
            pair.extend_from_slice(&parts[i + 1]);
            if let Some(&rank) = ranks.get(&pair) {
                if best.is_none_or(|(_, best_rank)| rank < best_rank) {
                    best = Some((i, rank));
                }
            }
        }
        let Some((i, _)) = best else { break };
        let mut merged = std::mem::take(&mut parts[i]);
        merged.extend_from_slice(&parts[i + 1]);
        parts.splice(i..=i + 1, [merged]);
    }

    parts.iter().map(|p| ranks[p]).collect()
}

/// Split `text` into pretokenized pieces and BPE-encode each one.
pub fn encode_ordinary(text: &str, ranks: &Ranks, pattern: Pattern) -> Vec<Rank> {
    let mut out = Vec::new();
    for piece in split(text, pattern) {
        out.extend(byte_pair_merge(piece.as_bytes(), ranks));
    }
    out
}

// ---------------------------------------------------------------------
// Pretokenizer
// ---------------------------------------------------------------------

/// Split `text` the way tiktoken's split regex would, without a regex
/// engine. Each returned slice is one pretoken, in order, covering `text`
/// exactly (concatenating the results reproduces the input).
pub fn split(text: &str, pattern: Pattern) -> Vec<&str> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let n = chars.len();
    if n == 0 {
        return Vec::new();
    }
    let end = text.len();
    let byte_at = |k: usize| if k < n { chars[k].0 } else { end };

    let mut out = Vec::with_capacity(n / 3 + 1);
    let mut i = 0;
    while i < n {
        let len = (if pattern == Pattern::Cl100kBase {
            match_contraction(&chars, i, true)
        } else {
            None
        })
        .or_else(|| match_word(&chars, i, pattern))
        .or_else(|| match_number(&chars, i))
        .or_else(|| match_symbol(&chars, i, pattern))
        .unwrap_or_else(|| match_whitespace(&chars, i));

        debug_assert!(len > 0, "pretokenizer must always advance");
        out.push(&text[byte_at(i)..byte_at(i + len)]);
        i += len;
    }
    out
}

/// `[^\r\n\p{L}\p{N}]` — anything that could open a "prefix" slot: not a
/// newline, not a letter, not a digit. (Whitespace and punctuation both
/// qualify.)
fn boundary_char(c: char) -> bool {
    c != '\r' && c != '\n' && !c.is_alphabetic() && !c.is_numeric()
}

fn is_symbolish(c: char) -> bool {
    !c.is_whitespace() && !c.is_alphabetic() && !c.is_numeric()
}

/// Best-effort combining-mark detector (`\p{M}`), covering the common
/// combining-diacritic Unicode blocks. See module docs for the accuracy
/// caveat on scripts outside these blocks.
fn is_mark(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F |
        0x1AB0..=0x1AFF |
        0x1DC0..=0x1DFF |
        0x20D0..=0x20FF |
        0xFE20..=0xFE2F
    )
}

fn is_wordish(c: char, pattern: Pattern) -> bool {
    c.is_alphabetic() || (pattern == Pattern::O200kBase && is_mark(c))
}

/// A contraction suffix starting at `i`: `'s`, `'t`, `'re`, `'ve`, `'m`,
/// `'ll`, or `'d`. Returns the number of chars consumed (including the
/// apostrophe).
///
/// `case_insensitive` names the behavior of the regex group each call
/// site implements. As shipped, both are `true`: cl100k_base's group is
/// `'(?i:[sdmt]|ll|ve|re)` and o200k_base's is
/// `(?i:'s|'t|'re|'ve|'m|'ll|'d)` — verified against real tiktoken's
/// pattern strings and split output, and pinned by the golden fixtures
/// (`x'Sy`, `DON'T STOP`, ...). The flag exists so the call sites document
/// which group they mirror; passing `false` would make that splitter
/// *finer* than the real one, which is the unsafe direction (see the
/// module docs).
fn match_contraction(chars: &[(usize, char)], i: usize, case_insensitive: bool) -> Option<usize> {
    if chars.get(i).map(|c| c.1) != Some('\'') {
        return None;
    }
    let at = |k: usize| {
        chars.get(i + k).map(|c| {
            if case_insensitive {
                c.1.to_ascii_lowercase()
            } else {
                c.1
            }
        })
    };
    match at(1)? {
        's' | 't' | 'm' | 'd' => Some(2),
        'r' if at(2) == Some('e') => Some(3),
        'v' if at(2) == Some('e') => Some(3),
        'l' if at(2) == Some('l') => Some(3),
        _ => None,
    }
}

/// cl100k: `[^\r\n\p{L}\p{N}]?\p{L}+` — matched exactly.
/// o200k: real tiktoken uses two case-homogeneous alternatives, `UP*LO+` /
/// `UP+LO*`, so it cuts at every case transition. This deliberately does
/// not: it consumes the whole run of wordish chars regardless of case, a
/// coarser split that's still encoding-equivalent (see the module docs).
/// Both patterns get an optional leading boundary char and, for o200k
/// only, an optional trailing contraction suffix (matched case-
/// insensitively, unlike cl100k's standalone, case-sensitive check in
/// `split`).
fn match_word(chars: &[(usize, char)], i: usize, pattern: Pattern) -> Option<usize> {
    let n = chars.len();
    let (start, prefix) = if is_wordish(chars[i].1, pattern) {
        (i, 0)
    } else if boundary_char(chars[i].1) && i + 1 < n && is_wordish(chars[i + 1].1, pattern) {
        (i + 1, 1)
    } else {
        return None;
    };

    let mut j = start;
    while j < n && is_wordish(chars[j].1, pattern) {
        j += 1;
    }
    let mut len = prefix + (j - start);

    if pattern == Pattern::O200kBase {
        if let Some(clen) = match_contraction(chars, i + len, true) {
            len += clen;
        }
    }
    Some(len)
}

/// `\p{N}{1,3}` — a run of 1 to 3 digits.
fn match_number(chars: &[(usize, char)], i: usize) -> Option<usize> {
    if !chars[i].1.is_numeric() {
        return None;
    }
    let n = chars.len();
    let mut len = 1;
    while len < 3 && i + len < n && chars[i + len].1.is_numeric() {
        len += 1;
    }
    Some(len)
}

/// cl100k: ` ?[^\s\p{L}\p{N}]+[\r\n]*`
/// o200k:  ` ?[^\s\p{L}\p{N}]+[\r\n/]*` (trailing class also absorbs `/`)
fn match_symbol(chars: &[(usize, char)], i: usize, pattern: Pattern) -> Option<usize> {
    let n = chars.len();
    let (start, prefix) = if chars[i].1 == ' ' && i + 1 < n && is_symbolish(chars[i + 1].1) {
        (i + 1, 1)
    } else if is_symbolish(chars[i].1) {
        (i, 0)
    } else {
        return None;
    };

    let mut j = start;
    while j < n && is_symbolish(chars[j].1) {
        j += 1;
    }
    let mut len = prefix + (j - start);

    while i + len < n {
        let c = chars[i + len].1;
        let trailing = c == '\r' || c == '\n' || (pattern == Pattern::O200kBase && c == '/');
        if !trailing {
            break;
        }
        len += 1;
    }
    Some(len)
}

/// Handles all three whitespace alternatives at once
/// (`\s*[\r\n]+`, `\s+(?!\S)`, `\s+`): find the whitespace run starting at
/// `i`; if it contains a newline, consume through the last newline in the
/// run; otherwise consume the whole run if it reaches end-of-input, else
/// consume all but its last character (which becomes the leading "prefix"
/// slot for whatever word/symbol token follows).
fn match_whitespace(chars: &[(usize, char)], i: usize) -> usize {
    let n = chars.len();
    let mut j = i;
    while j < n && chars[j].1.is_whitespace() {
        j += 1;
    }

    if let Some(p) = (i..j).rev().find(|&k| matches!(chars[k].1, '\r' | '\n')) {
        return p - i + 1;
    }
    if j == n {
        return j - i;
    }
    let run_len = j - i;
    if run_len >= 2 {
        run_len - 1
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranks_from(pairs: &[(&[u8], Rank)]) -> Ranks {
        let mut ranks = Ranks::new();
        for b in 0u32..256 {
            ranks.insert(vec![b as u8], b);
        }
        for (bytes, rank) in pairs {
            ranks.insert(bytes.to_vec(), *rank);
        }
        ranks
    }

    #[test]
    fn merge_prefers_lowest_rank_pair() {
        let ranks = ranks_from(&[(b"bc", 50), (b"ab", 100), (b"abc", 200)]);
        // "b"+"c" (rank 50) merges before "a"+"b" (rank 100), then
        // "a"+"bc" (rank 200) merges last.
        assert_eq!(byte_pair_merge(b"abc", &ranks), vec![200]);
    }

    #[test]
    fn merge_stops_when_no_pair_mergeable() {
        let ranks = ranks_from(&[]);
        assert_eq!(
            byte_pair_merge(b"xy", &ranks),
            vec![b'x' as Rank, b'y' as Rank]
        );
    }

    #[test]
    fn merge_single_byte_and_empty() {
        let ranks = ranks_from(&[]);
        assert_eq!(byte_pair_merge(b"", &ranks), Vec::<Rank>::new());
        assert_eq!(byte_pair_merge(b"a", &ranks), vec![b'a' as Rank]);
    }

    #[test]
    fn merge_only_applies_learned_pair() {
        let ranks = ranks_from(&[(b"ab", 50)]);
        // "ab" merges; the trailing "c" has no merge partner and stays lone.
        assert_eq!(byte_pair_merge(b"abc", &ranks), vec![50, b'c' as Rank]);
    }

    #[test]
    fn split_cl100k_word_and_space() {
        assert_eq!(
            split("hello world", Pattern::Cl100kBase),
            vec!["hello", " world"]
        );
    }

    #[test]
    fn split_cl100k_contraction_is_its_own_token() {
        assert_eq!(split("don't", Pattern::Cl100kBase), vec!["don", "'t"]);
    }

    #[test]
    fn split_cl100k_contraction_is_case_insensitive() {
        // cl100k_base's contraction group is `'(?i:[sdmt]|ll|ve|re)` —
        // case-insensitive — so `'S` is a contraction there too (verified
        // against real tiktoken's split: x'Sy -> x | 'S | y).
        assert_eq!(split("x'Sy", Pattern::Cl100kBase), vec!["x", "'S", "y"]);
        assert_eq!(
            split("DON'T STOP", Pattern::Cl100kBase),
            vec!["DON", "'T", " STOP"]
        );
    }

    #[test]
    fn split_o200k_contraction_is_case_insensitive() {
        // o200k_base folds case too, but attaches the contraction to the
        // word (real tiktoken: x'Sy -> x'S | y).
        assert_eq!(split("x'Sy", Pattern::O200kBase), vec!["x'S", "y"]);
        assert_eq!(
            split("DON'T STOP", Pattern::O200kBase),
            vec!["DON'T", " STOP"]
        );
    }

    #[test]
    fn split_cl100k_digits_group_in_threes() {
        assert_eq!(split("123456", Pattern::Cl100kBase), vec!["123", "456"]);
        assert_eq!(split("12", Pattern::Cl100kBase), vec!["12"]);
    }

    #[test]
    fn split_cl100k_punctuation() {
        assert_eq!(
            split("hello, world!", Pattern::Cl100kBase),
            vec!["hello", ",", " world", "!"]
        );
    }

    #[test]
    fn split_cl100k_multiple_spaces_attach_last_to_word() {
        assert_eq!(split("a  b", Pattern::Cl100kBase), vec!["a", " ", " b"]);
    }

    #[test]
    fn split_cl100k_trailing_whitespace_at_eof_is_one_token() {
        assert_eq!(split("a   ", Pattern::Cl100kBase), vec!["a", "   "]);
    }

    #[test]
    fn split_cl100k_blank_line_groups_newlines() {
        assert_eq!(split("a\n\nb", Pattern::Cl100kBase), vec!["a", "\n\n", "b"]);
    }

    #[test]
    fn split_cl100k_newline_then_trailing_spaces() {
        // The run "\n  " contains a newline not at its end: consume through
        // the newline only, leaving "  " for the next token(s).
        assert_eq!(split("\n  x", Pattern::Cl100kBase), vec!["\n", " ", " x"]);
    }

    #[test]
    fn split_roundtrips_to_original_text() {
        let text = "Hello, World! 123 don't\n\n  foo   ";
        for pattern in [Pattern::Cl100kBase, Pattern::O200kBase] {
            let pieces = split(text, pattern);
            assert_eq!(pieces.concat(), text);
        }
    }

    #[test]
    fn split_o200k_all_caps_word_is_one_token() {
        assert_eq!(
            split("HELLO world", Pattern::O200kBase),
            vec!["HELLO", " world"]
        );
    }

    #[test]
    fn split_o200k_mixed_case_run_is_one_token() {
        // UP* run "ABC" followed by LO+ run "def" merges into one token.
        assert_eq!(split("ABCdef", Pattern::O200kBase), vec!["ABCdef"]);
    }

    #[test]
    fn split_o200k_contraction_attaches_to_word() {
        assert_eq!(split("don't", Pattern::O200kBase), vec!["don't"]);
    }

    #[test]
    fn split_o200k_digits_and_symbols() {
        assert_eq!(split("1234", Pattern::O200kBase), vec!["123", "4"]);
        // The word alternative's optional prefix slot accepts any
        // non-letter/digit/newline char, including '/', and is tried
        // before the symbol-run alternative, so the slash attaches to the
        // following word rather than standing alone.
        assert_eq!(split("a/b", Pattern::O200kBase), vec!["a", "/b"]);
        // With nothing wordish to attach to, '/' falls through to the
        // symbol-run alternative on its own; a leading space never attaches
        // to a following digit run (only the letter/symbol alternatives
        // have an optional prefix slot).
        assert_eq!(split("a/ 1", Pattern::O200kBase), vec!["a", "/", " ", "1"]);
    }

    #[test]
    fn encode_ordinary_matches_manual_split_and_merge() {
        let ranks = ranks_from(&[(b"ab", 300)]);
        let tokens = encode_ordinary("ab cd", &ranks, Pattern::Cl100kBase);
        // "ab" merges to rank 300; " cd" has no learned merges so stays as
        // 3 individual bytes: ' ', 'c', 'd'.
        assert_eq!(tokens, vec![300, b' ' as Rank, b'c' as Rank, b'd' as Rank]);
    }
}
