#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionFilter {
    None,
    NodeTypes(Vec<String>),
    Categories(Vec<String>),
}