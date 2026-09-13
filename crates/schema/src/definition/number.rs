use std::cmp::Ordering;

use serde_json::Number;

use super::model::AdmissionIssue;

pub(super) fn compare_numbers(left: &Number, right: &Number) -> Result<Ordering, AdmissionIssue> {
    let left = Decimal::parse(left)?;
    let right = Decimal::parse(right)?;
    left.compare(&right)
}

pub(super) fn write_canonical_number(
    number: &Number,
    output: &mut Vec<u8>,
) -> Result<(), AdmissionIssue> {
    Decimal::parse(number)?.write_canonical(output);
    Ok(())
}

pub(super) fn write_equality_number(
    number: &Number,
    output: &mut Vec<u8>,
) -> Result<(), AdmissionIssue> {
    let decimal = Decimal::parse(number)?;
    if number.is_f64() && decimal.is_zero() {
        output.extend_from_slice(b"0.0");
    } else {
        output.extend_from_slice(number.to_string().as_bytes());
    }
    Ok(())
}

struct Decimal {
    is_negative: bool,
    digits: Vec<u8>,
    exponent: i64,
}

impl Decimal {
    fn parse(number: &Number) -> Result<Self, AdmissionIssue> {
        let rendered = number.to_string();
        let unsigned = rendered.strip_prefix('-').unwrap_or(&rendered);
        let (mantissa, exponent) = unsigned.split_once(['e', 'E']).map_or(
            Ok((unsigned, 0i64)),
            |(mantissa, exponent)| {
                exponent
                    .parse::<i64>()
                    .map(|exponent| (mantissa, exponent))
                    .map_err(|_| AdmissionIssue::InvalidBounds)
            },
        )?;
        let fractional_digits = mantissa
            .split_once('.')
            .map_or(0usize, |(_, fraction)| fraction.len());
        let mut digits = mantissa
            .bytes()
            .filter(u8::is_ascii_digit)
            .skip_while(|byte| *byte == b'0')
            .collect::<Vec<_>>();
        if digits.is_empty() {
            return Ok(Self {
                is_negative: false,
                digits: vec![b'0'],
                exponent: 0,
            });
        }
        let mut exponent = exponent
            .checked_sub(
                i64::try_from(fractional_digits).map_err(|_| AdmissionIssue::InvalidBounds)?,
            )
            .ok_or(AdmissionIssue::InvalidBounds)?;
        while digits.len() > 1 && digits.last() == Some(&b'0') {
            digits.pop();
            exponent = exponent
                .checked_add(1)
                .ok_or(AdmissionIssue::InvalidBounds)?;
        }
        Ok(Self {
            is_negative: rendered.starts_with('-'),
            digits,
            exponent,
        })
    }

    fn compare(&self, other: &Self) -> Result<Ordering, AdmissionIssue> {
        if self.is_negative != other.is_negative {
            return Ok(if self.is_negative {
                Ordering::Less
            } else {
                Ordering::Greater
            });
        }
        let magnitude = self.compare_magnitude(other)?;
        Ok(if self.is_negative {
            magnitude.reverse()
        } else {
            magnitude
        })
    }

    fn write_canonical(&self, output: &mut Vec<u8>) {
        if self.is_negative {
            output.push(b'-');
        }
        output.extend_from_slice(&self.digits);
        output.push(b'e');
        output.extend_from_slice(self.exponent.to_string().as_bytes());
    }

    fn is_zero(&self) -> bool {
        self.digits == *b"0"
    }

    fn compare_magnitude(&self, other: &Self) -> Result<Ordering, AdmissionIssue> {
        let self_order = i64::try_from(self.digits.len())
            .map_err(|_| AdmissionIssue::InvalidBounds)?
            .checked_add(self.exponent)
            .ok_or(AdmissionIssue::InvalidBounds)?;
        let other_order = i64::try_from(other.digits.len())
            .map_err(|_| AdmissionIssue::InvalidBounds)?
            .checked_add(other.exponent)
            .ok_or(AdmissionIssue::InvalidBounds)?;
        Ok(self_order.cmp(&other_order).then_with(|| {
            let width = self.digits.len().max(other.digits.len());
            (0..width)
                .map(|index| self.digits.get(index).copied().unwrap_or(b'0'))
                .cmp((0..width).map(|index| other.digits.get(index).copied().unwrap_or(b'0')))
        }))
    }
}
