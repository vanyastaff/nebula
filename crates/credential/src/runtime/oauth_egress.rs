//! Shared OAuth server-egress policy primitives.
//!
//! This module owns the protocol-independent part of the OAuth network
//! boundary. Composition roots still own their concrete HTTP clients and DNS
//! resolvers, but every first-party resolver feeds its answers through
//! [`validate_oauth_dns_answers`] and returns those exact socket addresses to
//! its connector.

use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
};

use url::{Host, Url};

/// Maximum UTF-8 byte length admitted for one OAuth server endpoint.
///
/// Eight KiB leaves ample room for provider routing parameters while bounding
/// parser work and the sensitive URL value retained by the validated type.
pub const OAUTH_ENDPOINT_MAX_BYTES: usize = 8 * 1024;

/// Maximum number of addresses accepted from one OAuth endpoint DNS lookup.
///
/// The resolver takes at most one additional answer so an oversized response
/// is rejected rather than silently truncated into a different address set.
pub const OAUTH_DNS_MAX_ANSWERS: usize = 32;

/// An HTTPS OAuth endpoint whose literal-host and URL-shape policy has already
/// been validated.
///
/// Domain names still require connect-time DNS validation by the concrete
/// transport. Keeping the parsed URL attached to this type prevents validating
/// one string and sending another.
#[derive(Clone, PartialEq, Eq)]
pub struct OAuthServerEndpoint(Url);

impl OAuthServerEndpoint {
    /// Parse a server-fetched OAuth endpoint.
    ///
    /// Userinfo, fragments, plaintext HTTP, localhost names, and non-global IP
    /// literals are rejected with closed, input-free errors. Provider-required
    /// query parameters are preserved exactly after URL normalization; the
    /// type's `Debug` and every transport error remain fully redacted so query
    /// values cannot become diagnostics.
    pub fn parse(raw: &str) -> Result<Self, OAuthEndpointError> {
        if raw.len() > OAUTH_ENDPOINT_MAX_BYTES {
            return Err(OAuthEndpointError::EndpointTooLong);
        }
        let url = Url::parse(raw).map_err(|_| OAuthEndpointError::InvalidUrl)?;
        if url.scheme() != "https" {
            return Err(OAuthEndpointError::HttpsRequired);
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(OAuthEndpointError::UserinfoForbidden);
        }
        if url.fragment().is_some() {
            return Err(OAuthEndpointError::FragmentForbidden);
        }
        if url.port() == Some(0) {
            return Err(OAuthEndpointError::InvalidPort);
        }
        validate_host(url.host())?;
        Ok(Self(url))
    }

    /// Explicitly expose the already-validated URL to a concrete HTTP
    /// transport.
    ///
    /// The full URL is sensitive because admitted provider-routing query
    /// parameters can carry tenant or credential-adjacent values. Callers
    /// must pass it directly to the HTTP client and never format or log it.
    /// The `expose_` name keeps every plaintext sink grep-able.
    #[must_use]
    pub fn expose_url(&self) -> &Url {
        &self.0
    }
}

impl fmt::Debug for OAuthServerEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OAuthServerEndpoint(<redacted>)")
    }
}

/// Closed endpoint-validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum OAuthEndpointError {
    /// The value is not an absolute URL.
    #[error("OAuth endpoint URL is invalid")]
    InvalidUrl,
    /// The raw URL exceeds the fixed parser and retention bound.
    #[error("OAuth endpoint URL exceeds the fixed length limit")]
    EndpointTooLong,
    /// Server-side OAuth traffic must use TLS.
    #[error("OAuth endpoint must use HTTPS")]
    HttpsRequired,
    /// Embedded URL credentials are forbidden.
    #[error("OAuth endpoint must not contain userinfo")]
    UserinfoForbidden,
    /// URL fragments are client-local and never belong on a server request.
    #[error("OAuth endpoint must not contain a fragment")]
    FragmentForbidden,
    /// An explicit port must be in the usable TCP range.
    #[error("OAuth endpoint port is invalid")]
    InvalidPort,
    /// The URL has no host.
    #[error("OAuth endpoint must include a host")]
    MissingHost,
    /// A literal or reserved localhost host is not globally routable.
    #[error("OAuth endpoint host is not globally routable")]
    NonGlobalHost,
}

/// Closed connect-time DNS-policy failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum OAuthDnsAnswerError {
    /// The resolver returned no usable address.
    #[error("OAuth DNS lookup returned no addresses")]
    Empty,
    /// The resolver returned more addresses than the fixed policy permits.
    #[error("OAuth DNS lookup returned too many addresses")]
    TooMany,
    /// At least one answer is special-use or otherwise non-global.
    ///
    /// Mixed public/private sets fail as a whole; the address itself is
    /// deliberately absent from diagnostics.
    #[error("OAuth DNS lookup returned a non-global address")]
    NonGlobal,
}

