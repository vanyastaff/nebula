#![no_main]

use libfuzzer_sys::fuzz_target;
use nebula_value::Value;
use arbitrary::Arbitrary;

#[derive(Arbitrary, Debug)]
enum Operation {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    And,
    Or,
    Not,
    Merge,
}

#[derive(Arbitrary, Debug)]
enum FuzzValue {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
}

impl FuzzValue {
    fn to_value(&self) -> Value {
        match self {
            FuzzValue::Null => Value::null(),
            FuzzValue::Bool(b) => Value::boolean(*b),
            FuzzValue::Integer(i) => Value::integer(*i),
            FuzzValue::Float(f) => Value::float(*f),
            FuzzValue::Text(s) => Value::text(s.clone()),
            FuzzValue::Bytes(b) => Value::bytes(b.clone()),
        }
    }
}

fuzz_target!(|data: (FuzzValue, FuzzValue, Operation)| {
    let (val1_fuzz, val2_fuzz, op) = data;

    let val1 = val1_fuzz.to_value();
    let val2 = val2_fuzz.to_value();

    // Try different operations - they should either succeed or return proper errors
    match op {
        Operation::Add => {
            let _ = val1.add(&val2);
        }
        Operation::Sub => {
            let _ = val1.sub(&val2);
        }
        Operation::Mul => {
            let _ = val1.mul(&val2);
        }
        Operation::Div => {
            let _ = val1.div(&val2);
        }
        Operation::Eq => {
            let _ = val1.eq(&val2);
        }
        Operation::And => {
            let _ = val1.and(&val2);
        }
        Operation::Or => {
            let _ = val1.or(&val2);
        }
        Operation::Not => {
            let _ = val1.not();
        }
        Operation::Merge => {
            let _ = val1.merge(&val2);
        }
    }

    // Clone should always work
    let _ = val1.clone();
    let _ = val2.clone();
});