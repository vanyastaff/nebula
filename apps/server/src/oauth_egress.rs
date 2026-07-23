//! Fixed network policy for first-party credential OAuth egress.
//!
//! Domain policy (URL shape and global-address classification) lives in
//! `nebula-credential`. This composition-root module binds that policy into
//! reqwest's connect-time resolver so the addresses that pass validation are
//! the exact addresses used for the connection.

#[cfg(test)]
use std::net::SocketAddr;
use std::{future::Future, net::IpAddr, pin::Pin, sync::Arc, time::Duration};

use nebula_credential::runtime::{OAUTH_DNS_MAX_ANSWERS, validate_oauth_dns_answers};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};

const OAUTH_TOKEN_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const OAUTH_TOKEN_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

type LookupFuture = Pin<Box<dyn Future<Output = Result<Vec<IpAddr>, LookupError>> + Send>>;

#[derive(Debug, Clone, Copy)]
struct LookupError;

impl std::fmt::Display for LookupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OAuth DNS lookup failed")
    }
}

impl std::error::Error for LookupError {}

trait HostLookup: Send + Sync {
    fn lookup(&self, host: String) -> LookupFuture;
}

struct SystemLookup;

impl HostLookup for SystemLookup {
    fn lookup(&self, host: String) -> LookupFuture {
        Box::pin(async move {
            tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map(|answers| {
                    answers
                        .take(OAUTH_DNS_MAX_ANSWERS.saturating_add(1))
                        .map(|answer| answer.ip())
                        .collect()
                })
                .map_err(|_| LookupError)
        })
    }
}

#[cfg(test)]
struct StaticTestLookup {
    answers: Vec<IpAddr>,
}

#[cfg(test)]
impl HostLookup for StaticTestLookup {
    fn lookup(&self, _host: String) -> LookupFuture {
        Box::pin(std::future::ready(Ok(self.answers.clone())))
    }
}

#[derive(Clone)]
struct GuardedResolver {
    lookup: Arc<dyn HostLookup>,
    #[cfg(test)]
    connect_override: Option<IpAddr>,
}

impl GuardedResolver {
    fn system() -> Self {
        Self {
            lookup: Arc::new(SystemLookup),
            #[cfg(test)]
            connect_override: None,
        }
    }

    #[cfg(test)]
    fn for_test(dns_answers: Vec<IpAddr>, connect_override: IpAddr) -> Self {
        Self {
            lookup: Arc::new(StaticTestLookup {
                answers: dns_answers,
            }),
            connect_override: Some(connect_override),
        }
    }
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let lookup = Arc::clone(&self.lookup);
        let host = name.as_str().to_owned();
        #[cfg(test)]
        let connect_override = self.connect_override;

        Box::pin(async move {
            let answers = lookup.lookup(host).await?;
            let validated = validate_oauth_dns_answers(answers).map_err(|_| LookupError)?;
            #[cfg(test)]
            let validated = match connect_override {
                Some(fixture_ip) => validated
                    .into_iter()
                    .map(|answer| SocketAddr::new(fixture_ip, answer.port()))
                    .collect(),
                None => validated,
            };
            Ok(Box::new(validated.into_iter()) as Addrs)
        })
    }
}

fn fixed_client_builder(resolver: GuardedResolver) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .tls_backend_rustls()
        .connect_timeout(OAUTH_TOKEN_HTTP_CONNECT_TIMEOUT)
        .timeout(OAUTH_TOKEN_HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .https_only(true)
        .referer(false)
        .connection_verbose(false)
        .no_proxy()
        .retry(reqwest::retry::never())
        .no_hickory_dns()
        .dns_resolver(resolver)
}

pub(crate) fn build_oauth_client() -> Result<reqwest::Client, reqwest::Error> {
    fixed_client_builder(GuardedResolver::system()).build()
}

#[cfg(test)]
pub(crate) fn build_test_oauth_client(
    trust_anchor: reqwest::Certificate,
    connect_ip: IpAddr,
    dns_answers: Vec<IpAddr>,
) -> Result<reqwest::Client, reqwest::Error> {
    fixed_client_builder(GuardedResolver::for_test(dns_answers, connect_ip))
        .tls_certs_only([trust_anchor])
        .build()
}

#[cfg(test)]
mod tests {
    use std::{
        net::{Ipv4Addr, Ipv6Addr},
        str::FromStr,
    };

    use super::*;

    async fn resolve_with(answers: Vec<IpAddr>) -> Result<Vec<SocketAddr>, String> {
        let resolver = GuardedResolver {
            lookup: Arc::new(StaticTestLookup { answers }),
            connect_override: None,
        };
        let name = Name::from_str("oauth.example.com").expect("valid DNS name");
        resolver
            .resolve(name)
            .await
            .map(Iterator::collect)
            .map_err(|error| error.to_string())
    }

    #[tokio::test]
    async fn resolver_rejects_private_mixed_empty_and_excessive_answers() {
        assert!(
            resolve_with(vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))])
                .await
                .is_err()
        );
        assert!(
            resolve_with(vec![
                IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
                IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            ])
            .await
            .is_err()
        );
        assert!(resolve_with(Vec::new()).await.is_err());
        assert!(
            resolve_with(vec![
                IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1));
                OAUTH_DNS_MAX_ANSWERS + 1
            ])
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn resolver_returns_only_the_exact_validated_connect_addresses() {
        let answers = vec![
            IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
            IpAddr::V6(Ipv6Addr::new(0x2606, 0x4700, 0x4700, 0, 0, 0, 0, 0x1111)),
        ];
        let resolved = resolve_with(answers.clone())
            .await
            .expect("public answers accepted");
        assert_eq!(
            resolved,
            answers
                .into_iter()
                .map(|answer| SocketAddr::new(answer, 0))
                .collect::<Vec<_>>()
        );
    }
}
