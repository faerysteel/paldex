//! Reads Palworld's localized text `DataTable`s out of a cooked `.uexp`
//! **without** a `.usmap`.
//!
//! Palworld's `Game.locres` files are empty — all user-facing text lives in
//! per-language `DataTable`s under `Pal/Content/L10N/<lang>/…/Text/`. Those
//! packages set `PKG_UnversionedProperties`, so their row *structs* can't be
//! deserialized without a schema. But the payload we want is an `FText`, and
//! `FText` with `Base` history serializes as three plain `FString`s:
//!
//! ```text
//! u32 Flags | u8 HistoryType(0 = Base) | FString Namespace | FString Key | FString SourceString
//! ```
//!
//! `FString` is length-prefixed and schema-free, so those triples are directly
//! readable. Verified against the real pak — `DT_PalNameText_Common.uexp`
//! yields `("DT_PalNameText_Common", "PAL_NAME_AmaterasuWolf_TextData",
//! "Kitsun")`, which is exactly the `CharacterID → display name` mapping the
//! app needs.
//!
//! Rather than guessing at row layout, we anchor on the namespace — which
//! always equals the table's own name — and read the two `FString`s that
//! follow. The `_TextData` key suffix then acts as a self-check that we landed
//! on a real record instead of a coincidental byte match.

use crate::uasset::{Reader, UassetError};

/// Suffix Palworld appends to every text-table row key.
const KEY_SUFFIX: &str = "_TextData";

/// One resolved row of a text `DataTable`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEntry {
    /// Row key with the `_TextData` suffix stripped, e.g.
    /// `PAL_NAME_AmaterasuWolf`.
    pub key: String,
    /// The localized string, or `None` when the row is an untranslated
    /// placeholder (see [`is_placeholder`]).
    pub text: Option<String>,
}

/// Whether a source string is one of the game's untranslated placeholders
/// rather than real localized text.
///
/// Shipping tables contain rows that were never translated; the real pak has
/// `PAL_NAME_BeardedDragon → "en_text"` and many `→ "en Text"`. Returning
/// these as display names would surface obvious junk in the UI, so callers
/// fall back to the raw id instead.
#[must_use]
pub fn is_placeholder(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return true;
    }
    // e.g. "en_text", "en Text", "ja text" — a 2-letter language tag followed
    // by the literal word "text".
    let normalized = t.to_ascii_lowercase().replace([' ', '_', '-'], "");
    if let Some(rest) = normalized.strip_suffix("text") {
        if rest.len() == 2 && rest.chars().all(|c| c.is_ascii_alphabetic()) {
            return true;
        }
    }
    false
}

/// Strip Unreal rich-text markup, keeping the readable content.
///
/// Some rows embed markup such as
/// `<itemName id=|AssaultRifle_Default1|/>`, which the game resolves at
/// display time. There is nothing to resolve it against here, so the tags are
/// removed; a row that is *entirely* markup ends up empty and is reported as a
/// placeholder.
#[must_use]
pub fn strip_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    for ch in text.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_owned()
}

/// Extract every `FText` row from a cooked text `DataTable`'s `.uexp`.
///
/// `namespace` is the table's own name (e.g. `DT_PalNameText_Common`), which
/// is what the game writes into each record's namespace field.
pub fn parse(uexp: &[u8], namespace: &str) -> Result<Vec<TextEntry>, UassetError> {
    let anchor = encode_ascii_fstring(namespace);
    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let mut search_from = 0usize;
    while let Some(rel) = find(&uexp[search_from..], &anchor) {
        let start = search_from + rel;
        search_from = start + anchor.len();

        // An FText's namespace is preceded by u32 Flags + u8 HistoryType.
        // Requiring Base history (0) rejects coincidental byte matches.
        if start == 0 || uexp[start - 1] != 0 {
            continue;
        }

        let mut r = Reader::at(uexp, search_from);
        let (Ok(key), Ok(source)) = (r.fstring(), r.fstring()) else {
            continue;
        };
        let Some(key) = key.strip_suffix(KEY_SUFFIX) else {
            continue;
        };
        if key.is_empty() || !seen.insert(key.to_owned()) {
            continue;
        }

        let cleaned = strip_markup(&source);
        let text = (!is_placeholder(&cleaned)).then_some(cleaned);
        entries.push(TextEntry { key: key.to_owned(), text });
        search_from = r.pos();
    }

    Ok(entries)
}

