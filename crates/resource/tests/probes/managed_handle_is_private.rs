use nebula_resource::ManagedHandle;

fn bypass_manager(handle: &dyn ManagedHandle) {
    handle.begin_close();
    handle.abort_maintenance();
}

fn main() {}
