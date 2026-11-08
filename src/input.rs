//! Decoding of raw input bytes (a `--file` or stdin) into text.
//!
//! The parser works on `&str`, so the boundary has to decide what the bytes
//! are.  Windows tools disagree on this: Notepad writes UTF-8 with or without
//! a BOM, PowerShell 5.1's `>` redirection writes UTF-16 LE with a BOM, and
//! older editors write a legacy code page.  Guessing wrongly is worse than
//! refusing: a leftover BOM becomes part of the first directory's name, and
//! a code-page file decoded lossily creates files with `�` in their names.
//!
//! # Rules
//!
//! 1. A UTF-8 BOM is removed.
//! 2. A UTF-16 BOM (LE `FF FE`, BE `FE FF`) selects UTF-16, which is decoded
//!    strictly.
//! 3. A UTF-32 BOM is recognised and rejected with a clear message.
//! 4. Anything else must be valid UTF-8.  Otherwise decoding fails and the
//!    error says how to re-save the file; nothing is replaced silently.

use crate::error::AppError;

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
const UTF16_LE_BOM: &[u8] = &[0xFF, 0xFE];
const UTF16_BE_BOM: &[u8] = &[0xFE, 0xFF];
const UTF32_LE_BOM: &[u8] = &[0xFF, 0xFE, 0x00, 0x00];
const UTF32_BE_BOM: &[u8] = &[0x00, 0x00, 0xFE, 0xFF];

/// Decode `bytes` according to the rules in the module docs.
pub(crate) fn decode(bytes: &[u8]) -> Result<String, AppError> {
    // UTF-32 must be tested before UTF-16: its little-endian BOM starts with
    // the UTF-16 one.  A UTF-16 file that really began with U+0000 would be
    // misread, but a tree never does.
    if bytes.starts_with(UTF32_LE_BOM) || bytes.starts_with(UTF32_BE_BOM) {
        return Err(AppError::invalid_encoding(
            "the input is UTF-32, which is not supported",
        ));
    }
    if let Some(rest) = bytes.strip_prefix(UTF8_BOM) {
        return decode_utf8(rest);
    }
    if let Some(rest) = bytes.strip_prefix(UTF16_LE_BOM) {
        return decode_utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(UTF16_BE_BOM) {
        return decode_utf16(rest, u16::from_be_bytes);
    }
    decode_utf8(bytes)
}

fn decode_utf8(bytes: &[u8]) -> Result<String, AppError> {
    // NUL is valid UTF-8, so UTF-16 without a BOM made of ASCII letters
    // (`m\0y\0…`) would pass `from_utf8` and yield names containing NUL.
    // A text tree never contains NUL.
    if bytes.contains(&0) {
        let reason = if looks_like_utf16_without_bom(bytes) {
            "the input looks like UTF-16 without a byte-order mark"
        } else {
            "the input contains NUL bytes, so it is not a plain text file"
        };
        return Err(AppError::invalid_encoding(reason));
    }

    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|err| {
            AppError::invalid_encoding(format!(
                "the input is not valid UTF-8 (first bad byte at offset {}); it may use a legacy \
             code page such as Windows-1251 or Windows-1252",
                err.valid_up_to()
            ))
        })
}

fn decode_utf16(bytes: &[u8], to_unit: fn([u8; 2]) -> u16) -> Result<String, AppError> {
    // UTF-16 is a sequence of two-byte units, so a leftover byte means the
    // input was cut off or is not UTF-16 at all.
    let (pairs, remainder) = bytes.as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(AppError::invalid_encoding(
            "the UTF-16 input has an odd number of bytes, so it is truncated or damaged",
        ));
    }
    let units = pairs.iter().map(|pair| to_unit(*pair));
    let text = char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .map_err(|_| {
            AppError::invalid_encoding("the UTF-16 input contains an unpaired surrogate")
        })?;

    // The UTF-8 path rejects NUL on the raw bytes; here U+0000 only exists
    // after decoding. A name with NUL would fail when the builder reaches it,
    // after earlier entries had already been created.
    if text.contains('\0') {
        return Err(AppError::invalid_encoding(
            "the input contains NUL characters, so it is not a plain text file",
        ));
    }
    Ok(text)
}

