//! Operator-facing topology settings.
//!
//! Registration JSON configures two separate things: the resource's own
//! [`Provider::Config`] (how to connect) and the
//! topology (how much capacity to hold). This module is the wire format for
//! the second one. Every field is optional, so `{}` or an absent value means
//! the built-in defaults. The format is flat (no tagged unions) so it maps to
//! a form: modes are string selects and a mode's extra parameter is a sibling
//! field that is rejected when the mode does not use it. Durations are integer
//! milliseconds with an explicit `_ms` suffix, unknown fields are rejected,
//! and every value is validated
//! through the topology's fallible constructor — operator input never reaches
//! a panicking path.
//!
//! [`ConfigurableTopology`] maps a settings value to a built topology. The
//! built-in [`Pooled`], [`Resident`] and [`Bounded`] implement it; a custom
//! topology may implement it to accept operator settings the same way.

use std::{num::NonZeroUsize, time::Duration};

use nebula_schema::{EnumSelect, HasSchema, Schema};
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
    /// Wire format of this topology's settings; its schema is published
    /// through [`ResourceFactory::topology_schema`](crate::ResourceFactory::topology_schema).
    type Settings: DeserializeOwned + HasSchema;

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

    /// Topology factory for [`KindActivator`](crate::KindActivator): builds
    /// from the request's topology settings with the initial fingerprint `0`
    /// (the manager advances it on config reload).
    ///
    /// # Errors
    ///
    /// As [`from_settings_value`](Self::from_settings_value).
    fn from_registration(settings: Option<&serde_json::Value>) -> Result<Self, Error> {
        Self::from_settings_value(settings, 0)
    }
}

/// Wraps a settings-free topology constructor as a
/// [`KindActivator`](crate::KindActivator) topology factory.
///
/// Operator settings sent to such a kind are **rejected**, never silently
/// ignored: an operator who sized a pool must not get a default one.
pub fn fixed<T, F>(
    build: F,
) -> impl Fn(Option<&serde_json::Value>) -> Result<T, Error> + Send + Sync
where
    F: Fn() -> T + Send + Sync,
{
    move |settings| match settings {
        None | Some(serde_json::Value::Null) => Ok(build()),
        Some(_) => Err(Error::permanent(
            "this resource kind does not accept topology settings",
        )),
    }
}

/// Idle-queue order for [`PoolSettings::strategy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, EnumSelect)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PoolStrategySetting {
    /// Reuse the most recently returned instance.
    #[field(label = "LIFO — reuse the hottest instance")]
    Lifo,
    /// Rotate through every instance.
    #[field(label = "FIFO — rotate evenly")]
    Fifo,
}

/// Startup warmup for [`PoolSettings::warmup`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, EnumSelect)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WarmupSetting {
    /// Create instances on demand.
    #[field(label = "None — create on first use")]
    None,
    /// Create `min_size` instances one at a time.
    #[field(label = "Sequential")]
    Sequential,
    /// Create `min_size` instances concurrently.
    #[field(label = "Parallel")]
    Parallel,
    /// Create instances with [`PoolSettings::warmup_interval_ms`] between them.
    #[field(label = "Staggered")]
    Staggered,
}

/// Operator settings for a [`Pooled`] topology. Absent fields keep the
/// [`PoolConfig`] defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Schema)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct PoolSettings {
    /// Warmup target and minimum idle instances.
    #[field(
        label = "Minimum size",
        description = "Warmup target and minimum idle instances"
    )]
    pub min_size: Option<u32>,
    /// Hard cap on instances (idle and checked out); must be at least 1.
    #[field(
        label = "Maximum size",
        description = "Cap on idle plus checked-out instances; at least 1"
    )]
    pub max_size: Option<u32>,
    /// Idle eviction threshold in milliseconds; `0` disables idle eviction.
    #[field(label = "Idle timeout (ms)", description = "0 disables idle eviction")]
    pub idle_timeout_ms: Option<u64>,
    /// Maximum instance lifetime in milliseconds; `0` disables it.
    #[field(
        label = "Max lifetime (ms)",
        description = "0 disables the lifetime limit"
    )]
    pub max_lifetime_ms: Option<u64>,
    /// Budget for one `Provider::create`, in milliseconds; must be positive.
    #[field(
        label = "Create timeout (ms)",
        description = "Budget for one create; must be positive"
    )]
    pub create_timeout_ms: Option<u64>,
    /// Idle-queue order.
    #[field(label = "Idle order", enum_select)]
    pub strategy: Option<PoolStrategySetting>,
    /// Startup warmup.
    #[field(label = "Warmup", enum_select)]
    pub warmup: Option<WarmupSetting>,
    /// Delay between staggered warmup creations, in milliseconds. Required
    /// with `warmup = staggered` and rejected otherwise.
    #[field(
        label = "Warmup interval (ms)",
        description = "Only with staggered warmup"
    )]
    pub warmup_interval_ms: Option<u64>,
    /// Run `Provider::check` on every checkout.
    #[field(label = "Test on checkout")]
    pub test_on_checkout: Option<bool>,
    /// Background maintenance interval in milliseconds; must be positive.
    #[field(label = "Maintenance interval (ms)", description = "Must be positive")]
    pub maintenance_interval_ms: Option<u64>,
    /// Cap on concurrent `Provider::create` calls; must be at least 1.
    #[field(label = "Concurrent creates", description = "At least 1")]
    pub max_concurrent_creates: Option<u32>,
}

