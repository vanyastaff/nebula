//! Compile-time assertion of the D2 hook shape: &self + slot + &Runtime,
//! plus on_credential_revoke. Body is never run; compilation is the test.
#![allow(dead_code, unused)]
use nebula_resource::Resource;

fn assert_send<T: Send>(_: T) {}

fn refresh_shape<R: Resource>(r: &R, rt: &R::Runtime) {
    let _f = r.on_credential_refresh("slot", rt);
    let _g = r.on_credential_revoke("slot", rt);
    assert_send(r.on_credential_refresh("slot", rt));
    assert_send(r.on_credential_revoke("slot", rt));
}

#[test]
fn hook_shape_compiles() {
    // The free fn above is the assertion; nothing to run.
}
