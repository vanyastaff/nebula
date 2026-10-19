//! Rate Limiting Middleware
//!
//! Global per-IP rate limiting for the Axum HTTP stack using the GCRA algorithm
//! (via the [`governor`] crate).
//!
//! `/health` and `/ready` paths are always excluded so that Kubernetes liveness
//! and readiness probes are never throttled even under load.
//!
//! The key is the TCP peer address. Forwarding headers are believed only when
//! that peer is a configured trusted proxy (`API_TRUSTED_PROXIES`); see
//! [`RateLimitState::with_trusted_proxies`].
//!
//! # Examples
//!
//! ```
//! use nebula_api::middleware::rate_limit::RateLimitState;
//!
//! // 100 requests per second per IP
//! let state = RateLimitState::new(100);
//! ```

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    num::NonZeroU32,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use axum::{
    extract::{ConnectInfo, Request},
    http::{HeaderValue, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use governor::{DefaultKeyedRateLimiter, Quota};

/// Paths that bypass rate limiting entirely.
///
/// Health and readiness probes must always succeed — orchestrators depend on
/// them independent of any traffic volume.
pub(crate) const EXCLUDED_PATHS: &[&str] = &["/health", "/ready"];

/// Shared, per-IP rate-limiter state.
///
/// Internally backed by a lock-free [`DashMap`](dashmap::DashMap) so it can be
/// cheaply cloned and shared across async tasks.
///
/// # Examples
///
/// ```
/// use nebula_api::middleware::rate_limit::RateLimitState;
///
/// let state = RateLimitState::new(100); // 100 req/s per IP
/// assert_eq!(state.requests_per_second(), 100);
/// ```
#[derive(Clone)]
pub struct RateLimitState {
    pub(crate) limiter: Arc<DefaultKeyedRateLimiter<IpAddr>>,
    requests_per_second: u32,
    trusted_proxies: Arc<[TrustedProxy]>,
    checks: Arc<AtomicU64>,
}

/// How many checks pass between sweeps of idle per-client state. Each sweep
/// is linear in the number of tracked clients, so this keeps the amortized
/// cost per request constant while bounding the map to recently seen keys.
const SWEEP_EVERY: u64 = 4096;

impl RateLimitState {
    /// Create a rate limiter that allows `requests_per_second` requests per IP.
    ///
    /// Values below 1 are clamped to 1.
    #[must_use]
    pub fn new(requests_per_second: u32) -> Self {
        let rps = NonZeroU32::new(requests_per_second.max(1)).unwrap_or(NonZeroU32::MIN);
        let quota = Quota::per_second(rps);
        Self {
            limiter: Arc::new(DefaultKeyedRateLimiter::keyed(quota)),
            requests_per_second,
            trusted_proxies: Arc::from([]),
            checks: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Believe `X-Forwarded-For` / `X-Real-IP` from these peers only.
    ///
    /// With no trusted proxies (the default) the key is always the TCP peer,
    /// so a client cannot pick its own bucket by sending a header. Behind a
    /// trusted proxy the key is the rightmost `X-Forwarded-For` hop that is
    /// not itself a trusted proxy: hops to its left were written by the
    /// client and prove nothing.
    #[must_use]
    pub fn with_trusted_proxies(mut self, proxies: Vec<TrustedProxy>) -> Self {
        self.trusted_proxies = Arc::from(proxies);
        self
    }

    /// Returns the configured rate per second.
    #[must_use]
    pub fn requests_per_second(&self) -> u32 {
        self.requests_per_second
    }

    /// Run the rate-limiting logic for a single request.
    ///
    /// Intended for use inside `axum::middleware::from_fn`:
    ///
    /// ```rust
    /// use axum::{Router, middleware, routing::get};
    /// use nebula_api::middleware::rate_limit::RateLimitState;
    ///
    /// let rl = RateLimitState::new(100);
    /// let app: Router = Router::new()
    ///     .route("/", get(|| async { "ok" }))
    ///     .layer(middleware::from_fn(move |req, next| {
    ///         let rl = rl.clone();
    ///         async move { rl.handle(req, next).await }
    ///     }));
    /// # let _ = app;
    /// ```
    pub async fn handle(&self, request: Request, next: Next) -> Response {
        // Bypass excluded paths (health / readiness probes)
        if EXCLUDED_PATHS.contains(&request.uri().path()) {
            return next.run(request).await;
        }

        let ip = self.client_ip(&request);
        if self.checks.fetch_add(1, Ordering::Relaxed) % SWEEP_EVERY == SWEEP_EVERY - 1 {
            self.limiter.retain_recent();
        }

        if self.limiter.check_key(&ip) == Ok(()) {
            next.run(request).await
        } else {
            let mut response = crate::error::ApiError::RateLimitExceeded.into_response();
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
            response
        }
    }

    /// The address this request is limited under.
    ///
    /// 1. No [`ConnectInfo<SocketAddr>`] (only in-process tests serve without it): loopback.
    /// 2. A peer that is not a trusted proxy: the peer itself.
    /// 3. A trusted proxy: the rightmost untrusted `X-Forwarded-For` hop, else its `X-Real-IP`,
    ///    else the proxy's own address. An unparsable hop ends the walk at the proxy, since
    ///    nothing to its left can be attributed.
    fn client_ip(&self, request: &Request) -> IpAddr {
        let Some(peer) = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|connect_info| connect_info.0.ip())
        else {
            return IpAddr::V4(Ipv4Addr::LOCALHOST);
        };
        if !self.is_trusted(peer) {
            return peer;
        }
        let headers = request.headers();
        let mut forwarded = headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(','))
            .collect::<Vec<_>>();
        if !forwarded.is_empty() {
            while let Some(hop) = forwarded.pop() {
                match hop.trim().parse::<IpAddr>() {
                    Ok(ip) if self.is_trusted(ip) => {},
                    Ok(ip) => return ip,
                    Err(_) => return peer,
                }
            }
            return peer;
        }
        headers
            .get("x-real-ip")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<IpAddr>().ok())
            .unwrap_or(peer)
    }

    fn is_trusted(&self, ip: IpAddr) -> bool {
        self.trusted_proxies.iter().any(|proxy| proxy.contains(ip))
    }
}

/// One `API_TRUSTED_PROXIES` entry: an address or a CIDR block.
///
/// ```
/// use nebula_api::middleware::rate_limit::TrustedProxy;
///
/// let block: TrustedProxy = "10.0.0.0/8".parse().unwrap();
/// assert!(block.contains("10.1.2.3".parse().unwrap()));
/// assert!(!block.contains("11.0.0.1".parse().unwrap()));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct TrustedProxy {
    network: IpAddr,
    prefix: u8,
}

