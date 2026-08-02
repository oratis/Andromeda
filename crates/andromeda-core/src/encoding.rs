//! Deterministic encodings shared by every Andromeda signature scheme.
//!
//! Two signature schemes now exist in this workspace — detached HCM manifest
//! signatures (`andromeda-hardware`) and detached capability signatures
//! ([`crate::capability_signing`]) — and both must agree, byte for byte, on
//! how a typed value becomes a message and how raw bytes become text. A second
//! copy of either encoder is a place where the two can silently drift, so both
//! live here and both schemes call these functions rather than their own.
//!
//! Neither encoder is cryptographic on its own; they only remove ambiguity.

/// Lowercase hex, the workspace's single encoding for opaque byte strings.
///
/// Keys, signatures, and digests are hex everywhere in this repository
/// (`ArtifactPin.sha256`, `ManifestSignature.sig`, verifying keys, the `taskd`
/// API token). Deliberately not base64: one encoding for one job means a
/// reader never has to ask which is in play, and it keeps a base64 crate out
/// of the dependency graph.
pub mod hex {
    use std::fmt::Write as _;

    /// A hex string that could not be decoded.
    ///
    /// The message is the bare reason ("odd number of hex digits"), with no
    /// "invalid hex" prefix: callers wrap this in their own error, and a prefix
    /// here would show up doubled in the final message.
    #[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
    #[error("{reason}")]
    pub struct HexError {
        reason: String,
    }

    impl HexError {
        fn new(reason: impl Into<String>) -> Self {
            Self {
                reason: reason.into(),
            }
        }
    }

    /// Lowercase-hex encodes `bytes`.
    #[must_use]
    pub fn encode(bytes: &[u8]) -> String {
        let mut hex = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            // Writing to a String is infallible.
            let _ = write!(hex, "{byte:02x}");
        }
        hex
    }

    /// Decodes an even-length hex string, ignoring surrounding whitespace.
    ///
    /// Accepts either case on input; [`encode`] only ever emits lowercase.
    ///
    /// # Errors
    /// Returns [`HexError`] when the input has an odd number of digits or
    /// contains a character that is not a hex digit.
    pub fn decode(input: &str) -> Result<Vec<u8>, HexError> {
        let input = input.trim();
        if input.len() % 2 != 0 {
            return Err(HexError::new("odd number of hex digits"));
        }
        let bytes = input.as_bytes();
        let mut out = Vec::with_capacity(bytes.len() / 2);
        let mut index = 0;
        while index < bytes.len() {
            let high = digit(bytes[index])?;
            let low = digit(bytes[index + 1])?;
            out.push((high << 4) | low);
            index += 2;
        }
        Ok(out)
    }

    fn digit(character: u8) -> Result<u8, HexError> {
        match character {
            b'0'..=b'9' => Ok(character - b'0'),
            b'a'..=b'f' => Ok(character - b'a' + 10),
            b'A'..=b'F' => Ok(character - b'A' + 10),
            other => Err(HexError::new(format!(
                "invalid hex digit '{}'",
                char::from(other)
            ))),
        }
    }
}

/// Canonical JSON: the byte string a signature actually covers.
///
/// Rules, and the reason each exists:
///
/// - **Object keys sorted** by Unicode scalar value, so a re-serialization that
///   happens to emit fields in another order still verifies.
/// - **Compact output** (no insignificant whitespace), so pretty-printing a
///   file on disk does not invalidate its signature.
/// - **Arrays keep their order**, because array order is semantic.
/// - **Scalars use `serde_json`'s own encoding**, which already escapes strings
///   deterministically. Callers must not put floats in a signed structure; no
///   Andromeda signed type has one.
///
/// Signers are expected to serialize the *typed* model rather than reuse the
/// bytes they parsed, so "field omitted" and "field written as `null`" cannot
/// produce two different messages for one logical value.
pub mod canonical_json {
    use std::fmt::Write as _;

    /// Serializes `value` as canonical JSON into a new `String`.
    #[must_use]
    pub fn to_string(value: &serde_json::Value) -> String {
        let mut out = String::new();
        write(value, &mut out);
        out
    }

    /// Recursively writes `value` as canonical JSON with sorted object keys.
    pub fn write(value: &serde_json::Value, out: &mut String) {
        match value {
            serde_json::Value::Object(map) => {
                out.push('{');
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort_unstable();
                for (index, key) in keys.into_iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    write_string(key, out);
                    out.push(':');
                    write(&map[key], out);
                }
                out.push('}');
            }
            serde_json::Value::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    write(item, out);
                }
                out.push(']');
            }
            // Scalars: `Value`'s Display is compact JSON with correct escaping.
            scalar => {
                let _ = write!(out, "{scalar}");
            }
        }
    }

    /// Writes `text` as a JSON string literal (quoted and escaped).
    fn write_string(text: &str, out: &mut String) {
        let _ = write!(out, "{}", serde_json::Value::String(text.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use super::{canonical_json, hex};

    #[test]
    fn hex_round_trips_and_rejects_bad_input() {
        let bytes = [0x00u8, 0x0f, 0xa5, 0xff];
        assert_eq!(hex::encode(&bytes), "000fa5ff");
        assert_eq!(hex::decode("000fA5ff").unwrap(), bytes);
        assert_eq!(hex::decode("  000fa5ff \n").unwrap(), bytes);
        assert!(hex::decode("abc").is_err()); // odd length
        assert!(hex::decode("zz").is_err()); // non-hex digit
        assert_eq!(hex::encode(&[]), "");
        assert_eq!(hex::decode("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn object_keys_are_sorted_regardless_of_input_order() {
        let a: serde_json::Value = serde_json::from_str(r#"{"b":1,"a":2}"#).unwrap();
        let b: serde_json::Value = serde_json::from_str(r#"{"a":2,"b":1}"#).unwrap();
        assert_eq!(canonical_json::to_string(&a), r#"{"a":2,"b":1}"#);
        assert_eq!(canonical_json::to_string(&a), canonical_json::to_string(&b));
    }

    #[test]
    fn array_order_is_preserved() {
        let value: serde_json::Value = serde_json::from_str("[3,1,2]").unwrap();
        assert_eq!(canonical_json::to_string(&value), "[3,1,2]");
    }

    #[test]
    fn whitespace_and_nesting_do_not_change_the_bytes() {
        let pretty: serde_json::Value = serde_json::from_str(
            "{\n  \"outer\": {\n    \"z\": [1, {\"y\": null, \"x\": true}]\n  }\n}",
        )
        .unwrap();
        assert_eq!(
            canonical_json::to_string(&pretty),
            r#"{"outer":{"z":[1,{"x":true,"y":null}]}}"#
        );
    }

    #[test]
    fn strings_are_escaped_not_copied_raw() {
        let value = serde_json::json!({ "quote\"key": "line\nbreak" });
        assert_eq!(
            canonical_json::to_string(&value),
            r#"{"quote\"key":"line\nbreak"}"#
        );
    }
}
