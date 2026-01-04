#![no_main]

use libfuzzer_sys::fuzz_target;
use nebula_value::{Array, Object, Value};
use arbitrary::Arbitrary;

#[derive(Arbitrary, Debug)]
enum CollectionOp {
    ArrayPush,
    ArrayConcat,
    ArrayGet(usize),
    ObjectInsert(String),
    ObjectGet(String),
    ObjectMerge,
}

fuzz_target!(|data: (Vec<i32>, Vec<(String, i32)>, CollectionOp)| {
    let (array_data, object_data, op) = data;

    // Create array
    let array_items: Vec<_> = array_data.iter().map(|&i| Value::integer(i as i64)).collect();
    let array = Array::from_vec(array_items);

    // Create object
    let object_items: Vec<_> = object_data.iter()
        .map(|(k, v)| (k.clone(), Value::integer(*v as i64)))
        .collect();
    let object = Object::from_iter(object_items);

    // Execute operation
    match op {
        CollectionOp::ArrayPush => {
            let _ = array.push(42);
        }
        CollectionOp::ArrayConcat => {
            let _ = array.concat(&array);
        }
        CollectionOp::ArrayGet(idx) => {
            let _ = array.get(idx);
        }
        CollectionOp::ObjectInsert(key) => {
            let _ = object.insert(key, 123);
        }
        CollectionOp::ObjectGet(key) => {
            let _ = object.get(&key);
        }
        CollectionOp::ObjectMerge => {
            let _ = object.merge(&object);
        }
    }

    // Clone should always work
    let _ = array.clone();
    let _ = object.clone();

    // Iteration should work
    for _ in array.iter() {}
    for _ in object.keys() {}
    for _ in object.values() {}
});