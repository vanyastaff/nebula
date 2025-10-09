# Nebula Expression - Priority Matrix & Dependencies

## 🎯 Матрица приоритетов (Eisenhower Matrix)

```
                    URGENT
                      ↑
        ┌─────────────┼─────────────┐
        │             │             │
        │     P0      │     P1      │
        │  DO FIRST   │  SCHEDULE   │
        │             │             │
I   ────┤  • Template │  • Token    │
M       │  • Engine   │  • Error    │
P       │  • Context  │    Context  │
O       │  • AST      │  • Iterator │
R       │  • Lexer    │    Builtins │
T       │  • Safety   │             │
A       │    Limits   │             │
N       │  • API      │             │
T   ────┤─────────────┼─────────────┤
        │             │             │
        │     P2      │     P3      │
        │  DELEGATE   │  ELIMINATE  │
        │             │             │
        │  • Constant │  • Color    │
        │    Folding  │    Output   │
        │  • SIMD     │  • Docs     │
        │  • Optics   │             │
        │             │             │
        └─────────────┼─────────────┘
                      ↓
                  NOT URGENT
```

---

## 📊 Impact vs Effort Matrix

```
HIGH IMPACT
    ↑
    │
    │  ┌──────────┐  ┌──────────┐
    │  │ Template │  │ Context  │
    │  │  (4h)    │  │  (5.5h)  │
    │  │   P0.1   │  │   P0.3   │
    │  └──────────┘  └──────────┘
    │
    │  ┌──────────┐  ┌──────────┐
    │  │  Engine  │  │   AST    │
    │  │  (3h)    │  │   (6h)   │
    │  │   P0.2   │  │   P0.4   │
    │  └──────────┘  └──────────┘
    │
    │  ┌──────────┐  ┌──────────┐
 I  │  │  Regex   │  │  Lexer   │
 M  │  │  Cache   │  │  Zero    │
 P  │  │  (2.5h)  │  │  (6.5h)  │
 A  │  │   P0.8   │  │   P0.5   │
 C  │  └──────────┘  └──────────┘
 T  │
    │  ┌──────────┐  ┌──────────┐
    │  │ API      │  │  Token   │
    │  │  (1.5h)  │  │  (4h)    │
    │  │  P0.10   │  │   P1.1   │
    │  └──────────┘  └──────────┘
    │
    │         ┌──────────────────┐
    │         │ Error Recovery   │
    │         │      (8h)        │
    │         │       P2.2       │
    │         └──────────────────┘
    │
LOW │  ┌─────────────────────────┐
    │  │   Color Output (2h)     │
    │  │        P3.1             │
    │  └─────────────────────────┘
    │
    └────────────────────────────────→
       LOW EFFORT            HIGH EFFORT
```

**Легенда**:
- 🟢 Low Effort: < 3h
- 🟡 Medium Effort: 3-6h
- 🔴 High Effort: > 6h

---

## 🔗 Граф зависимостей

