//! A session never outlives its body: the body cannot keep the borrowed
//! session.

use nebula_sdk::integration::resource::{
    Cost, ManagedRow, PoolProvider, Pooled, Provider, SessionProvider, SessionSpec,
};

fn smuggle<R>(row: &ManagedRow<R>)
where
    R: SessionProvider + PoolProvider + Provider<Topology = Pooled<R>> + Clone,
{
    let mut escaped = None;
    let _unit = row.session(SessionSpec::new(Cost::ONE), |tx, _cx| {
        escaped = Some(tx);
        Box::pin(async { Ok(()) })
    });
}

fn main() {}
