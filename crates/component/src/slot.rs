//! Slots: what a component needs from the system.
//!
//! A slot is a named dependency of one component on a resource or a
//! credential, required or optional. It is declared once, as a struct field
//! whose type implements [`SlotValue`]; the field type says what the slot
//! points at and whether it may stay empty, so neither is written twice.
//!
//! ```
//! use nebula_component::{Component, Dependencies, Slot, SlotValue, kind};
//! use nebula_metadata::{Metadata, metadata_name};
//!
//! struct Postgres;
//!
//! impl Component for Postgres {
//!     type Kind = kind::Resource;
//!     type Config = ();
//!
//!     const KEY: &'static str = "postgres";
//!
//!     fn metadata() -> Metadata {
//!         Metadata::new(metadata_name!("Postgres"), "A connection pool")
//!     }
//! }
//!
//! /// What a field holding the pool would be; the resource crate provides
//! /// the real handle type.
//! struct PoolHandle;
//!
//! impl SlotValue for PoolHandle {
//!     type Target = Postgres;
//! }
//!
//! let dependencies = Dependencies::none()
//!     .slot(Slot::of::<PoolHandle>("db"))
//!     .slot(Slot::of::<Option<PoolHandle>>("replica").with_purpose("Read traffic"));
//!
//! assert!(dependencies.validate().is_ok());
//! assert!(dependencies.get("db").is_some_and(Slot::is_required));
//! assert!(dependencies.get("replica").is_some_and(|slot| !slot.is_required()));
//! ```
//!
//! This crate owns only the declaration. How a slot gets its value (a
//! resource handle, a credential guard) belongs to the resource and
//! credential crates and to the engine that binds a node or a resource row.

use std::any::TypeId;

use domain_key::Key;

use crate::{
    Component,
    component::key_of,
    kind::{self, ComponentKind},
};

/// A kind a slot can point at: a resource or a credential.
///
/// An action is never a dependency, so [`kind::Action`] does not implement
/// it. [`ComponentKind`] is sealed, which keeps this set closed as well.
pub trait DependencyKind: ComponentKind {
    /// The [`SlotTarget`] naming component `C` of this kind.
    #[doc(hidden)]
    fn target_of<C: Component<Kind = Self>>() -> SlotTarget;
}

impl DependencyKind for kind::Resource {
    fn target_of<C: Component<Kind = Self>>() -> SlotTarget {
        SlotTarget::Resource(Target::of::<C>())
    }
}

impl DependencyKind for kind::Credential {
    fn target_of<C: Component<Kind = Self>>() -> SlotTarget {
        SlotTarget::Credential(Target::of::<C>())
    }
}

/// A field type that can hold a slot's value.
///
/// The value types implement it, not the components: a resource handle names
/// the resource it hands out, a credential handle the credential it reads.
/// `Option<V>` is the optional form of any slot value `V`. A derive therefore
/// never parses a field type; it writes `Slot::of::<FieldType>(key)`, and a
/// field of any other type fails with this trait's message.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot hold a slot",
    label = "a `#[slot]` field must hold a slot value",
    note = "a slot field holds a resource handle or a credential handle, optionally in `Option<...>`"
)]
pub trait SlotValue: Send + Sync + 'static {
    /// The component this value comes from.
    type Target: Component<Kind: DependencyKind>;

    /// Whether the slot must be bound. Only `Option<V>` says `false`.
    const REQUIRED: bool = true;
}

impl<V: SlotValue> SlotValue for Option<V> {
    type Target = V::Target;

    const REQUIRED: bool = false;
}

/// The component a slot points at: its typed key and its Rust type.
///
/// The fields are private, and the only constructor reads both from the same
/// type, so a target can never pair one component's key with another's type.
pub struct Target<K: ComponentKind> {
    key: Key<K::Domain>,
    type_id: TypeId,
    type_name: &'static str,
}

impl<K: ComponentKind> Target<K> {
    /// The target naming component `C`.
    #[must_use]
    pub fn of<C: Component<Kind = K>>() -> Self {
        Self {
            key: key_of::<C>(),
            type_id: TypeId::of::<C>(),
            type_name: std::any::type_name::<C>(),
        }
    }

    /// The target component's key.
    #[must_use]
    pub fn key(&self) -> &Key<K::Domain> {
        &self.key
    }

    /// The target component's Rust type, which registration compares with
    /// the type registered under [`key`](Self::key).
    #[must_use]
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// The target component's Rust type name, for diagnostics only.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        self.type_name
    }
}

impl<K: ComponentKind> Clone for Target<K> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            type_id: self.type_id,
            type_name: self.type_name,
        }
    }
}

