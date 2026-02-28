# Archived From "docs/archive/cicd-setup.md"

## Overview

This document describes the continuous integration and continuous deployment setup for Nebula, including build pipelines, testing strategies, deployment workflows, and release management.

**Текущее состояние:**
- Реализован CI на GitHub Actions (`ci.yml`, `security-audit.yml`, `miri.yml`) для форматирования, линтинга, проверки, тестов, документации, security‑аудита и безопасной проверки `unsafe`‑кода.
- Разделы про Docker/Helm/Kubernetes и продакшн‑деплой описывают целевую архитектуру (Phase 5) и могут отличаться от того, что сейчас лежит в репозитории.

---

## GitHub Actions Workflows

### Main CI Workflow (текущее состояние)

```yaml
# .github/workflows/ci.yml
name: CI

on:
  push:
    branches: [main]
  pull_request:

env:
  CARGO_TERM_COLOR: always

concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true

jobs:
  fmt:
    name: Formatting
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt
      - run: cargo fmt --all -- --check

  clippy:
    name: Clippy
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo clippy --workspace -- -D warnings

  check:
    name: Check
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo check --workspace --all-targets

  test:
    name: Tests
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace

  doc:
    name: Documentation
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo doc --no-deps --workspace
```

### Security Audit Workflow (текущее состояние)

```yaml
# .github/workflows/security-audit.yml
name: Security Audit

on:
  schedule:
    - cron: '0 0 * * 1'
  push:
    branches: [main]
    paths:
      - '**/Cargo.toml'
      - '**/Cargo.lock'
      - '.cargo/audit.toml'
      - '.github/workflows/security-audit.yml'
  pull_request:
    paths:
      - '**/Cargo.toml'
      - '**/Cargo.lock'
      - '.cargo/audit.toml'
  workflow_dispatch:

jobs:
  audit:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: taiki-e/install-action@cargo-audit
      - name: Run cargo audit
        run: cargo audit
```

### Miri Safety Workflow (текущее состояние)

```yaml
# .github/workflows/miri.yml
name: Miri Safety Tests

on:
  push:
    branches: [main]
    paths:
      - "crates/memory/**"
      - ".github/workflows/miri.yml"
  pull_request:
    branches: [main]
    paths:
      - "crates/memory/**"
      - ".github/workflows/miri.yml"

jobs:
  miri:
    name: Miri Undefined Behavior Detection
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@nightly
        with:
          components: miri, rust-src
      - run: cargo miri setup
      - name: Run Miri tests on nebula-memory
        run: |
          cd crates/memory
          MIRIFLAGS="-Zmiri-permissive-provenance -Zmiri-disable-isolation" \
          cargo +nightly miri test --test safety_check
```

### Build and Deploy Workflow

```yaml
# .github/workflows/deploy.yml
name: Build and Deploy

on:
  push:
    branches: [main]
    tags: ['v*']

env:
  REGISTRY: ghcr.io
  IMAGE_NAME: ${{ github.repository }}

jobs:
  build:
    name: Build Images
    runs-on: ubuntu-latest
    permissions:
      contents: read
      packages: write
    outputs:
      version: ${{ steps.meta.outputs.version }}
    steps:
      - uses: actions/checkout@v4
      
      - name: Set up Docker Buildx
        uses: docker/setup-buildx-action@v3
        
      - name: Log in to Container Registry
        uses: docker/login-action@v3
        with:
          registry: ${{ env.REGISTRY }}
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}
          
      - name: Extract metadata
        id: meta
        uses: docker/metadata-action@v5
        with:
          images: ${{ env.REGISTRY }}/${{ env.IMAGE_NAME }}
          tags: |
            type=ref,event=branch
            type=ref,event=pr
            type=semver,pattern={{version}}
            type=semver,pattern={{major}}.{{minor}}
            type=sha
            
      - name: Build and push API image
        uses: docker/build-push-action@v5
        with:
          context: .
          file: ./docker/api.Dockerfile
          push: true
          tags: ${{ steps.meta.outputs.tags }}-api
          labels: ${{ steps.meta.outputs.labels }}
          cache-from: type=gha
          cache-to: type=gha,mode=max
          
      - name: Build and push Worker image
        uses: docker/build-push-action@v5
        with:
          context: .
          file: ./docker/worker.Dockerfile
          push: true
          tags: ${{ steps.meta.outputs.tags }}-worker
          labels: ${{ steps.meta.outputs.labels }}
          cache-from: type=gha
          cache-to: type=gha,mode=max
          
      - name: Build and push Engine image
        uses: docker/build-push-action@v5
        with:
          context: .
          file: ./docker/engine.Dockerfile
          push: true
          tags: ${{ steps.meta.outputs.tags }}-engine
          labels: ${{ steps.meta.outputs.labels }}
          cache-from: type=gha
          cache-to: type=gha,mode=max

  deploy-staging:
    name: Deploy to Staging
    needs: build
    runs-on: ubuntu-latest
    environment: staging
    steps:
      - uses: actions/checkout@v4
      
      - name: Install kubectl
        uses: azure/setup-kubectl@v3
        
      - name: Configure kubectl
        run: |
          echo "${{ secrets.KUBE_CONFIG }}" | base64 -d > kubeconfig
          export KUBECONFIG=kubeconfig
          
      - name: Deploy to staging
        run: |
          helm upgrade --install nebula-staging ./charts/nebula \
            --namespace staging \
            --create-namespace \
            --values ./charts/nebula/values.staging.yaml \
            --set image.tag=${{ needs.build.outputs.version }} \
            --wait
            
      - name: Run smoke tests
        run: |
          kubectl run smoke-test \
            --image=${{ env.REGISTRY }}/${{ env.IMAGE_NAME }}:${{ needs.build.outputs.version }}-test \
            --rm -i --restart=Never \
            --namespace staging \
            -- /app/smoke-tests.sh

  deploy-production:
    name: Deploy to Production
    needs: [build, deploy-staging]
    runs-on: ubuntu-latest
    environment: production
    if: startsWith(github.ref, 'refs/tags/v')
    steps:
      - uses: actions/checkout@v4
      
      - name: Configure kubectl
        run: |
          echo "${{ secrets.PROD_KUBE_CONFIG }}" | base64 -d > kubeconfig
          export KUBECONFIG=kubeconfig
          
      - name: Deploy to production
        run: |
          helm upgrade --install nebula ./charts/nebula \
            --namespace production \
            --create-namespace \
            --values ./charts/nebula/values.production.yaml \
            --set image.tag=${{ needs.build.outputs.version }} \
            --atomic \
            --timeout 10m
```