```
┌─────────────────────────────────────────────────────────┐
│                    Week 1: Foundation                   │
└─────────────────────────────────────────────────────────┘

    ┌──────────────┐
    │ P0.1         │
    │ Template     │──┐
    │ Zero-Copy    │  │
    └──────────────┘  │
                      ├──→ Integration Tests
    ┌──────────────┐  │         ↓
    │ P0.2         │  │    Benchmarks
    │ Engine       │──┤         ↓
    │ RwLock       │  │    Performance OK?
    └──────────────┘  │
                      │
    ┌──────────────┐  │
    │ P0.3         │  │
    │ Context      │──┤
    │ Arc          │  │
    └──────────────┘  │
                      │
    ┌──────────────┐  │
    │ P0.4         │  │
    │ AST          │──┘
    │ Interning    │
    └──────────────┘

┌─────────────────────────────────────────────────────────┐
│                  Week 2: Safety + Perf                  │
└─────────────────────────────────────────────────────────┘

    ┌──────────────┐
    │ P0.5         │
    │ Lexer        │────────→ P0.9 (Parser)
    │ Zero-Copy    │              ↓
    └──────────────┘         Recursion
                             Limit
    ┌──────────────┐              ↓
    │ P0.6         │         DoS Testing
    │ Eval         │──┐
    │ Recursion    │  │
    └──────────────┘  │
                      ├──→ Security Audit
    ┌──────────────┐  │         ↓
    │ P0.7         │  │    Fuzzing
    │ Short-       │──┤
    │ circuit      │  │
    └──────────────┘  │
                      │
    ┌──────────────┐  │
    │ P0.8         │  │
    │ Regex        │──┘
    │ Cache        │
    └──────────────┘

┌─────────────────────────────────────────────────────────┐
│                    Week 3: API                          │
└─────────────────────────────────────────────────────────┘

    ┌──────────────┐
    │ P0.10        │
    │ API Surface  │───┐
    └──────────────┘   │
                       ├──→ Migration Guide
    ┌──────────────┐   │         ↓
    │ P0.11        │   │    Update Examples
    │ Feature      │───┤         ↓
    │ Flags        │   │    Documentation
    └──────────────┘   │
                       │
    ┌──────────────┐   │
    │ P0.12        │   │
    │ Builtin      │───┘
    │ Type Safety  │
    └──────────────┘
```

---

## 🎯 Критический путь (Critical Path)

### Последовательность выполнения

```
START
  ↓
P0.1 Template (4h)
  ↓
P0.2 Engine (3h)
  ↓
[Integration Test]
  ↓
P0.3 Context (5.5h)
  ↓
P0.4 AST (6h)
  ↓
[Benchmark Suite]
  ↓
P0.5 Lexer (6.5h)
  ↓
P0.6 Eval Safety (3.5h)
  ↓
P0.7 Short-circuit (3.5h)
  ↓
P0.8 Regex (2.5h)
  ↓
P0.9 Parser Safety (2.5h)
  ↓
[Security Audit]
  ↓
P0.10 API (1.5h)
  ↓
P0.11 Features (3.5h)
  ↓
P0.12 Type Safety (7h)
  ↓
[Final Review]
  ↓
DONE
```

**Total Critical Path**: ~49 hours (~6 working days)

---

## 📈 ROI Ranking (Return on Investment)

### Top 10 by ROI

| Rank | Task | Effort | Impact | ROI Score |
|------|------|--------|--------|-----------|
| 1 | P0.2 Engine RwLock | 3h | 🔴🔴🔴🔴🔴 | **10.0** |
| 2 | P0.8 Regex Cache | 2.5h | 🔴🔴🔴🔴 | **9.6** |
| 3 | P0.10 API Surface | 1.5h | 🔴🔴🔴 | **9.3** |
| 4 | P0.7 Short-circuit | 3.5h | 🔴🔴🔴🔴 | **9.1** |
| 5 | P0.1 Template | 4h | 🔴🔴🔴🔴🔴 | **9.0** |
| 6 | P0.6 Eval Recursion | 3.5h | 🔴🔴🔴 | **8.6** |
| 7 | P0.9 Parser Recursion | 2.5h | 🔴🔴 | **8.0** |
| 8 | P0.3 Context Arc | 5.5h | 🔴🔴🔴🔴 | **7.5** |
| 9 | P0.4 AST Interning | 6h | 🔴🔴🔴🔴 | **7.0** |
| 10 | P0.5 Lexer Zero-Copy | 6.5h | 🔴🔴🔴 | **6.5** |

**ROI Formula**: `Impact (1-5) × 10 / Effort (hours)`

### Рекомендуемый порядок (по ROI)

1. **Quick Wins** (Week 1, Days 1-2):
   - P0.2 Engine RwLock (3h)
   - P0.8 Regex Cache (2.5h)
   - P0.10 API Surface (1.5h)