/// An `API_TRUSTED_PROXIES` entry that is neither an address nor a CIDR block.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not an IP address or CIDR block: {0:?}")]
pub struct InvalidTrustedProxy(String);

impl TrustedProxy {
    /// Whether `ip` falls inside this block. An IPv4 block also matches the
    /// IPv4-mapped IPv6 form of its addresses, which dual-stack listeners report.
    #[must_use]
    pub fn contains(&self, ip: IpAddr) -> bool {
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
            IpAddr::V4(_) => ip,
        };
        match (self.network, ip) {
            (IpAddr::V4(network), IpAddr::V4(ip)) => prefix_matches(
                u128::from(network.to_bits()),
                u128::from(ip.to_bits()),
                self.prefix,
                32,
            ),
            (IpAddr::V6(network), IpAddr::V6(ip)) => {
                prefix_matches(network.to_bits(), ip.to_bits(), self.prefix, 128)
            },
            _ => false,
        }
    }
}

fn prefix_matches(network: u128, ip: u128, prefix: u8, width: u8) -> bool {
    let host_bits = u32::from(width - prefix);
    let mask = if host_bits >= 128 {
        0
    } else {
        u128::MAX << host_bits
    };
    let width_mask = if width == 128 {
        u128::MAX
    } else {
        (1u128 << width) - 1
    };
    (network ^ ip) & mask & width_mask == 0
}

impl std::str::FromStr for TrustedProxy {
    type Err = InvalidTrustedProxy;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidTrustedProxy(raw.to_owned());
        let (address, prefix) = match raw.split_once('/') {
            Some((address, prefix)) => (address, Some(prefix)),
            None => (raw, None),
        };
        let network: IpAddr = address.trim().parse().map_err(|_| invalid())?;
        let width = if network.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(prefix) => prefix.trim().parse::<u8>().map_err(|_| invalid())?,
            None => width,
        };
        if prefix > width {
            return Err(invalid());
        }
        Ok(Self { network, prefix })
    }
}

