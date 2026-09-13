//! Versioned canonical encoding of the single canonical tree representation.

use std::convert::Infallible;

use nebula_validator::foundation::FieldPath as ValuePath;

use super::{
    ContentId, VALUE_CANON_VERSION,
    canonical::{write_json_v1, write_lp, write_varint},
    tree::ValueTree,
};
use crate::{
    Expression, ProgramSyntax, ValidationError,
    commitment::{CommitmentId, CommitmentKey, SecretCommitmentPolicy, write_secret_commitment},
};

const DOMAIN: &[u8] = b"nbschema-value-v";
const LIST: u8 = 0x06;
const OBJECT: u8 = 0x07;
const EXPRESSION: u8 = 0x08;

impl<E> ValueTree<E> {
    fn canonical_with(
        &self,
        source: &impl Fn(&E) -> Result<(ProgramSyntax, &str), ValidationError>,
        policy: &SecretCommitmentPolicy<'_>,
    ) -> Result<Vec<u8>, ValidationError> {
        self.check_depth(&ValuePath::root(), 0)?;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(DOMAIN);
        bytes.extend_from_slice(&VALUE_CANON_VERSION.to_be_bytes());
        self.write_tree(&mut bytes, source, policy)?;
        Ok(bytes)
    }

    fn write_tree(
        &self,
        bytes: &mut Vec<u8>,
        source: &impl Fn(&E) -> Result<(ProgramSyntax, &str), ValidationError>,
        policy: &SecretCommitmentPolicy<'_>,
    ) -> Result<(), ValidationError> {
        match self {
            Self::Literal(value) => write_json_v1(value.as_json(), bytes, 0)?,
            Self::Object(values) => {
                bytes.push(OBJECT);
                write_varint(bytes, values.len() as u64);
                let mut entries: Vec<_> = values.iter().collect();
                entries.sort_unstable_by(|(left, _), (right, _)| {
                    left.as_bytes().cmp(right.as_bytes())
                });
                for (key, value) in entries {
                    write_lp(bytes, key.as_bytes());
                    value.write_tree(bytes, source, policy)?;
                }
            },
            Self::List(values) => {
                bytes.push(LIST);
                write_varint(bytes, values.len() as u64);
                for value in values {
                    value.write_tree(bytes, source, policy)?;
                }
            },
            Self::Expression(expression) => {
                let (syntax, source) = source(expression)?;
                bytes.push(EXPRESSION);
                bytes.push(match syntax {
                    ProgramSyntax::Auto => 0,
                    ProgramSyntax::Expression => 1,
                    ProgramSyntax::Template => 2,
                });
                write_lp(bytes, source.as_bytes());
            },
            Self::Secret(secret) => match policy {
                SecretCommitmentPolicy::Reject => {
                    return Err(ValidationError::builder("secret.not_hashable")
                        .message("secret values require an explicit keyed commitment")
                        .build());
                },
                SecretCommitmentPolicy::Keyed(key) => write_secret_commitment(secret, key, bytes),
            },
        }
        Ok(())
    }

    pub(crate) fn canonical_data_bytes(&self) -> Result<Vec<u8>, ValidationError> {
        self.canonical_with(
            &|_| {
                Err(ValidationError::builder("expression.unresolved")
                    .message("canonical data still contains an expression")
                    .build())
            },
            &SecretCommitmentPolicy::Reject,
        )
    }
}

macro_rules! canonical_tree {
    ($expression:ty, $source:expr) => {
        impl ValueTree<$expression> {
            /// Encode the tree independently of object insertion order.
            ///
            /// Numeric spellings normalize; expressions retain their authored syntax
            /// and exact source. This is content-addressing, not the serde wire form.
            ///
            /// # Errors
            /// Rejects secrets without a keyed commitment and over-deep trees.
            pub fn canonical_bytes(&self) -> Result<Vec<u8>, ValidationError> {
                self.canonical_with(&$source, &SecretCommitmentPolicy::Reject)
            }

            /// Encode with process-scoped, keyed secret commitments.
            ///
            /// # Errors
            /// Rejects trees deeper than the value depth limit.
            pub fn canonical_bytes_committing(
                &self,
                key: &CommitmentKey,
            ) -> Result<Vec<u8>, ValidationError> {
                self.canonical_with(&$source, &SecretCommitmentPolicy::Keyed(key))
            }

            /// Compute a stable content identifier for a secret-free tree.
            ///
            /// # Errors
            /// Propagates canonical encoding failures.
            pub fn content_id(&self) -> Result<ContentId, ValidationError> {
                self.canonical_bytes()
                    .map(|bytes| ContentId::from_digest(*blake3::hash(&bytes).as_bytes()))
            }

            /// Compute a process-scoped identifier under an explicit commitment key.
            ///
            /// # Errors
            /// Propagates canonical encoding failures.
            pub fn content_id_committing(
                &self,
                key: &CommitmentKey,
            ) -> Result<CommitmentId, ValidationError> {
                self.canonical_bytes_committing(key)
                    .map(|bytes| CommitmentId::from_digest(*blake3::hash(&bytes).as_bytes()))
            }
        }
    };
}

canonical_tree!(Expression, |expression: &Expression| Ok((
    expression.syntax(),
    expression.source()
)));
canonical_tree!(Infallible, |&never: &Infallible| match never {});
