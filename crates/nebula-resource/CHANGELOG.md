# Changelog

All notable changes to nebula-resource will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added - Phase 3 & 4 Major Enhancements

#### Phase 3 Week 15: Advanced Pooling Strategies
- **Weighted Round Robin Strategy**
  - Weight calculation based on health status, performance metrics, and response times
  - Automatic weight decay for stale health checks (>30s)
  - Resource selection proportional to calculated weights

- **Adaptive Strategy**
  - Dynamic strategy selection based on historical performance
  - Maintains selection history (up to 100 recent operations)
  - Auto-adjusts between FIFO and Weighted strategies
  - Hybrid mode with performance-based scoring

- **Performance Metrics**
  - Per-resource performance tracking (response time, success/failure counts)
  - Pool-wide metrics (wait time, utilization, failed acquisitions)
  - Average response time and success rate calculation

- **Pool Monitoring & Insights**
  - Real-time utilization tracking (under-utilized <30%, over-utilized >80%)
  - Automatic scaling recommendations (scale up/down/no change)
  - Health score calculation (0.0 to 1.0)
  - `PoolMonitoringInsights` API for operational visibility

- **Testing**
  - 7 new comprehensive tests for pooling strategies
  - All 78 tests passing

#### Phase 3 Week 16: Credential Integration
- **Core Integration**
  - New `credentials` module with nebula-credential integration
  - `ResourceCredentialProvider` for token management with caching
  - Automatic token refresh (5-minute expiration threshold)
  - Smart caching to reduce credential manager calls

- **Credential Rotation**
  - `CredentialRotationHandler` for per-resource rotation
  - `CredentialRotationScheduler` for automated rotation
  - Configurable rotation intervals (default: 1 hour)
  - Background task with graceful start/stop
  - Tracing integration for rotation events

- **Database Integration**
  - PostgreSQL support for credentials
  - URL placeholder support: `{credential}`, `{password}`, `{token}`
  - Optional `credential_provider` in resource instances
  - Feature-gated with `credentials` flag

#### Phase 4 Week 17: Testing & Examples
- **Examples**
  - `simple_pool.rs` - Basic resource pool usage
  - `credential_rotation.rs` - Credential management demonstration
  - Complete working examples with console output
  - Step-by-step documentation

- **Documentation**
  - 30+ comprehensive documentation files
  - API documentation
  - Architecture guides
  - Tutorial-style examples

### Technical Details

#### Architecture Improvements
- Extended `PoolEntry` with weight and performance fields
- Added `AdaptiveState` for strategy learning
- New `PoolMonitoringInsights` struct for monitoring
- `ScalingRecommendation` enum for autoscaling guidance
- Thread-safe credential caching with Arc<RwLock>

#### Performance Optimizations
- Weighted selection reduces overhead on unhealthy resources
- Adaptive learning minimizes poor strategy choices
- Credential caching reduces manager calls
- Efficient pool statistics tracking

#### Dependencies
- nebula-credential integration (optional)
- All existing dependencies maintained
- No breaking changes to public API

### Testing
- 78 unit tests (all passing)
- 2 working examples
- Integration ready for CI/CD

### Breaking Changes
None - all changes are additive and backwards compatible.

### Migration Guide
No migration needed. All new features are:
- Feature-gated (opt-in)
- Backwards compatible
- Default behaviors unchanged

To use new features:
```toml
[dependencies]
nebula-resource = { version = "0.1", features = ["credentials"] }
```

### Contributors
- Claude AI Assistant (implementation)
- Human oversight and requirements

---

## Previous Releases

See git history for earlier changes.
