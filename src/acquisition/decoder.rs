//! Decode raw 16-bit words into typed values, per register configuration.

use crate::acquisition::AcquisitionError;
use crate::config::{RegisterConfig, ValueType, WordOrder};
use crate::domain::{RawSample, SensorId};

/// Decode `words` (from a single register read) into the configured numeric value.
pub fn decode_words(words: &[u16], value_type: ValueType, word_order: WordOrder) -> f64 {
    match value_type {
        ValueType::U16 => f64::from(words[0]),
        ValueType::I16 => f64::from(words[0] as i16),
        ValueType::U32 => f64::from(combine(words, word_order)),
        ValueType::I32 => f64::from(combine(words, word_order) as i32),
        ValueType::F32 => f64::from(f32::from_bits(combine(words, word_order))),
    }
}

/// Combine two registers into a 32-bit word.
/// `Big`: first register is the high word. `Little`: first register is the low word.
fn combine(words: &[u16], order: WordOrder) -> u32 {
    match order {
        WordOrder::Big => (u32::from(words[0]) << 16) | u32::from(words[1]),
        WordOrder::Little => (u32::from(words[1]) << 16) | u32::from(words[0]),
    }
}

/// Build a [`RawSample`] from the words returned for one register group.
/// Validates the response length (a broken/evil slave must not panic us).
pub fn raw_sample(
    device: &str,
    reg: &RegisterConfig,
    words: &[u16],
) -> Result<RawSample, AcquisitionError> {
    let need = usize::from(reg.value_type.register_count());
    if words.len() < need {
        return Err(AcquisitionError::Register {
            device: device.to_string(),
            name: reg.name.clone(),
            message: format!(
                "slave returned {} register(s), {} required by value_type {:?}",
                words.len(),
                need,
                reg.value_type
            ),
        });
    }
    Ok(RawSample {
        sensor_id: SensorId(reg.sensor_id.clone()),
        name: reg.name.clone(),
        raw_value: decode_words(words, reg.value_type, reg.word_order),
        unit: reg.unit.clone(),
        timestamp: RawSample::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_u16() {
        assert_eq!(decode_words(&[0x1234], ValueType::U16, WordOrder::Big), 0x1234 as f64);
    }

    #[test]
    fn decode_i16_negative() {
        assert_eq!(decode_words(&[0xFFFF], ValueType::I16, WordOrder::Big), -1.0);
    }

    #[test]
    fn decode_u32_big() {
        assert_eq!(
            decode_words(&[0xDEAD, 0xBEEF], ValueType::U32, WordOrder::Big),
            0xDEAD_BEEFu32 as f64
        );
    }

    #[test]
    fn decode_u32_little() {
        assert_eq!(
            decode_words(&[0xBEEF, 0xDEAD], ValueType::U32, WordOrder::Little),
            0xDEAD_BEEFu32 as f64
        );
    }

    #[test]
    fn decode_i32_negative() {
        // -2 = 0xFFFFFFFE
        assert_eq!(
            decode_words(&[0xFFFF, 0xFFFE], ValueType::I32, WordOrder::Big),
            -2.0
        );
    }

    #[test]
    fn decode_f32_big() {
        // 12.5f32 = 0x41480000 -> [0x4148, 0x0000]
        assert_eq!(
            decode_words(&[0x4148, 0x0000], ValueType::F32, WordOrder::Big),
            12.5
        );
    }

    #[test]
    fn decode_f32_little() {
        assert_eq!(
            decode_words(&[0x0000, 0x4148], ValueType::F32, WordOrder::Little),
            12.5
        );
    }

    #[test]
    fn raw_sample_rejects_short_response() {
        let reg = RegisterConfig {
            name: "r".into(),
            sensor_id: "s.r".into(),
            function: crate::config::RegisterFunction::Holding,
            address: 0,
            count: Some(2),
            value_type: ValueType::F32,
            word_order: WordOrder::Big,
            unit: None,
        };
        assert!(raw_sample("dev", &reg, &[0x4148]).is_err());
    }
}