### PR Validation Workflow

```yaml
# .github/workflows/pr-validation.yml
name: PR Validation

on:
  pull_request:
    types: [opened, synchronize, reopened]

jobs:
  validate:
    name: Validate PR
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
          
      - name: Check commit messages
        uses: wagoid/commitlint-github-action@v5
        
      - name: Check PR title
        uses: deepakputhraya/action-pr-title@master
        with:
          regex: '^(feat|fix|docs|style|refactor|perf|test|chore)(\(.+\))?: .+'
          
      - name: Label PR
        uses: actions/labeler@v4
        with:
          repo-token: "${{ secrets.GITHUB_TOKEN }}"
          
      - name: Size label
        uses: codelytv/pr-size-labeler@v1
        with:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          
      - name: Check for breaking changes
        run: |
          cargo install cargo-breaking
          cargo breaking

  benchmark:
    name: Performance Regression Check
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
          
      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
        
      - name: Run benchmarks
        run: cargo bench --all-features -- --output-format bencher | tee output.txt
        
      - name: Store benchmark result
        uses: benchmark-action/github-action-benchmark@v1
        with:
          tool: 'cargo'
          output-file-path: output.txt
          github-token: ${{ secrets.GITHUB_TOKEN }}
          auto-push: false
          comment-on-alert: true
          alert-threshold: '150%'
          fail-on-alert: true
```

---

## Testing Strategies

### Unit Test Execution

```yaml
# .github/workflows/test-matrix.yml
name: Test Matrix

on: [push, pull_request]

jobs:
  test:
    name: Test ${{ matrix.crate }}
    runs-on: ubuntu-latest
    strategy:
      matrix:
        crate:
          - nebula-core
          - nebula-memory
          - nebula-engine
          - nebula-worker
          - nebula-api
    steps:
      - uses: actions/checkout@v4
      
      - name: Run tests for ${{ matrix.crate }}
        run: |
          cd crates/${{ matrix.crate }}
          cargo test --all-features
```

### Integration Tests

```bash
#!/bin/bash
# scripts/integration-tests.sh

set -e

echo "Starting integration test environment..."

# Start test dependencies
docker-compose -f docker-compose.test.yml up -d

# Wait for services
./scripts/wait-for-it.sh localhost:5432 -- echo "PostgreSQL ready"
./scripts/wait-for-it.sh localhost:9092 -- echo "Kafka ready"

# Run integration tests
cargo test --test '*' --features integration-tests

# Cleanup
docker-compose -f docker-compose.test.yml down -v
```

### End-to-End Tests

```typescript
// e2e/workflow-execution.spec.ts
import { test, expect } from '@playwright/test';

test.describe('Workflow Execution', () => {
  test('should execute simple workflow', async ({ request }) => {
    // Create workflow
    const workflow = await request.post('/api/v1/workflows', {
      data: {
        name: 'Test Workflow',
        nodes: [
          {
            id: 'start',
            type: 'http_request',
            config: {
              url: 'https://api.example.com/data'
            }
          }
        ]
      }
    });
    
    expect(workflow.ok()).toBeTruthy();
    const workflowData = await workflow.json();
    
    // Execute workflow
    const execution = await request.post(
      `/api/v1/workflows/${workflowData.id}/execute`,
      {
        data: {
          input: { test: true }
        }
      }
    );
    
    expect(execution.ok()).toBeTruthy();
    const executionData = await execution.json();
    
    // Poll for completion
    let status;
    for (let i = 0; i < 30; i++) {
      const result = await request.get(
        `/api/v1/executions/${executionData.execution_id}`
      );
      const data = await result.json();
      status = data.status;
      
      if (status === 'completed' || status === 'failed') {
        break;
      }
      
      await new Promise(resolve => setTimeout(resolve, 1000));
    }
    
    expect(status).toBe('completed');
  });
});
```

