use domain_key::{define_domain, key_type};
pub use domain_key::KeyParseError;


define_domain!(PrameterDomain, "parameter");
key_type!(ParameterKey, PrameterDomain);