/// Encode a string the way an Unreal ASCII `FString` appears on disk, so it can
/// be located by exact byte match.
fn encode_ascii_fstring(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() + 5);
    out.extend_from_slice(&(s.len() as i32 + 1).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
    out.push(0);
    out
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ftext_record(namespace: &str, key: &str, source: &str) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&0u32.to_le_bytes()); // Flags
        out.push(0); // HistoryType::Base
        out.extend_from_slice(&encode_ascii_fstring(namespace));
        out.extend_from_slice(&encode_ascii_fstring(key));
        out.extend_from_slice(&encode_ascii_fstring(source));
        out
    }

    #[test]
    fn extracts_rows_and_strips_the_key_suffix() {
        let mut buf = vec![0xAA; 16]; // leading junk, as in a real uexp
        buf.extend(ftext_record("DT_PalNameText_Common", "PAL_NAME_AmaterasuWolf_TextData", "Kitsun"));
        buf.extend(ftext_record("DT_PalNameText_Common", "PAL_NAME_Anubis_TextData", "Anubis"));

        let rows = parse(&buf, "DT_PalNameText_Common").expect("parse");
        assert_eq!(
            rows,
            vec![
                TextEntry { key: "PAL_NAME_AmaterasuWolf".into(), text: Some("Kitsun".into()) },
                TextEntry { key: "PAL_NAME_Anubis".into(), text: Some("Anubis".into()) },
            ]
        );
    }

    /// Untranslated rows must come back as `None`, not as the literal
    /// placeholder — otherwise the UI shows "en_text" as a Pal's name.
    #[test]
    fn untranslated_rows_report_no_text() {
        let buf = ftext_record("DT_T", "PAL_NAME_BeardedDragon_TextData", "en_text");
        let rows = parse(&buf, "DT_T").expect("parse");
        assert_eq!(rows[0].text, None);
    }

    #[test]
    fn recognizes_placeholder_variants() {
        assert!(is_placeholder("en_text"));
        assert!(is_placeholder("en Text"));
        assert!(is_placeholder("  "));
        assert!(!is_placeholder("Kitsun"));
        assert!(!is_placeholder("Text"));
        assert!(!is_placeholder("Mau Cryst"));
    }

    #[test]
    fn strips_rich_text_markup() {
        assert_eq!(strip_markup("<itemName id=|AssaultRifle_Default1|/>"), "");
        assert_eq!(strip_markup("Ring of <b>Water</b> Resistance"), "Ring of Water Resistance");
        assert_eq!(strip_markup("Melpaca"), "Melpaca");
    }

    /// A byte sequence that merely looks like the namespace, without the
    /// preceding Base-history marker, must not produce a bogus row.
    #[test]
    fn ignores_matches_without_base_history_marker() {
        let mut buf = vec![0xFFu8]; // non-zero byte where HistoryType would be
        buf.extend(encode_ascii_fstring("DT_T"));
        buf.extend(encode_ascii_fstring("PAL_NAME_Fake_TextData"));
        buf.extend(encode_ascii_fstring("Fake"));
        assert!(parse(&buf, "DT_T").expect("parse").is_empty());
    }

    #[test]
    fn ignores_keys_without_the_expected_suffix() {
        let buf = ftext_record("DT_T", "PAL_NAME_Something", "Value");
        assert!(parse(&buf, "DT_T").expect("parse").is_empty());
    }

    #[test]
    fn truncation_never_panics() {
        let mut full = vec![0u8; 8];
        full.extend(ftext_record("DT_T", "PAL_NAME_Anubis_TextData", "Anubis"));
        for n in 0..full.len() {
            let _ = parse(&full[..n], "DT_T");
        }
    }
}