impl<K: ComponentKind> PartialEq for Target<K> {
    fn eq(&self, other: &Self) -> bool {
        self.type_id == other.type_id && self.key == other.key
    }
}

impl<K: ComponentKind> Eq for Target<K> {}

impl<K: ComponentKind> std::fmt::Debug for Target<K> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Target")
            .field("kind", &K::NAME)
            .field("key", &self.key.as_str())
            .field("type_name", &self.type_name)
            .finish()
    }
}

/// What a slot points at.
///
/// The set is closed, like [`DependencyKind`]: a match over it is exhaustive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotTarget {
    /// A resource.
    Resource(Target<kind::Resource>),
    /// A credential.
    Credential(Target<kind::Credential>),
}

impl SlotTarget {
    /// The target naming component `C`, a resource or a credential.
    #[must_use]
    pub fn of<C>() -> Self
    where
        C: Component<Kind: DependencyKind>,
    {
        <C::Kind as DependencyKind>::target_of::<C>()
    }

    /// The target's kind name: `"resource"` or `"credential"`.
    #[must_use]
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Resource(_) => <kind::Resource as ComponentKind>::NAME,
            Self::Credential(_) => <kind::Credential as ComponentKind>::NAME,
        }
    }

    /// The target component's key as a string.
    #[must_use]
    pub fn key_str(&self) -> &str {
        match self {
            Self::Resource(target) => target.key().as_str(),
            Self::Credential(target) => target.key().as_str(),
        }
    }

    /// The target component's Rust type.
    #[must_use]
    pub fn type_id(&self) -> TypeId {
        match self {
            Self::Resource(target) => target.type_id(),
            Self::Credential(target) => target.type_id(),
        }
    }

    /// The target component's Rust type name, for diagnostics only.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Resource(target) => target.type_name(),
            Self::Credential(target) => target.type_name(),
        }
    }
}

/// One declared dependency: a local name, what it points at, and whether it
/// must be bound.
///
/// The key is the slot's address. A workflow node or a resource row binds a
/// slot by this key; unbound, a resource slot takes the instance registered
/// under the target's own key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    key: &'static str,
    target: SlotTarget,
    required: bool,
    purpose: Option<&'static str>,
}

impl Slot {
    /// The slot `key` held in a field of type `V`.
    ///
    /// `V` gives both the target and the requiredness: `Option<V>` declares
    /// an optional slot. Admission checks the key; see
    /// [`Dependencies::validate`].
    #[must_use]
    pub fn of<V: SlotValue>(key: &'static str) -> Self {
        Self {
            key,
            target: SlotTarget::of::<V::Target>(),
            required: V::REQUIRED,
            purpose: None,
        }
    }

    /// Sets what the slot is for, shown in the catalog and the editor.
    #[must_use]
    pub fn with_purpose(mut self, purpose: &'static str) -> Self {
        self.purpose = Some(purpose);
        self
    }

    /// The slot's key.
    #[must_use]
    pub fn key(&self) -> &'static str {
        self.key
    }

    /// What the slot points at.
    #[must_use]
    pub fn target(&self) -> &SlotTarget {
        &self.target
    }

    /// Whether the slot must be bound.
    #[must_use]
    pub fn is_required(&self) -> bool {
        self.required
    }

    /// What the slot is for, if the author said.
    #[must_use]
    pub fn purpose(&self) -> Option<&'static str> {
        self.purpose
    }
}

/// The slots a component declares, in declaration order.
///
/// A component without slots returns [`Dependencies::none`], which is the
/// default of [`Component::dependencies`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dependencies {
    slots: Vec<Slot>,
}

impl Dependencies {
    /// No slots.
    #[must_use]
    pub const fn none() -> Self {
        Self { slots: Vec::new() }
    }

    /// Adds `slot` after the slots already declared.
    #[must_use]
    pub fn slot(mut self, slot: Slot) -> Self {
        self.slots.push(slot);
        self
    }

