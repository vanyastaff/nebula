use nebula_sdk::prelude::{DeprecationNotice, MetadataVersion};

fn main() {
    let notice = DeprecationNotice::new(MetadataVersion::new(1, 0, 0));
    let _ = notice.since;
}