/// Validate all DNS answers and produce the exact addresses a connector must
/// use.
///
/// The all-or-nothing policy rejects empty, oversized, private, loopback,
/// link-local, documentation, multicast, ULA, transition, and mixed answer
/// sets. Returned ports are zero; reqwest replaces them with the URL's
/// explicit or scheme-default port.
pub fn validate_oauth_dns_answers(
    answers: Vec<IpAddr>,
) -> Result<Vec<SocketAddr>, OAuthDnsAnswerError> {
    if answers.is_empty() {
        return Err(OAuthDnsAnswerError::Empty);
    }
    if answers.len() > OAUTH_DNS_MAX_ANSWERS {
        return Err(OAuthDnsAnswerError::TooMany);
    }
    if answers
        .iter()
        .any(|answer| !oauth_egress_ip_is_globally_routable(*answer))
    {
        return Err(OAuthDnsAnswerError::NonGlobal);
    }

    Ok(answers
        .into_iter()
        .map(|answer| SocketAddr::new(answer, 0))
        .collect())
}

/// Return whether an address is admitted for first-party OAuth egress.
///
/// This conservative classifier follows the IANA IPv4 and IPv6
/// special-purpose registries. It intentionally rejects address families and
/// transition ranges whose global reachability is ambiguous.
#[must_use]
pub fn oauth_egress_ip_is_globally_routable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ipv4_is_globally_routable(ip),
        IpAddr::V6(ip) => ipv6_is_globally_routable(ip),
    }
}

fn validate_host(host: Option<Host<&str>>) -> Result<(), OAuthEndpointError> {
    match host.ok_or(OAuthEndpointError::MissingHost)? {
        Host::Domain(host) if is_localhost_name(host) => Err(OAuthEndpointError::NonGlobalHost),
        Host::Domain(_) => Ok(()),
        Host::Ipv4(ip) if oauth_egress_ip_is_globally_routable(IpAddr::V4(ip)) => Ok(()),
        Host::Ipv6(ip) if oauth_egress_ip_is_globally_routable(IpAddr::V6(ip)) => Ok(()),
        Host::Ipv4(_) | Host::Ipv6(_) => Err(OAuthEndpointError::NonGlobalHost),
    }
}

fn is_localhost_name(host: &str) -> bool {
    let normalized = host.trim_end_matches('.');
    normalized.eq_ignore_ascii_case("localhost")
        || normalized.to_ascii_lowercase().ends_with(".localhost")
}

fn ipv4_is_globally_routable(ip: Ipv4Addr) -> bool {
    const SPECIAL: &[(Ipv4Addr, u8)] = &[
        (Ipv4Addr::UNSPECIFIED, 8),
        (Ipv4Addr::new(10, 0, 0, 0), 8),
        (Ipv4Addr::new(100, 64, 0, 0), 10),
        (Ipv4Addr::new(127, 0, 0, 0), 8),
        (Ipv4Addr::new(169, 254, 0, 0), 16),
        (Ipv4Addr::new(172, 16, 0, 0), 12),
        (Ipv4Addr::new(192, 0, 0, 0), 24),
        (Ipv4Addr::new(192, 0, 2, 0), 24),
        (Ipv4Addr::new(192, 88, 99, 0), 24),
        (Ipv4Addr::new(192, 168, 0, 0), 16),
        (Ipv4Addr::new(198, 18, 0, 0), 15),
        (Ipv4Addr::new(198, 51, 100, 0), 24),
        (Ipv4Addr::new(203, 0, 113, 0), 24),
        (Ipv4Addr::new(224, 0, 0, 0), 4),
        (Ipv4Addr::new(240, 0, 0, 0), 4),
    ];

    !SPECIAL
        .iter()
        .any(|(network, prefix)| ipv4_in_prefix(ip, *network, *prefix))
}

fn ipv6_is_globally_routable(ip: Ipv6Addr) -> bool {
    if ip.to_ipv4_mapped().is_some() {
        return false;
    }

    const SPECIAL_WITHIN_GLOBAL_UNICAST: &[(Ipv6Addr, u8)] = &[
        (Ipv6Addr::new(0x2001, 0, 0, 0, 0, 0, 0, 0), 23),
        (Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 0), 32),
        (Ipv6Addr::new(0x2002, 0, 0, 0, 0, 0, 0, 0), 16),
        (Ipv6Addr::new(0x3fff, 0, 0, 0, 0, 0, 0, 0), 20),
    ];

    ipv6_in_prefix(ip, Ipv6Addr::new(0x2000, 0, 0, 0, 0, 0, 0, 0), 3)
        && !SPECIAL_WITHIN_GLOBAL_UNICAST
            .iter()
            .any(|(network, prefix)| ipv6_in_prefix(ip, *network, *prefix))
}

fn ipv4_in_prefix(ip: Ipv4Addr, network: Ipv4Addr, prefix: u8) -> bool {
    let mask = u32::MAX.checked_shl(u32::from(32 - prefix)).unwrap_or(0);
    u32::from(ip) & mask == u32::from(network) & mask
}

