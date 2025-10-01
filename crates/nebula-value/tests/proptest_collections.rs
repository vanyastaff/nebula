//! Property-based tests for collection types (Array, Object)

use nebula_value::{Array, Object};
use proptest::prelude::*;
use serde_json::json;

// ===== ARRAY PROPERTIES =====

proptest! {
    #[test]
    fn array_length_matches_vec(items in prop::collection::vec(any::<i32>(), 0..100)) {
        let json_items: Vec<_> = items.iter().map(|&i| json!(i)).collect();
        let array = Array::from_vec(json_items);

        prop_assert_eq!(array.len(), items.len());
    }

    #[test]
    fn array_empty_iff_zero_length(items in prop::collection::vec(any::<i32>(), 0..10)) {
        let json_items: Vec<_> = items.iter().map(|&i| json!(i)).collect();
        let array = Array::from_vec(json_items);

        prop_assert_eq!(array.is_empty(), items.is_empty());
        prop_assert_eq!(array.is_empty(), array.len() == 0);
    }

    #[test]
    fn array_get_in_bounds(items in prop::collection::vec(any::<i32>(), 1..100), idx in 0usize..99) {
        let json_items: Vec<_> = items.iter().map(|&i| json!(i)).collect();
        let array = Array::from_vec(json_items.clone());

        if idx < items.len() {
            let value = array.get(idx);
            prop_assert!(value.is_some());
            prop_assert_eq!(value, Some(&json_items[idx]));
        }
    }

    #[test]
    fn array_push_increases_length(items in prop::collection::vec(any::<i32>(), 0..100), new_val in any::<i32>()) {
        let json_items: Vec<_> = items.iter().map(|&i| json!(i)).collect();
        let array = Array::from_vec(json_items);
        let original_len = array.len();

        let new_array = array.push(json!(new_val));

        prop_assert_eq!(new_array.len(), original_len + 1);
        prop_assert_eq!(new_array.get(original_len), Some(&json!(new_val)));
    }

    #[test]
    fn array_concat_length_sum(a in prop::collection::vec(any::<i32>(), 0..50), b in prop::collection::vec(any::<i32>(), 0..50)) {
        let json_a: Vec<_> = a.iter().map(|&i| json!(i)).collect();
        let json_b: Vec<_> = b.iter().map(|&i| json!(i)).collect();

        let arr_a = Array::from_vec(json_a);
        let arr_b = Array::from_vec(json_b);

        let concat = arr_a.concat(&arr_b);

        prop_assert_eq!(concat.len(), a.len() + b.len());
    }

    #[test]
    fn array_clone_equals_original(items in prop::collection::vec(any::<i32>(), 0..100)) {
        let json_items: Vec<_> = items.iter().map(|&i| json!(i)).collect();
        let array = Array::from_vec(json_items.clone());
        let cloned = array.clone();

        prop_assert_eq!(array.len(), cloned.len());

        for (i, _item) in json_items.iter().enumerate() {
            prop_assert_eq!(array.get(i), cloned.get(i));
        }
    }

    #[test]
    fn array_iter_length_matches(items in prop::collection::vec(any::<i32>(), 0..100)) {
        let json_items: Vec<_> = items.iter().map(|&i| json!(i)).collect();
        let array = Array::from_vec(json_items);

        let iter_count = array.iter().count();
        prop_assert_eq!(iter_count, items.len());
    }

    #[test]
    fn array_push_original_unchanged(items in prop::collection::vec(any::<i32>(), 1..50), new_val in any::<i32>()) {
        let json_items: Vec<_> = items.iter().map(|&i| json!(i)).collect();
        let original = Array::from_vec(json_items.clone());
        let original_len = original.len();

        let _modified = original.push(json!(new_val));

        // Original should be unchanged
        prop_assert_eq!(original.len(), original_len);
        for (i, item) in json_items.iter().enumerate() {
            prop_assert_eq!(original.get(i), Some(item));
        }
    }
}

// ===== OBJECT PROPERTIES =====

proptest! {
    #[test]
    fn object_empty_iff_zero_length(entries in prop::collection::vec((".*", any::<i32>()), 0..10)) {
        let json_entries: Vec<_> = entries.iter()
            .map(|(k, v)| (k.clone(), json!(v)))
            .collect();
        let object = Object::from_iter(json_entries);

        prop_assert_eq!(object.is_empty(), entries.is_empty());
        prop_assert_eq!(object.is_empty(), object.len() == 0);
    }

    #[test]
    fn object_get_existing_key(entries in prop::collection::vec((".*", any::<i32>()), 1..20)) {
        let json_entries: Vec<_> = entries.iter()
            .map(|(k, v)| (k.clone(), json!(v)))
            .collect();
        let object = Object::from_iter(json_entries.clone());

        if let Some((key, _)) = json_entries.first() {
            // Just check that key exists (value might differ if key was duplicated)
            let value = object.get(key);
            prop_assert!(value.is_some());
        }
    }

    #[test]
    fn object_contains_key_consistency(entries in prop::collection::vec((".*", any::<i32>()), 1..20)) {
        let json_entries: Vec<_> = entries.iter()
            .map(|(k, v)| (k.clone(), json!(v)))
            .collect();
        let object = Object::from_iter(json_entries.clone());

        if let Some((key, _)) = json_entries.first() {
            prop_assert_eq!(object.contains_key(key), object.get(key).is_some());
        }
    }

    #[test]
    fn object_insert_preserves_value(key in ".*", val in any::<i32>()) {
        let object = Object::new();
        let new_object = object.insert(key.clone(), json!(val));

        prop_assert_eq!(new_object.get(&key), Some(&json!(val)));
    }

    #[test]
    fn object_clone_equals_original(entries in prop::collection::vec((".*", any::<i32>()), 0..20)) {
        let json_entries: Vec<_> = entries.iter()
            .map(|(k, v)| (k.clone(), json!(v)))
            .collect();
        let object = Object::from_iter(json_entries.clone());
        let cloned = object.clone();

        prop_assert_eq!(object.len(), cloned.len());

        // Just check that all keys from object are in clone with same values
        for (key, val) in json_entries.iter() {
            prop_assert_eq!(object.get(key), cloned.get(key));
        }
    }

    #[test]
    fn object_insert_original_unchanged(key in ".*", val in any::<i32>()) {
        let original = Object::new();
        let original_len = original.len();

        let _modified = original.insert(key.clone(), json!(val));

        // Original should still be empty
        prop_assert_eq!(original.len(), original_len);
        prop_assert!(!original.contains_key(&key));
    }
}