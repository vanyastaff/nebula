use nebula_core::ResourceKey;

use super::{Manager, ManagerConfig};
use crate::events::ResourceEvent;

fn registered(n: usize) -> ResourceEvent {
    ResourceEvent::Registered {
        key: ResourceKey::new(format!("bus-{n}")).expect("valid static resource key"),
    }
}

#[tokio::test]
async fn zero_capacity_is_clamped_instead_of_panicking() {
    assert_eq!(
        ManagerConfig::default()
            .with_event_bus_capacity(0)
            .event_bus_capacity,
        1
    );
    // The public field bypasses the setter; construction must still not panic.
    let _manager = Manager::with_config(ManagerConfig {
        event_bus_capacity: 0,
        ..ManagerConfig::default()
    });
}

#[tokio::test]
async fn overflowing_the_configured_capacity_is_reported_as_drops() {
    let manager = Manager::with_config(ManagerConfig::default().with_event_bus_capacity(2));
    let mut slow = manager.subscribe_events();
    for n in 0..6 {
        manager.emit(registered(n));
    }
    // DropOldest attributes the lag when the slow subscriber next pulls.
    let first = slow.recv().await.expect("the newest events survive");
    assert!(matches!(first, ResourceEvent::Registered { .. }));

    let stats = manager.event_bus_stats();
    assert_eq!(stats.sent_count, 6);
    assert_eq!(
        stats.dropped_count, 4,
        "6 events through a 2-slot bus lose 4"
    );
}
