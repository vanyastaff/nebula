// `nebula_validator::RuleContext` was deleted (no shim). Referencing it
// must not compile — this locks the removal against a silent revival.
fn main() {
    fn _needs_ctx<T: nebula_validator::RuleContext>(_: T) {}
}
