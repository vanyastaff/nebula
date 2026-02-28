# Archived From "docs/archive/nebula-complete.md"

#### 5. nebula-expression (Week 4)
- [ ] 5.1 **Parser Development**
  - [ ] 5.1.1 Define grammar specification
  - [ ] 5.1.2 Implement tokenizer
  - [ ] 5.1.3 Implement recursive descent parser
  - [ ] 5.1.4 Create AST structures
  - [ ] 5.1.5 Add error recovery
  - [ ] 5.1.6 Add position tracking
  - [ ] 5.1.7 Write parser tests

- [ ] 5.2 **Core Expressions**
  - [ ] 5.2.1 Variable access ($nodes, $vars, etc)
  - [ ] 5.2.2 Property access (dot notation)
  - [ ] 5.2.3 Array indexing
  - [ ] 5.2.4 Method calls
  - [ ] 5.2.5 Literals (string, number, bool)
  - [ ] 5.2.6 Null handling

- [ ] 5.3 **Operators**
  - [ ] 5.3.1 Arithmetic operators (+, -, *, /, %)
  - [ ] 5.3.2 Comparison operators (==, !=, <, >, <=, >=)
  - [ ] 5.3.3 Logical operators (&&, ||, !)
  - [ ] 5.3.4 Ternary operator (? :)
  - [ ] 5.3.5 Null coalescing (??)
  - [ ] 5.3.6 String concatenation

- [ ] 5.4 **Functions**
  - [ ] 5.4.1 String functions (concat, substring, etc)
  - [ ] 5.4.2 Array functions (filter, map, reduce)
  - [ ] 5.4.3 Date functions (format, parse, add)
  - [ ] 5.4.4 Math functions (round, floor, ceil)
  - [ ] 5.4.5 Type conversion functions
  - [ ] 5.4.6 Custom function registration

- [ ] 5.5 **Evaluator**
  - [ ] 5.5.1 Create evaluation context
  - [ ] 5.5.2 Implement AST walker
  - [ ] 5.5.3 Add type checking
  - [ ] 5.5.4 Add short-circuit evaluation
  - [ ] 5.5.5 Add error handling
  - [ ] 5.5.6 Add performance optimization

---

## nebula-expression

### Purpose
Полноценный expression язык для динамического вычисления значений в workflows.

### Responsibilities
- Парсинг expressions
- Вычисление expressions
- Функции и операторы
- Type checking

### Architecture
```rust
pub struct ExpressionEngine {
    parser: Parser,
    evaluator: Evaluator,
    functions: FunctionRegistry,
    operators: OperatorRegistry,
}
```

### Expression Examples
```
// Простой доступ
$nodes.http_request.body.user.email

// Операторы
$nodes.calc.value * 100 + $vars.base_amount

// Функции
concat($nodes.first_name.output, " ", $nodes.last_name.output)
formatDate(now(), "YYYY-MM-DD")

// Pipe operations
$nodes.users.list 
  | filter(u => u.active) 
  | map(u => u.email)
  | join(", ")

// Условные выражения
$vars.env == "prod" ? $nodes.prod_config : $nodes.dev_config
```

---

