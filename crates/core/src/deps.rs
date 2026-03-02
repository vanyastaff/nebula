use std::fmt;
use std::marker::PhantomData;

/// Kind of component participating in the dependency graph.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum DependencyKind {
    /// Runtime resource (e.g. database, HTTP client).
    Resource,
    /// Credential definition / provider.
    Credential,
    /// Action type (node implementation in a workflow).
    Action,
}

/// A single edge endpoint in the dependency graph.
///
/// This is intentionally minimal: everything is identified by a static
/// string key and a high‑level kind. Individual crates decide how to map
/// their types and metadata onto these identifiers.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Dependency {
    /// Component kind (resource / credential / action).
    pub kind: DependencyKind,
    /// Stable identifier for the component type.
    ///
    /// Typically this is a static key such as a resource ID
    /// (`"postgres"`) or action key (`"google.drive.list_files"`).
    pub id: &'static str,
}

/// Errors produced while validating or resolving dependencies.
#[derive(Debug)]
pub enum DependencyError {
    /// A required dependency was not registered.
    Missing {
        /// Component that declared the dependency.
        required_by: Dependency,
        /// The missing component.
        missing: Dependency,
    },

    /// A cycle was detected in the dependency graph.
    Cycle {
        /// Components participating in the cycle, in order.
        path: Vec<Dependency>,
    },

    /// Invariant in the backing registry was violated.
    ///
    /// This indicates a bug in the engine rather than user configuration.
    RegistryInvariant(&'static str),
}

impl fmt::Display for DependencyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DependencyError::Missing { required_by, missing } => {
                write!(
                    f,
                    "missing dependency: {:?} depends on {:?}, but it is not registered",
                    required_by, missing
                )
            }
            DependencyError::Cycle { path } => {
                write!(f, "dependency cycle detected: ")?;
                for (i, dep) in path.iter().enumerate() {
                    if i > 0 {
                        write!(f, " -> ")?;
                    }
                    write!(f, "{:?}", dep)?;
                }
                Ok(())
            }
            DependencyError::RegistryInvariant(msg) => {
                write!(f, "registry invariant violated: {msg}")
            }
        }
    }
}

impl std::error::Error for DependencyError {}

/// Abstract view over a resolved component registry.
///
/// The core crate intentionally does not know about concrete managers
/// (resource manager, credential manager, action registry, etc.).
/// Those crates implement this trait to bridge their own registries
/// into the generic [`FromRegistry`] world.
pub trait ResolvedRegistry {
    /// Ensure that a resource with the given identifier is available.
    fn ensure_resource(&self, _id: &'static str) -> Result<(), DependencyError> {
        Ok(())
    }

    /// Ensure that a credential with the given identifier is available.
    fn ensure_credential(&self, _id: &'static str) -> Result<(), DependencyError> {
        Ok(())
    }

    /// Ensure that an action with the given identifier is available.
    fn ensure_action(&self, _id: &'static str) -> Result<(), DependencyError> {
        Ok(())
    }
}

/// Types that can declare dependencies and be resolved from a registry.
pub trait FromRegistry: Sized + Send + Sync + 'static {
    /// Static declaration of dependencies required by this type.
    fn dependencies() -> Vec<Dependency>;

    /// Resolve this type from an already‑initialized registry.
    fn resolve(registry: &dyn ResolvedRegistry) -> Result<Self, DependencyError>;
}

/// Marker type describing a dependency on component `T`.
///
/// This type carries no data at runtime; all information lives in the
/// registry and in the associated `T` metadata.
pub struct Requires<T> {
    _marker: PhantomData<T>,
}

impl<T> Requires<T> {
    /// Create a new marker instance.
    pub const fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

/// No dependencies.
impl FromRegistry for () {
    fn dependencies() -> Vec<Dependency> {
        Vec::new()
    }

    fn resolve(_: &dyn ResolvedRegistry) -> Result<Self, DependencyError> {
        Ok(())
    }
}

impl<A, B> FromRegistry for (A, B)
where
    A: FromRegistry,
    B: FromRegistry,
{
    fn dependencies() -> Vec<Dependency> {
        let mut v = A::dependencies();
        v.extend(B::dependencies());
        v
    }

    fn resolve(registry: &dyn ResolvedRegistry) -> Result<Self, DependencyError> {
        Ok((A::resolve(registry)?, B::resolve(registry)?))
    }
}

impl<A, B, C> FromRegistry for (A, B, C)
where
    A: FromRegistry,
    B: FromRegistry,
    C: FromRegistry,
{
    fn dependencies() -> Vec<Dependency> {
        let mut v = A::dependencies();
        v.extend(B::dependencies());
        v.extend(C::dependencies());
        v
    }

    fn resolve(registry: &dyn ResolvedRegistry) -> Result<Self, DependencyError> {
        Ok((
            A::resolve(registry)?,
            B::resolve(registry)?,
            C::resolve(registry)?,
        ))
    }
}

/// Macro for declaring dependency tuples in a concise form.
///
/// Expands:
/// - `deps![]` or `deps!()` into `()`
/// - `deps![T]` into `Requires<T>`
/// - `deps![A, B, C]` into `(Requires<A>, Requires<B>, Requires<C>)`
#[macro_export]
macro_rules! deps {
    () => {
        ()
    };
    ($T:ty) => {
        $crate::deps::Requires<$T>
    };
    ($($T:ty),+ $(,)?) => {
        ( $( $crate::deps::Requires<$T> ),+ )
    };
}

