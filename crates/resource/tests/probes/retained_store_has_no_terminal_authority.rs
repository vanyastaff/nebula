use nebula_resource::RetainedStore;

fn require_clone<T: Clone>() {}

fn bypass_framework(store: &RetainedStore<u32>) {
    require_clone::<RetainedStore<u32>>();
    let _ = store.drain_all();
    let _ = store.drain_retired();
    let _ = store.begin_close();
}

fn main() {}
