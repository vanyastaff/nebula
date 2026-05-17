//! ADR-0052: the field reference is carried *inside* the plan; there is no
//! parallel decls/entries collection to desync. A reordered `plans` carries
//! its payload with it, so positional cross-wiring is unrepresentable.
// Chained `todo!()` placeholders make later initializers unreachable; the only
// assertion this fixture makes is the `#[non_exhaustive]` E0639, so suppress
// the incidental (compiler-version-sensitive) unreachable-expression warning.
#![allow(unreachable_code)]
fn main() {
    // FieldPlan has no public constructor; a runner cannot fabricate a plan
    // pointing at a different field's payload.
    let _ = nebula_validator::policy::FieldPlan {
        path: todo!(),
        presence: todo!(),
        requiredness: todo!(),
        directive: todo!(),
        payload: todo!(),
    };
}
