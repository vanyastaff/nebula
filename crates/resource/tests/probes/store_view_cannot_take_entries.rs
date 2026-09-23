use nebula_resource::topology::StoreView;

async fn take_from_hook(view: StoreView<'_, u32>) {
    let _ = view.checkout().await;
    let _ = view.drain_all().await;
    view.bump_revoke_epoch();
}

fn main() {}