2. **High Impact** (Week 1, Days 3-5):
   - P0.1 Template (4h)
   - P0.7 Short-circuit (3.5h)
   - P0.3 Context Arc (5.5h)

3. **Safety Critical** (Week 2):
   - P0.6 Eval Recursion (3.5h)
   - P0.9 Parser Recursion (2.5h)
   - P0.5 Lexer Zero-Copy (6.5h)

4. **Architecture** (Week 3):
   - P0.4 AST Interning (6h)
   - P0.11 Features (3.5h)
   - P0.12 Type Safety (7h)

---

## 🚦 Traffic Light Status

### Current Status (Pre-P0)

```
Performance     🔴 Poor
  Concurrent:   🔴 10k ops/sec (target: 75k)
  Allocations:  🔴 15 per eval (target: 3)
  Memory:       🔴 High (no sharing)

Safety          🔴 Critical
  DoS:          🔴 Vulnerable
  Crashes:      🔴 Possible (stack overflow)
  Errors:       🟡 Basic (no context)

API Quality     🟡 Medium
  Stability:    🔴 Internals exposed
  Features:     🔴 No optional deps
  Docs:         🟡 Basic coverage

Testing         🟡 Medium
  Unit:         🟢 Good coverage
  Integration:  🟡 Basic
  Fuzzing:      🔴 None
```

### After P0

```
Performance     🟢 Excellent
  Concurrent:   🟢 75k ops/sec
  Allocations:  🟢 3 per eval
  Memory:       🟢 Efficient sharing

Safety          🟢 Robust
  DoS:          🟢 Protected
  Crashes:      🟢 Prevented
  Errors:       🟢 Rich context

API Quality     🟢 Excellent
  Stability:    🟢 Clean boundary
  Features:     🟢 Optional deps
  Docs:         🟢 Comprehensive

Testing         🟢 Strong
  Unit:         🟢 Excellent
  Integration:  🟢 Comprehensive
  Fuzzing:      🟢 Integrated
```

---

## 📊 Gantt Chart (P0 Tasks)

```
Week 1: Foundation
Days:    Mon    Tue    Wed    Thu    Fri
P0.1  [=====]
P0.2        [====]
P0.3             [=========]
P0.4                  [==========]
Test                         [===]

Week 2: Safety
Days:    Mon    Tue    Wed    Thu    Fri
P0.5  [===========]
P0.6                [======]
P0.7                      [======]
P0.8                            [====]
P0.9                                [====]

Week 3: API
Days:    Mon    Tue    Wed    Thu    Fri
P0.10 [==]
P0.11    [======]
P0.12         [============]
Docs                       [=====]

Legend:
[===] = Working on task
      = Free/Testing
```

---

## 🎯 Milestone Checklist

### Milestone 1: Foundation (End of Week 1)

- [ ] P0.1: Template Zero-Copy implemented
  - [ ] Tests passing
  - [ ] Benchmark shows 5x improvement
  - [ ] Zero breaking changes

- [ ] P0.2: Engine RwLock implemented
  - [ ] Concurrent benchmark shows 7.5x
  - [ ] No deadlocks

- [ ] P0.3: Context Arc implemented
  - [ ] Clone benchmark shows 40x
  - [ ] Nested scopes working

- [ ] P0.4: AST Interning implemented
  - [ ] Clone benchmark shows 10x
  - [ ] Memory test shows 50% reduction

**Gate**: All benchmarks pass targets

---

### Milestone 2: Safety (End of Week 2)

- [ ] P0.5: Lexer Zero-Copy implemented
  - [ ] 0 allocations confirmed
  - [ ] Unicode tests passing

- [ ] P0.6: Eval Recursion Limit
  - [ ] DoS test blocked (depth > 100)
  - [ ] Clear error message

- [ ] P0.7: Short-circuit implemented
  - [ ] `false && f()` doesn't call f
  - [ ] `null?.prop` works

