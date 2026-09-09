use nebula_resource::{RetainedId, RetainedStore};

fn reenter_store(store: &RetainedStore<u32>, id: RetainedId) {
    store.with_entry(id, |_| store.retire(id));
}

fn main() {}
