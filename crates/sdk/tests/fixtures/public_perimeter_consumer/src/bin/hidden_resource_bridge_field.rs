fn extract_factory<R, FResource, FTopology>(
    contribution: nebula_sdk::__private::resource::contribution::ResourceContributionBridge<
        R,
        FResource,
        FTopology,
    >,
) {
    let _ = contribution.factory;
}

fn main() {
    let _ = extract_factory::<(), (), ()>;
}