- [ ] P0.8: Regex Cache implemented
  - [ ] Benchmark shows 100x (cached)
  - [ ] LRU eviction works

- [ ] P0.9: Parser Recursion Limit
  - [ ] DoS test blocked
  - [ ] Error message clear

**Gate**: Security audit passes, fuzzing finds no issues

---

### Milestone 3: API (End of Week 3)

- [ ] P0.10: API Surface clean
  - [ ] No public internal modules
  - [ ] Migration guide written

- [ ] P0.11: Feature Flags working
  - [ ] `--no-default-features` builds
  - [ ] Optional deps conditional

- [ ] P0.12: Builtin Type Safety
  - [ ] All builtins migrated
  - [ ] Compile-time checks work

**Gate**: No breaking changes for existing users, docs complete

---

## 🔄 Feedback Loop

```
Plan → Implement → Test → Measure → Review
  ↑                                    ↓
  └────────────────────────────────────┘
           Iterate until targets met
```

### Metrics to Track

**Per Task**:
- [ ] Time actual vs estimated
- [ ] Performance improvement actual vs target
- [ ] Breaking changes (expected: 0)
- [ ] Bugs found in testing

**Per Week**:
- [ ] Cumulative performance gain
- [ ] Test coverage %
- [ ] Documentation completeness
- [ ] Code review feedback

**Per Milestone**:
- [ ] All success criteria met
- [ ] Benchmarks validate improvements
- [ ] No regressions
- [ ] User feedback positive

---

## 🎓 Learning & Knowledge Transfer

### Key Concepts to Document

1. **Zero-Copy Patterns**
   - When to use `Cow<'a, str>`
   - Lifetime design patterns
   - `SmallVec` usage

2. **Concurrency Patterns**
   - `RwLock` vs `Mutex`
   - `Arc` vs `Rc`
   - Lock contention mitigation

3. **Performance Optimization**
   - Profiling methodology
   - Benchmark interpretation
   - Allocation tracking

4. **API Design**
   - Stability guarantees
   - Feature flags strategy
   - Breaking change policy

### Knowledge Base Articles

- [ ] "Zero-Copy Template Design" (after P0.1)
- [ ] "Concurrent Caching Patterns" (after P0.2)
- [ ] "Arc-based Context Pattern" (after P0.3)
- [ ] "String Interning Best Practices" (after P0.4)
- [ ] "DoS Protection Techniques" (after P0.6-P0.9)
- [ ] "API Stability Guidelines" (after P0.10-P0.12)

---

## 📞 Communication Plan

### Daily Standups

**Questions**:
1. What did you complete yesterday?
2. What are you working on today?
3. Any blockers?

### Weekly Reviews

**Agenda**:
1. Milestone progress
2. Benchmark results
3. Risks & mitigations
4. Next week plan

### Monthly Retrospectives

**Topics**:
1. What went well?
2. What could improve?
3. Action items for next month

---

## 🚨 Risk Mitigation

### Identified Risks

| Risk | Probability | Impact | Mitigation |
|------|------------|--------|------------|
| Breaking changes slip in | Medium | High | Comprehensive tests, deprecation policy |
| Performance regressions | Low | High | Continuous benchmarking, gates |
| Scope creep | Medium | Medium | Strict priority adherence, weekly reviews |
| Implementation bugs | Medium | Medium | Code review, extensive testing |
| Schedule delays | Low | Medium | Buffer time, parallel work where possible |

---

## ✅ Definition of Done

For each task to be considered "done":

- [ ] Code implemented & reviewed
- [ ] All tests pass (unit + integration)
- [ ] Benchmarks show expected improvement
- [ ] Documentation updated
- [ ] No clippy warnings
- [ ] CHANGELOG.md updated
- [ ] Migration guide (if breaking)
- [ ] Peer reviewed & approved
- [ ] Merged to main

---

**Last Updated**: 2025-01-08
**Version**: 1.0
**Owner**: Development Team