impl TryFrom<String> for TrustedProxy {
    type Error = InvalidTrustedProxy;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        raw.parse()
    }
}

impl From<TrustedProxy> for String {
    fn from(proxy: TrustedProxy) -> Self {
        format!("{}/{}", proxy.network, proxy.prefix)
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;

    use super::*;

    fn proxies(entries: &[&str]) -> RateLimitState {
        RateLimitState::new(1).with_trusted_proxies(
            entries
                .iter()
                .map(|entry| entry.parse().expect("valid entry"))
                .collect(),
        )
    }

    fn request(peer: [u8; 4], headers: &[(&str, &str)]) -> Request {
        let mut builder = axum::http::Request::builder().uri("/api");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let mut request = builder.body(Body::empty()).expect("request");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from((peer, 4000))));
        request
    }

    fn ip(raw: &str) -> IpAddr {
        raw.parse().expect("ip")
    }

    #[test]
    fn an_untrusted_peer_cannot_choose_its_bucket() {
        let state = proxies(&[]);
        let request = request(
            [203, 0, 113, 7],
            &[
                ("x-forwarded-for", "198.51.100.1"),
                ("x-real-ip", "198.51.100.2"),
            ],
        );
        assert_eq!(state.client_ip(&request), ip("203.0.113.7"));
    }

    #[test]
    fn a_trusted_proxy_names_the_rightmost_untrusted_hop() {
        let state = proxies(&["10.0.0.0/8"]);
        // The client prepended a forged hop; the proxies appended the real one.
        let request = request(
            [10, 0, 0, 5],
            &[("x-forwarded-for", "1.1.1.1, 198.51.100.9, 10.0.0.4")],
        );
        assert_eq!(state.client_ip(&request), ip("198.51.100.9"));
    }

    #[test]
    fn forwarded_hops_span_repeated_headers() {
        let state = proxies(&["10.0.0.5"]);
        let mut request = request([10, 0, 0, 5], &[("x-forwarded-for", "1.1.1.1")]);
        request
            .headers_mut()
            .append("x-forwarded-for", HeaderValue::from_static("198.51.100.3"));
        assert_eq!(state.client_ip(&request), ip("198.51.100.3"));
    }

    #[test]
    fn an_unparsable_hop_falls_back_to_the_proxy() {
        let state = proxies(&["10.0.0.5"]);
        let request = request([10, 0, 0, 5], &[("x-forwarded-for", "1.1.1.1, unknown")]);
        assert_eq!(state.client_ip(&request), ip("10.0.0.5"));
    }

    #[test]
    fn a_trusted_proxy_without_forwarded_for_may_name_x_real_ip() {
        let state = proxies(&["10.0.0.5"]);
        let request = request([10, 0, 0, 5], &[("x-real-ip", "198.51.100.4")]);
        assert_eq!(state.client_ip(&request), ip("198.51.100.4"));
    }

    #[test]
    fn trusted_proxy_entries_parse_and_match() {
        let v4: TrustedProxy = "192.168.0.0/16".parse().expect("v4 block");
        assert!(v4.contains(ip("192.168.255.1")));
        assert!(v4.contains(ip("::ffff:192.168.1.1")));
        assert!(!v4.contains(ip("192.169.0.1")));
        let all: TrustedProxy = "0.0.0.0/0".parse().expect("v4 any");
        assert!(all.contains(ip("8.8.8.8")));
        assert!(!all.contains(ip("2001:db8::1")));
        let v6: TrustedProxy = "2001:db8::/32".parse().expect("v6 block");
        assert!(v6.contains(ip("2001:db8:1::1")));
        assert!(!v6.contains(ip("2001:db9::1")));
        let single: TrustedProxy = "::1".parse().expect("v6 address");
        assert!(single.contains(ip("::1")));
        assert!(!single.contains(ip("::2")));
        for invalid in ["", "10.0.0.0/33", "::/129", "10.0.0.0/x", "host.example"] {
            assert!(invalid.parse::<TrustedProxy>().is_err(), "{invalid:?}");
        }
        assert_eq!(String::from(v4), "192.168.0.0/16");
    }
}