    /// The slots in declaration order.
    #[must_use]
    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }

    /// The slot declared under `key`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Slot> {
        self.slots.iter().find(|slot| slot.key == key)
    }

    /// Whether no slot is declared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// The slots that point at a resource.
    pub fn resource_slots(&self) -> impl Iterator<Item = (&Slot, &Target<kind::Resource>)> {
        self.slots.iter().filter_map(|slot| match &slot.target {
            SlotTarget::Resource(target) => Some((slot, target)),
            SlotTarget::Credential(_) => None,
        })
    }

    /// The slots that point at a credential.
    pub fn credential_slots(&self) -> impl Iterator<Item = (&Slot, &Target<kind::Credential>)> {
        self.slots.iter().filter_map(|slot| match &slot.target {
            SlotTarget::Credential(target) => Some((slot, target)),
            SlotTarget::Resource(_) => None,
        })
    }

    /// Checks every slot key and that no key is declared twice.
    ///
    /// Admission runs this; a derive checks the same rules at compile time.
    ///
    /// # Errors
    ///
    /// Returns [`SlotError`] for the first key that is not a valid slot key
    /// or repeats an earlier one.
    pub fn validate(&self) -> Result<(), SlotError> {
        for (index, slot) in self.slots.iter().enumerate() {
            if !is_valid_slot_key(slot.key) {
                return Err(SlotError::InvalidKey { key: slot.key });
            }
            if self.slots[..index]
                .iter()
                .any(|earlier| earlier.key == slot.key)
            {
                return Err(SlotError::DuplicateKey { key: slot.key });
            }
        }
        Ok(())
    }
}

/// Whether `key` is a valid slot key: 1 to 64 bytes of lowercase ASCII
/// letters, digits and `_`, starting with a letter and not ending with `_`.
///
/// A slot key reads like the field it is declared on, which is the default.
#[must_use]
pub const fn is_valid_slot_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    if !bytes[0].is_ascii_lowercase() || bytes[bytes.len() - 1] == b'_' {
        return false;
    }
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_') {
            return false;
        }
        index += 1;
    }
    true
}

/// A component's declared slots break a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SlotError {
    /// The key is not a valid slot key.
    #[error(
        "slot key `{key}` is invalid: use 1 to 64 lowercase ASCII letters, digits and `_`, starting with a letter"
    )]
    InvalidKey {
        /// The rejected key.
        key: &'static str,
    },
    /// Two slots share a key.
    #[error("slot key `{key}` is declared twice")]
    DuplicateKey {
        /// The repeated key.
        key: &'static str,
    },
}

#[cfg(test)]
mod tests {
    use nebula_metadata::{Metadata, metadata_name};

    use super::*;

    struct Pool;
    struct Token;

    impl Component for Pool {
        type Kind = kind::Resource;
        type Config = ();

        const KEY: &'static str = "pool";

        fn metadata() -> Metadata {
            Metadata::new(metadata_name!("Pool"), "")
        }
    }

    impl Component for Token {
        type Kind = kind::Credential;
        type Config = ();

        const KEY: &'static str = "token";

        fn metadata() -> Metadata {
            Metadata::new(metadata_name!("Token"), "")
        }
    }

    struct PoolHandle;
    struct TokenHandle;

    impl SlotValue for PoolHandle {
        type Target = Pool;
    }

    impl SlotValue for TokenHandle {
        type Target = Token;
    }

    #[test]
    fn the_field_type_gives_the_target_and_the_requiredness() {
        let db = Slot::of::<PoolHandle>("db");
        let auth = Slot::of::<Option<TokenHandle>>("auth");

        assert!(db.is_required());
        assert!(!auth.is_required());
        assert_eq!(db.target(), &SlotTarget::Resource(Target::of::<Pool>()));
        assert_eq!(auth.target().kind_name(), "credential");
        assert_eq!(auth.target().key_str(), "token");
        assert_eq!(auth.target().type_id(), TypeId::of::<Token>());
    }

    #[test]
    fn slots_keep_declaration_order_and_split_by_kind() {
        let dependencies = Dependencies::none()
            .slot(Slot::of::<TokenHandle>("auth"))
            .slot(Slot::of::<PoolHandle>("db"));

        let keys: Vec<_> = dependencies.slots().iter().map(Slot::key).collect();
        assert_eq!(keys, ["auth", "db"]);
        assert_eq!(dependencies.resource_slots().count(), 1);
        assert_eq!(dependencies.credential_slots().count(), 1);
        assert!(dependencies.validate().is_ok());
    }

    #[test]
    fn validation_rejects_a_repeated_or_malformed_key() {
        let repeated = Dependencies::none()
            .slot(Slot::of::<PoolHandle>("db"))
            .slot(Slot::of::<TokenHandle>("db"));
        assert_eq!(
            repeated.validate(),
            Err(SlotError::DuplicateKey { key: "db" })
        );

        for key in ["", "Db", "1db", "db_", "db.main", "db-main"] {
            let malformed = Dependencies::none().slot(Slot::of::<PoolHandle>(key));
            assert_eq!(malformed.validate(), Err(SlotError::InvalidKey { key }));
        }
    }

    #[test]
    fn a_slot_key_reads_like_a_field_name() {
        assert!(is_valid_slot_key("db"));
        assert!(is_valid_slot_key("api_key2"));
        assert!(!is_valid_slot_key(&"a".repeat(65)));
    }
}
