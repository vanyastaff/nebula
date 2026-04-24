//! Probe 3 — RefreshDispatcher::for_credential::<NonRefreshable>() rejected.
//!
//! Expected: E0277 — bound `Refreshable` not satisfied.

use credential_proto::RefreshDispatcher;
use credential_proto_builtin::ApiKeyCredential;

fn main() {
    // ApiKeyCredential does not impl Refreshable.
    let _d = RefreshDispatcher::<ApiKeyCredential>::for_credential();
}
