// A `match` on the `#[non_exhaustive]` `Presence` enum that omits a
// variant and has no wildcard arm must not compile — this is what makes
// a presence verdict impossible to silently mishandle.
fn main() {
    let p = nebula_validator::Presence::Active;
    match p {
        nebula_validator::Presence::Active => {}
    }
}
