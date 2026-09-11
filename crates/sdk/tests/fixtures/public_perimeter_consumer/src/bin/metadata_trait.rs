fn name_metadata<T: nebula_sdk::prelude::Metadata>() {}

fn main() {
    let _ = name_metadata::<()>;
}
