//! Operator-facing topology settings.
//!
//! Registration JSON configures two separate things: the resource's own
//! [`Provider::Config`](crate::Provider::Config) (how to connect) and the
//! topology (how much capacity to hold). This module is the wire format for
//! the second one. Every field is optional, so `{}` or an absent value means
//! the built-in defaults. Durations are integer milliseconds with an explicit
//! `_ms` suffix, unknown fields are rejected, and every value is validated
//! through the topology's fallible constructor — operator input never reaches
//! a panicking path.
//!
//! [`ConfigurableTopology`] maps a settings value to a built topology. The
//! built-in [`Pooled`], [`Resident`] and [`Bounded`] implement it; a custom
//! topology may implement it to accept operator settings the same way.

use std::{num::NonZeroUsize, time::Duration};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    error::Error,
    resource::Provider,
    topology::{
        Bounded, BoundedProvider, PoolProvider, Pooled, Resident, ResidentProvider, Topology,
        pooled::config::{Config as PoolConfig, WarmupStrategy},
        resident::config::Config as ResidentConfig,
        store::PoolStrategy,
    },
};

/// A topology that can be built from operator-supplied settings.
pub trait ConfigurableTopology<R: Provider>: Topology<R> + Sized {
    /// Wire format of this topology's settings.
    type Settings: DeserializeOwned;

    /// Builds the topology from parsed settings (`None` = defaults).
    ///
    /// `fingerprint` is the resource config's
    /// [`fingerprint`](crate::ResourceConfig::fingerprint), used by
    /// topologies that evict instances built against an older config.
    ///
    /// # Errors
    ///
    /// Returns a permanent [`Error`] when the settings describe a topology
    /// that cannot work (a zero pool, a zero timeout, a missing mode).
    fn from_settings(settings: Option<Self::Settings>, fingerprint: u64) -> Result<Self, Error>;

    /// Parses raw registration JSON (`None` or `null` = defaults), then
    /// builds the topology through [`from_settings`](Self::from_settings).
    ///
    /// # Errors
    ///
    /// Returns a permanent [`Error`] for malformed or unknown fields and for
    /// every error [`from_settings`](Self::from_settings) returns.
    fn from_settings_value(
        value: Option<&serde_json::Value>,
        fingerprint: u64,
    ) -> Result<Self, Error> {
        let settings = match value {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => Some(Self::Settings::deserialize(value).map_err(|error| {
                Error::permanent(format!("invalid topology settings: {error}"))
            })?),
        };
        Self::from_settings(settings, fingerprint)
    }
}

/// Idle-queue order for [`PoolSettings::strategy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PoolStrategySetting {
    /// Reuse the most recently returned instance.
    Lifo,
    /// Rotate through every instance.
    Fifo,
}

/// Startup warmup for [`PoolSettings::warmup`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum WarmupSetting {
    /// Create instances on demand.
    None,
    /// Create `min_size` instances one at a time.
    Sequential,
    /// Create `min_size` instances concurrently.
    Parallel,
    /// Create instances with a fixed delay between them.
    Staggered {
        /// Delay between successive creations, in milliseconds.
        interval_ms: u64,
    },
}

/// Operator settings for a [`Pooled`] topology. Absent fields keep the
/// [`PoolConfig`] defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct PoolSettings {
    /// Warmup target and minimum idle instances.
    pub min_size: Option<u32>,
    /// Hard cap on instances (idle and checked out); must be at least 1.
    pub max_size: Option<u32>,
    /// Idle eviction threshold in milliseconds; `0` disables idle eviction.
    pub idle_timeout_ms: Option<u64>,
    /// Maximum instance lifetime in milliseconds; `0` disables it.
    pub max_lifetime_ms: Option<u64>,
    /// Budget for one `Provider::create`, in milliseconds; must be positive.
    pub create_timeout_ms: Option<u64>,
    /// Idle-queue order.
    pub strategy: Option<PoolStrategySetting>,
    /// Startup warmup.
    pub warmup: Option<WarmupSetting>,
    /// Run `Provider::check` on every checkout.
    pub test_on_checkout: Option<bool>,
    /// Background maintenance interval in milliseconds; must be positive.
    pub maintenance_interval_ms: Option<u64>,
    /// Cap on concurrent `Provider::create` calls; must be at least 1.
    pub max_concurrent_creates: Option<u32>,
}

