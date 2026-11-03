//! The evidence an award carries: the generated `BadgeEvidence`, in its wire names, written once
//! at the award and never re-derived.

use crate::index::IndexError;
use crate::protocol::BadgeEvidence;

use super::BadgeError;

/// `evidence` as `badge_award.evidence` stores it: the wire names, with every absent field left
/// out rather than written as `null`.
///
/// # Errors
/// [`IndexError::Corrupt`] when the document does not serialise, which a generated struct of
/// strings and integers never does.
pub fn encode(evidence: &BadgeEvidence) -> Result<String, IndexError> {
    let mut value = serde_json::to_value(evidence).map_err(|e| IndexError::Corrupt {
        detail: format!("badge evidence did not encode: {e}"),
    })?;
    if let serde_json::Value::Object(fields) = &mut value {
        fields.retain(|_, field| !field.is_null());
    }
    Ok(value.to_string())
}

/// A stored evidence document read back. A key the generated type does not declare is refused
/// by the type itself, so a field no schema names cannot ride along.
///
/// # Errors
/// [`BadgeError::Evidence`] when `raw` is not a `BadgeEvidence` — a key it does not declare
/// among others.
pub fn decode(raw: &str) -> Result<BadgeEvidence, BadgeError> {
    serde_json::from_str(raw).map_err(|e| BadgeError::Evidence(e.to_string()))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    #[test]
    fn a_key_the_type_does_not_declare_is_refused() {
        let refused = decode(r#"{"projectName":"x","lines":3}"#);
        eprintln!("{refused:?}");
        assert!(matches!(refused, Err(BadgeError::Evidence(_))));
    }

    #[test]
    fn evidence_round_trips_without_its_absent_fields() {
        let evidence = decode(r#"{"projectName":"x","gapDays":200}"#).unwrap();
        let encoded = encode(&evidence).unwrap();
        assert_eq!(encoded, r#"{"gapDays":200,"projectName":"x"}"#);
        assert_eq!(decode(&encoded).unwrap(), evidence);
    }
}
