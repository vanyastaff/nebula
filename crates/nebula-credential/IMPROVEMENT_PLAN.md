# nebula-credential Improvement Plan

## Goal
Transform nebula-credential into production-ready system by adding concrete implementations, tests, and examples.

## Current Status
✅ Strong architecture (traits, security)
✅ CredentialId migrated to nebula-core
✅ Cyclic dependency fixed
❌ 0 tests
❌ 0 examples
❌ No concrete implementations

## Implementation Order

### 1. Storage Implementations (Week 1-2)
- [ ] PostgresStateStore (production backend)
- [ ] FileStateStore (dev/test)
- [ ] MemoryStateStore (testing)
- Tests: 25+

### 2. Cache Implementations (Week 2-3)
- [ ] MemoryTokenCache (L1)
- [ ] RedisTokenCache (L2)
- [ ] TieredCache (L1+L2)
- Tests: 30+

### 3. Lock Implementations (Week 3)
- [ ] RedisDistributedLock
- [ ] PostgresAdvisoryLock
- [ ] LocalLock
- Tests: 15+

### 4. Providers (Week 4-6)
- [ ] OAuth2Provider (most important)
- [ ] AwsStsProvider
- [ ] ApiKeyProvider (simple)
- [ ] BearerTokenProvider (simple)
- Tests: 50+

### 5. Testing & Examples (Week 7-8)
- [ ] Integration tests: 100+
- [ ] Examples: 10+
- [ ] Property tests: 15+

### 6. Observability (Week 9)
- [ ] Prometheus metrics
- [ ] OpenTelemetry tracing
- [ ] Audit logging

### 7. Performance (Week 10)
- [ ] Benchmarks
- [ ] Optimization
- [ ] Targets: <1ms cache hit, <50ms refresh

### 8. Documentation (Week 11)
- [ ] Update README
- [ ] Production guide
- [ ] CHANGELOG

Total: ~11 weeks

## Next: Start with PostgresStateStore