impl PoolSettings {
    /// Resolves the settings against [`PoolConfig`] defaults.
    ///
    /// # Errors
    ///
    /// Returns a permanent [`Error`] for a zero `create_timeout_ms`,
    /// `maintenance_interval_ms` or `max_concurrent_creates`, and for a
    /// `warmup_interval_ms` that does not match the warmup mode. Size
    /// invariants are checked by [`Pooled::try_new`].
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
        match (self.warmup, self.warmup_interval_ms) {
            (Some(WarmupSetting::Staggered), Some(ms)) => {
                config.warmup = WarmupStrategy::Staggered {
                    interval: positive_millis("warmup_interval_ms", ms)?,
                };
            },
            (Some(WarmupSetting::Staggered), None) => {
                return Err(Error::permanent(
                    "pool settings: staggered warmup requires warmup_interval_ms",
                ));
            },
            (_, Some(_)) => {
                return Err(Error::permanent(
                    "pool settings: warmup_interval_ms applies only to staggered warmup",
                ));
            },
            (Some(WarmupSetting::None), None) => config.warmup = WarmupStrategy::None,
            (Some(WarmupSetting::Sequential), None) => config.warmup = WarmupStrategy::Sequential,
            (Some(WarmupSetting::Parallel), None) => config.warmup = WarmupStrategy::Parallel,
            (None, None) => {},
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Schema)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct ResidentSettings {
    /// Recreate the shared instance when its liveness check fails.
    #[field(label = "Recreate on failure")]
    pub recreate_on_failure: Option<bool>,
    /// Budget for one `Provider::create`, in milliseconds; must be positive.
    #[field(
        label = "Create timeout (ms)",
        description = "Budget for one create; must be positive"
    )]
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

/// Concurrency policy for [`BoundedSettings::mode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, EnumSelect)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BoundedModeSetting {
    /// At most [`BoundedSettings::max_concurrent`] leases.
    #[field(label = "Capped")]
    Capped,
    /// Exactly one lease at a time over a reused instance.
    #[field(label = "Exclusive")]
    Exclusive,
    /// No concurrency limit.
    #[field(label = "Unbounded")]
    Unbounded,
}

/// Operator settings for a [`Bounded`] topology. There is no default mode:
/// a bounded resource must state its concurrency policy explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Schema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct BoundedSettings {
    /// Concurrency policy.
    #[field(label = "Mode", enum_select)]
    pub mode: BoundedModeSetting,
    /// Concurrent lease cap. Required with `mode = capped` (at least 1) and
    /// rejected otherwise.
    #[field(label = "Max concurrent leases", description = "Only with capped mode")]
    #[serde(default)]
    pub max_concurrent: Option<u32>,
}

impl BoundedSettings {
    /// Settings for `Capped(max_concurrent)`.
    #[must_use]
    pub const fn capped(max_concurrent: u32) -> Self {
        Self {
            mode: BoundedModeSetting::Capped,
            max_concurrent: Some(max_concurrent),
        }
    }

    /// Settings for `Exclusive` or `Unbounded` (no cap).
    #[must_use]
    pub const fn uncapped(mode: BoundedModeSetting) -> Self {
        Self {
            mode,
            max_concurrent: None,
        }
    }
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
        let Some(settings) = settings else {
            return Err(Error::permanent(
                "bounded topology settings are required: set mode to capped, exclusive or \
                 unbounded",
            ));
        };
        match (settings.mode, settings.max_concurrent) {
            (BoundedModeSetting::Capped, Some(cap)) => usize::try_from(cap)
                .ok()
                .and_then(NonZeroUsize::new)
                .ok_or_else(|| {
                    Error::permanent("bounded settings: max_concurrent must be at least 1")
                })
                .and_then(|cap| Self::capped(cap.get())),
            (BoundedModeSetting::Capped, None) => Err(Error::permanent(
                "bounded settings: capped mode requires max_concurrent",
            )),
            (_, Some(_)) => Err(Error::permanent(
                "bounded settings: max_concurrent applies only to capped mode",
            )),
            (BoundedModeSetting::Exclusive, None) => Ok(Self::exclusive()),
            (BoundedModeSetting::Unbounded, None) => Ok(Self::unbounded()),
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