fn ipv6_in_prefix(ip: Ipv6Addr, network: Ipv6Addr, prefix: u8) -> bool {
    let mask = u128::MAX.checked_shl(u32::from(128 - prefix)).unwrap_or(0);
    u128::from(ip) & mask == u128::from(network) & mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_rejects_unsafe_shapes_and_special_use_literals() {
        for raw in [
            "",
            "http://provider.example/token",
            "https://user:secret@provider.example/token",
            "https://provider.example/token#fragment",
            "https://provider.example:0/token",
            "https://localhost/token",
            "https://api.localhost./token",
            "https://10.0.0.1/token",
            "https://169.254.169.254/token",
            "https://192.0.2.1/token",
            "https://[::1]/token",
            "https://[fc00::1]/token",
            "https://[fe80::1]/token",
            "https://[::ffff:7f00:1]/token",
        ] {
            assert!(
                OAuthServerEndpoint::parse(raw).is_err(),
                "unsafe endpoint accepted: {raw}"
            );
        }

        for raw in [
            "https://provider.example/token",
            "https://provider.example:1/token",
            "https://provider.example:65535/token",
            "https://1.1.1.1/token",
            "https://[2606:4700:4700::1111]/token",
        ] {
            assert!(
                OAuthServerEndpoint::parse(raw).is_ok(),
                "global endpoint rejected: {raw}"
            );
        }
    }

    #[test]
    fn endpoint_length_bound_is_exact_and_input_free() {
        const PREFIX: &str = "https://provider.example/";
        let boundary = format!(
            "{PREFIX}{}",
            "a".repeat(OAUTH_ENDPOINT_MAX_BYTES - PREFIX.len())
        );
        assert_eq!(boundary.len(), OAUTH_ENDPOINT_MAX_BYTES);
        assert!(
            OAuthServerEndpoint::parse(&boundary).is_ok(),
            "endpoint at the fixed byte boundary must be accepted"
        );

        let oversized = format!("{boundary}diagnostic-canary");
        let error = OAuthServerEndpoint::parse(&oversized)
            .expect_err("oversized endpoint must fail before URL parsing");
        assert_eq!(error, OAuthEndpointError::EndpointTooLong);
        let diagnostic = format!("{error:?} {error}");
        assert!(!diagnostic.contains("diagnostic-canary"));
        assert_eq!(
            error.to_string(),
            "OAuth endpoint URL exceeds the fixed length limit"
        );
    }

    #[test]
    fn endpoint_debug_is_constant_and_redacted() {
        let first =
            OAuthServerEndpoint::parse("https://one.example/token?tenant=alpha").expect("valid");
        let second = OAuthServerEndpoint::parse(
            "https://two.example/long/path?client_secret=diagnostic-canary",
        )
        .expect("valid");

        assert_eq!(format!("{first:?}"), format!("{second:?}"));
        assert!(!format!("{second:?}").contains("diagnostic-canary"));
    }

    #[test]
    fn classifier_rejects_special_use_and_accepts_public_controls() {
        for raw in [
            "0.0.0.0",
            "10.1.2.3",
            "100.100.100.200",
            "127.0.0.1",
            "169.254.1.1",
            "172.16.0.1",
            "192.0.2.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "::ffff:10.0.0.1",
            "64:ff9b::10.0.0.1",
            "100::1",
            "2001:db8::1",
            "2001:2::1",
            "2002:a00:1::",
            "3fff::1",
            "fc00::1",
            "fe80::1",
            "ff02::1",
        ] {
            let ip: IpAddr = raw.parse().expect("valid table IP");
            assert!(
                !oauth_egress_ip_is_globally_routable(ip),
                "special-use IP accepted: {raw}"
            );
        }

        for raw in [
            "1.1.1.1",
            "8.8.8.8",
            "9.9.9.9",
            "2001:4860:4860::8888",
            "2606:4700:4700::1111",
        ] {
            let ip: IpAddr = raw.parse().expect("valid public IP");
            assert!(
                oauth_egress_ip_is_globally_routable(ip),
                "public control rejected: {raw}"
            );
        }
    }

    #[test]
    fn dns_answers_are_all_or_nothing_and_preserve_exact_addresses() {
        let public = vec![
            IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
            "2606:4700:4700::1111".parse().expect("valid IPv6"),
        ];
        let validated =
            validate_oauth_dns_answers(public.clone()).expect("public answers accepted");
        assert_eq!(
            validated,
            public
                .into_iter()
                .map(|ip| SocketAddr::new(ip, 0))
                .collect::<Vec<_>>()
        );

        assert_eq!(
            validate_oauth_dns_answers(Vec::new()),
            Err(OAuthDnsAnswerError::Empty)
        );
        assert_eq!(
            validate_oauth_dns_answers(vec![
                IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
                IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            ]),
            Err(OAuthDnsAnswerError::NonGlobal)
        );
        assert_eq!(
            validate_oauth_dns_answers(vec![
                IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1));
                OAUTH_DNS_MAX_ANSWERS + 1
            ]),
            Err(OAuthDnsAnswerError::TooMany)
        );
    }
}