impl PoolSettings {
    /// Resolves the settings against [`PoolConfig`] defaults.
    ///
    /// # Errors
    ///
    /// Returns a permanent [`Error`] for a zero `create_timeout_ms`,
    /// `maintenance_interval_ms` or `max_concurrent_creates`. Size invariants
    /// are checked by [`Pooled::try_new`].
    pub fn into_config(self) -> Result<PoolConfig, Error> {
        let mut config = PoolConfig::default();
        if let Some(min_size) = self.min_size {
            config.min_size = min_size;
        }
        if let Some(max_size) = self.max_size {
            config.max_size = max_size;
        }
        if let Some(ms) = self.idle_timeout_ms {
            config.idle_timeout = optional_millis(ms);
        }
        if let Some(ms) = self.max_lifetime_ms {
            config.max_lifetime = optional_millis(ms);
        }
        if let Some(ms) = self.create_timeout_ms {
            config.create_timeout = positive_millis("create_timeout_ms", ms)?;
        }
        if let Some(strategy) = self.strategy {
            config.strategy = match strategy {
                PoolStrategySetting::Lifo => PoolStrategy::Lifo,
                PoolStrategySetting::Fifo => PoolStrategy::Fifo,
            };
        }
        if let Some(warmup) = self.warmup {
            config.warmup = match warmup {
                WarmupSetting::None => WarmupStrategy::None,
                WarmupSetting::Sequential => WarmupStrategy::Sequential,
                WarmupSetting::Parallel => WarmupStrategy::Parallel,
                WarmupSetting::Staggered { interval_ms } => WarmupStrategy::Staggered {
                    interval: Duration::from_millis(interval_ms),
                },
            };
        }
        if let Some(test_on_checkout) = self.test_on_checkout {
            config.test_on_checkout = test_on_checkout;
        }
        if let Some(ms) = self.maintenance_interval_ms {
            config.maintenance_interval = positive_millis("maintenance_interval_ms", ms)?;
        }
        if let Some(creates) = self.max_concurrent_creates {
            if creates == 0 {
                return Err(Error::permanent(
                    "pool settings: max_concurrent_creates must be at least 1",
                ));
            }
            config.max_concurrent_creates = creates;
        }
        Ok(config)
    }
}

/// Operator settings for a [`Resident`] topology. Absent fields keep the
/// [`ResidentConfig`] defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct ResidentSettings {
    /// Recreate the shared instance when its liveness check fails.
    pub recreate_on_failure: Option<bool>,
    /// Budget for one `Provider::create`, in milliseconds; must be positive.
    pub create_timeout_ms: Option<u64>,
}

impl ResidentSettings {
    /// Resolves the settings against [`ResidentConfig`] defaults.
    ///
    /// # Errors
    ///
    /// Returns a permanent [`Error`] for a zero `create_timeout_ms`.
    pub fn into_config(self) -> Result<ResidentConfig, Error> {
        let mut config = ResidentConfig::default();
        if let Some(recreate) = self.recreate_on_failure {
            config.recreate_on_failure = recreate;
        }
        if let Some(ms) = self.create_timeout_ms {
            config.create_timeout = positive_millis("create_timeout_ms", ms)?;
        }
        Ok(config)
    }
}

/// Operator settings for a [`Bounded`] topology. There is no default mode:
/// a bounded resource must state its concurrency policy explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum BoundedSettings {
    /// At most `max_concurrent` leases; must be at least 1.
    Capped {
        /// Concurrent lease cap.
        max_concurrent: usize,
    },
    /// Exactly one lease at a time over a reused instance.
    Exclusive,
    /// No concurrency limit.
    Unbounded,
}

impl<R> ConfigurableTopology<R> for Pooled<R>
where
    R: Provider<Topology = Pooled<R>> + PoolProvider + Clone + Send + Sync + 'static,
{
    type Settings = PoolSettings;

    fn from_settings(settings: Option<PoolSettings>, fingerprint: u64) -> Result<Self, Error> {
        Self::try_new(settings.unwrap_or_default().into_config()?, fingerprint)
    }
}

impl<R> ConfigurableTopology<R> for Resident<R>
where
    R: Provider<Topology = Resident<R>> + ResidentProvider + Send + Sync + 'static,
{
    type Settings = ResidentSettings;

    fn from_settings(settings: Option<ResidentSettings>, _fingerprint: u64) -> Result<Self, Error> {
        Ok(Self::new(settings.unwrap_or_default().into_config()?))
    }
}

impl<R> ConfigurableTopology<R> for Bounded<R>
where
    R: Provider<Topology = Bounded<R>> + BoundedProvider + Send + Sync + 'static,
{
    type Settings = BoundedSettings;

    fn from_settings(settings: Option<BoundedSettings>, _fingerprint: u64) -> Result<Self, Error> {
        match settings {
            None => Err(Error::permanent(
                "bounded topology settings are required: set mode to capped, exclusive or \
                 unbounded",
            )),
            Some(BoundedSettings::Capped { max_concurrent }) => NonZeroUsize::new(max_concurrent)
                .ok_or_else(|| {
                    Error::permanent("bounded settings: max_concurrent must be at least 1")
                })
                .and_then(|cap| Self::capped(cap.get())),
            Some(BoundedSettings::Exclusive) => Ok(Self::exclusive()),
            Some(BoundedSettings::Unbounded) => Ok(Self::unbounded()),
        }
    }
}

fn optional_millis(ms: u64) -> Option<Duration> {
    (ms > 0).then(|| Duration::from_millis(ms))
}

fn positive_millis(field: &str, ms: u64) -> Result<Duration, Error> {
    if ms == 0 {
        return Err(Error::permanent(format!(
            "topology settings: {field} must be positive"
        )));
    }
    Ok(Duration::from_millis(ms))
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
