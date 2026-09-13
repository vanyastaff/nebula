use nebula_validator::{DeferredRule, Predicate, Rule, RuleRef, RuleView, ValueRule};

use super::{canonical::Writer, model::AdmissionIssue};

pub(super) fn write_rules(output: &mut Writer, rules: &[Rule]) -> Result<(), AdmissionIssue> {
    output.count(rules.len())?;
    for rule in rules {
        write_rule(output, rule.root())?;
    }
    Ok(())
}

pub(super) fn write_rule(output: &mut Writer, rule: RuleRef<'_>) -> Result<(), AdmissionIssue> {
    match rule.view() {
        RuleView::Value(value) => {
            output.u8(0x30)?;
            write_value_rule(output, value)
        },
        RuleView::Predicate(predicate) => {
            output.u8(0x31)?;
            write_predicate(output, predicate)
        },
        RuleView::All(children) => {
            output.u8(0x32)?;
            output.count(children.len())?;
            for child in children {
                write_rule(output, child)?;
            }
            Ok(())
        },
        RuleView::Any(children) => {
            output.u8(0x33)?;
            output.count(children.len())?;
            for child in children {
                write_rule(output, child)?;
            }
            Ok(())
        },
        RuleView::Not(inner) => {
            output.u8(0x34)?;
            write_rule(output, inner)
        },
        RuleView::Deferred(deferred) => {
            output.u8(0x35)?;
            write_deferred(output, deferred)
        },
        RuleView::Described { inner, message } => {
            output.u8(0x36)?;
            write_rule(output, inner)?;
            output.string(message)
        },
        _ => Err(AdmissionIssue::InvalidRule),
    }
}

fn write_value_rule(output: &mut Writer, rule: &ValueRule) -> Result<(), AdmissionIssue> {
    match rule {
        ValueRule::MinLength(value) => write_usize(output, 0x40, *value),
        ValueRule::MaxLength(value) => write_usize(output, 0x41, *value),
        ValueRule::Pattern(pattern) => {
            output.u8(0x42)?;
            output.string(pattern.as_str())
        },
        ValueRule::Min(number) => write_number(output, 0x43, number),
        ValueRule::Max(number) => write_number(output, 0x44, number),
        ValueRule::GreaterThan(number) => write_number(output, 0x45, number),
        ValueRule::LessThan(number) => write_number(output, 0x46, number),
        ValueRule::OneOf(values) => {
            output.u8(0x47)?;
            output.count(values.len())?;
            for value in values {
                output.exact_json(value)?;
            }
            Ok(())
        },
        ValueRule::MinItems(value) => write_usize(output, 0x48, *value),
        ValueRule::MaxItems(value) => write_usize(output, 0x49, *value),
        ValueRule::Email => output.u8(0x4a),
        ValueRule::Url => output.u8(0x4b),
        _ => Err(AdmissionIssue::InvalidRule),
    }
}

fn write_predicate(output: &mut Writer, predicate: &Predicate) -> Result<(), AdmissionIssue> {
    match predicate {
        Predicate::Eq(path, value) => write_value_predicate(output, 0x50, path.as_str(), value),
        Predicate::Ne(path, value) => write_value_predicate(output, 0x51, path.as_str(), value),
        Predicate::Gt(path, number) => write_number_predicate(output, 0x52, path.as_str(), number),
        Predicate::Gte(path, number) => write_number_predicate(output, 0x53, path.as_str(), number),
        Predicate::Lt(path, number) => write_number_predicate(output, 0x54, path.as_str(), number),
        Predicate::Lte(path, number) => write_number_predicate(output, 0x55, path.as_str(), number),
        Predicate::IsTrue(path) => write_path_predicate(output, 0x56, path.as_str()),
        Predicate::IsFalse(path) => write_path_predicate(output, 0x57, path.as_str()),
        Predicate::Set(path) => write_path_predicate(output, 0x58, path.as_str()),
        Predicate::Empty(path) => write_path_predicate(output, 0x59, path.as_str()),
        Predicate::Contains(path, value) => {
            write_value_predicate(output, 0x5a, path.as_str(), value)
        },
        Predicate::Matches(path, pattern) => {
            output.u8(0x5b)?;
            output.string(path.as_str())?;
            output.string(pattern.as_str())
        },
        Predicate::In(path, values) => {
            output.u8(0x5c)?;
            output.string(path.as_str())?;
            output.count(values.len())?;
            for value in values {
                output.exact_json(value)?;
            }
            Ok(())
        },
        _ => Err(AdmissionIssue::InvalidRule),
    }
}

fn write_deferred(output: &mut Writer, rule: &DeferredRule) -> Result<(), AdmissionIssue> {
    match rule {
        DeferredRule::Custom(expression) => {
            output.u8(0x60)?;
            output.string(expression)
        },
        DeferredRule::UniqueBy(path) => {
            output.u8(0x61)?;
            output.string(path.as_str())
        },
        _ => Err(AdmissionIssue::InvalidRule),
    }
}

fn write_usize(output: &mut Writer, tag: u8, value: usize) -> Result<(), AdmissionIssue> {
    output.u8(tag)?;
    output.u64(u64::try_from(value).map_err(|_| AdmissionIssue::IndexOverflow)?)
}

fn write_number(
    output: &mut Writer,
    tag: u8,
    number: &serde_json::Number,
) -> Result<(), AdmissionIssue> {
    output.u8(tag)?;
    output.number(number)
}

fn write_path_predicate(output: &mut Writer, tag: u8, path: &str) -> Result<(), AdmissionIssue> {
    output.u8(tag)?;
    output.string(path)
}

fn write_value_predicate(
    output: &mut Writer,
    tag: u8,
    path: &str,
    value: &serde_json::Value,
) -> Result<(), AdmissionIssue> {
    output.u8(tag)?;
    output.string(path)?;
    output.exact_json(value)
}

fn write_number_predicate(
    output: &mut Writer,
    tag: u8,
    path: &str,
    number: &serde_json::Number,
) -> Result<(), AdmissionIssue> {
    output.u8(tag)?;
    output.string(path)?;
    output.number(number)
}