/// UTF-16 text made of mostly ASCII has a zero byte next to every letter.
fn looks_like_utf16_without_bom(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes.iter().filter(|&&b| b == 0).count() * 4 >= bytes.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16_le(text: &str) -> Vec<u8> {
        let mut out = UTF16_LE_BOM.to_vec();
        out.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        out
    }

    fn utf16_be(text: &str) -> Vec<u8> {
        let mut out = UTF16_BE_BOM.to_vec();
        out.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
        out
    }

    #[test]
    fn plain_utf8_is_unchanged() {
        assert_eq!(
            decode("my-app/\n└── a.rs".as_bytes()).unwrap(),
            "my-app/\n└── a.rs"
        );
    }

    #[test]
    fn utf8_bom_is_removed() {
        let mut bytes = UTF8_BOM.to_vec();
        bytes.extend("my-app/".as_bytes());
        assert_eq!(decode(&bytes).unwrap(), "my-app/");
    }

    #[test]
    fn utf16_le_with_bom_is_decoded() {
        assert_eq!(
            decode(&utf16_le("my-app/\n└── 日本.rs")).unwrap(),
            "my-app/\n└── 日本.rs"
        );
    }

    #[test]
    fn utf16_be_with_bom_is_decoded() {
        assert_eq!(
            decode(&utf16_be("my-app/\n└── a.rs")).unwrap(),
            "my-app/\n└── a.rs"
        );
    }

    #[test]
    fn utf16_surrogate_pairs_are_decoded() {
        assert_eq!(decode(&utf16_le("📁 docs/")).unwrap(), "📁 docs/");
    }

    #[test]
    fn bom_only_input_is_empty_text() {
        assert_eq!(decode(UTF8_BOM).unwrap(), "");
        assert_eq!(decode(UTF16_LE_BOM).unwrap(), "");
        assert_eq!(decode(b"").unwrap(), "");
    }

    #[test]
    fn invalid_utf8_is_an_error_not_replacement_characters() {
        // "привет" in Windows-1251.
        let err = decode(&[0xEF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2])
            .unwrap_err()
            .to_string();
        assert!(err.contains("not valid UTF-8"), "{err}");
        assert!(err.contains("Windows-1251"), "{err}");
        assert!(err.contains("UTF-8"), "{err}");
    }

    #[test]
    fn nul_character_in_utf16_is_rejected_like_in_utf8() {
        // A BOM-marked file decodes cleanly, so the check on the raw UTF-8
        // bytes never sees this NUL.
        for encode in [utf16_le, utf16_be] {
            let err = decode(&encode("ok/\nbad\0name")).unwrap_err().to_string();
            assert!(err.contains("NUL"), "{err}");
        }
    }

    #[test]
    fn nul_bytes_in_otherwise_valid_utf8_are_rejected() {
        let err = decode(b"a\0b").unwrap_err().to_string();
        assert!(err.contains("NUL"), "{err}");
    }

    #[test]
    fn utf16_without_bom_is_recognised_in_the_message() {
        let bytes: Vec<u8> = "my-app/"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let err = decode(&bytes).unwrap_err().to_string();
        assert!(err.contains("UTF-16 without a byte-order mark"), "{err}");
    }

    #[test]
    fn truncated_utf16_is_an_error() {
        let mut bytes = utf16_le("ab");
        bytes.pop();
        assert!(decode(&bytes)
            .unwrap_err()
            .to_string()
            .contains("odd number of bytes"));
    }

    #[test]
    fn unpaired_surrogate_is_an_error() {
        let mut bytes = UTF16_LE_BOM.to_vec();
        bytes.extend(0xD800u16.to_le_bytes());
        assert!(decode(&bytes)
            .unwrap_err()
            .to_string()
            .contains("unpaired surrogate"));
    }

    #[test]
    fn utf32_is_rejected_by_name() {
        for bom in [UTF32_LE_BOM, UTF32_BE_BOM] {
            let err = decode(bom).unwrap_err().to_string();
            assert!(err.contains("UTF-32"), "{err}");
        }
    }
}
