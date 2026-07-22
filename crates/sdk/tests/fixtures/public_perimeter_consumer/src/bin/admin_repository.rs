fn name_admin_repository<
    S: nebula_sdk::integration::credential::OwnerScopedCredentialRepository,
>() {
}

fn main() {
    let _ = name_admin_repository::<()>;
}
