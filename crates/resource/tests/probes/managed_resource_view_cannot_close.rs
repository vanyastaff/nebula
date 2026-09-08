use nebula_resource::ManagedResourceView;

fn bypass_manager(view: &ManagedResourceView) {
    view.begin_close();
}

fn main() {}
