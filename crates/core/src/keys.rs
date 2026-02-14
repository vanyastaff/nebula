pub use domain_key::KeyParseError;
use domain_key::{define_domain, key_type};

define_domain!(PrameterDomain, "parameter");
key_type!(ParameterKey, PrameterDomain);

define_domain!(CredentialDomain, "credential");
key_type!(CredentialKey, CredentialDomain);
