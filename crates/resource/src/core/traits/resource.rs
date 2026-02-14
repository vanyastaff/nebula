pub trait Resource: Send + Sync + 'static {
    type Instance;
    type Config;
}