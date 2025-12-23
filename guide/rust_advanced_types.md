# Продвинутые возможности системы типов Rust

## Generic Features (Обобщённое программирование)

### Базовые

**Generic Types:**
```rust
struct Container<T> {
    value: T,
}

enum Option<T> {
    Some(T),
    None,
}
```

**Generic Functions:**
```rust
fn print_type<T: std::fmt::Debug>(value: T) {
    println!("{:?}", value);
}
```

**Generic Implementations:**
```rust
impl<T> Container<T> {
    fn new(value: T) -> Self {
        Self { value }
    }
}

impl<T: Clone> Container<T> {
    fn duplicate(&self) -> T {
        self.value.clone()
    }
}
```

**Trait Bounds:**
```rust
fn process<T: Clone + Debug>(item: T) {
    let copy = item.clone();
    println!("{:?}", copy);
}
```

**Where Clauses:**
```rust
fn complex_function<T, U>(t: T, u: U) -> String
where
    T: Display + Clone,
    U: Debug + Into<String>,
{
    format!("{} {:?}", t, u)
}
```

**Const Generics:**
```rust
struct Array<T, const N: usize> {
    data: [T; N],
}

impl<T: Default + Copy, const N: usize> Array<T, N> {
    fn new() -> Self {
        Self {
            data: [T::default(); N],
        }
    }
}

// Использование
let arr: Array<i32, 5> = Array::new();
```

### Продвинутые

**GAT (Generic Associated Types):**
```rust
trait Container {
    type Item<'a> where Self: 'a;
    
    fn get<'a>(&'a self) -> Self::Item<'a>;
}

struct VecContainer<T> {
    data: Vec<T>,
}

impl<T> Container for VecContainer<T> {
    type Item<'a> = &'a T where T: 'a;
    
    fn get<'a>(&'a self) -> Self::Item<'a> {
        &self.data[0]
    }
}
```

**HRTB (Higher-Rank Trait Bounds):**
```rust
// Функция, которая принимает замыкание, работающее с любым временем жизни
fn apply_to_ref<F>(f: F)
where
    F: for<'a> Fn(&'a str) -> &'a str,
{
    let s = String::from("hello");
    println!("{}", f(&s));
}

// Использование
apply_to_ref(|s| s);
```

**Generic Const Expressions (unstable):**
```rust
#![feature(generic_const_exprs)]

struct ArrayPair<T, const N: usize>
where
    [T; N * 2]: Sized,
{
    data: [T; N * 2],
}
```

**Negative Trait Bounds (unstable):**
```rust
#![feature(negative_impls)]

struct NotSendable {
    value: i32,
}

impl !Send for NotSendable {}
```

## Associated Items

**Associated Types:**
```rust
trait Iterator {
    type Item;
    
    fn next(&mut self) -> Option<Self::Item>;
}

struct Counter {
    count: u32,
}

impl Iterator for Counter {
    type Item = u32;
    
    fn next(&mut self) -> Option<Self::Item> {
        self.count += 1;
        Some(self.count)
    }
}
```

**Associated Constants:**
```rust
trait Math {
    const PI: f64;
    const E: f64;
}

struct StandardMath;

impl Math for StandardMath {
    const PI: f64 = 3.14159;
    const E: f64 = 2.71828;
}

// Использование
let pi = StandardMath::PI;
```

**Associated Functions:**
```rust
struct Point {
    x: f64,
    y: f64,
}

impl Point {
    // Ассоциированная функция (не метод)
    fn origin() -> Self {
        Self { x: 0.0, y: 0.0 }
    }
    
    // Метод
    fn distance(&self, other: &Point) -> f64 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }
}

// Использование
let p = Point::origin();
```

## Trait System Features

**Trait Objects:**
```rust
trait Draw {
    fn draw(&self);
}

struct Circle;
struct Square;

impl Draw for Circle {
    fn draw(&self) { println!("Drawing circle"); }
}

impl Draw for Square {
    fn draw(&self) { println!("Drawing square"); }
}

// Трейт-объект
let shapes: Vec<Box<dyn Draw>> = vec![
    Box::new(Circle),
    Box::new(Square),
];

for shape in shapes {
    shape.draw();
}
```

**Trait Inheritance:**
```rust
trait Animal {
    fn make_sound(&self);
}

trait Dog: Animal {
    fn wag_tail(&self);
}

struct GoldenRetriever;

impl Animal for GoldenRetriever {
    fn make_sound(&self) {
        println!("Woof!");
    }
}

impl Dog for GoldenRetriever {
    fn wag_tail(&self) {
        println!("*wags tail*");
    }
}
```

**Marker Traits:**
```rust
// Send и Sync - маркерные трейты
fn spawn_thread<T: Send + 'static>(value: T) {
    std::thread::spawn(move || {
        // Используем value
    });
}

// Sized - еще один маркерный трейт
fn generic_function<T: Sized>(value: T) {
    // T имеет известный размер во время компиляции
}
```

**impl Trait (RPIT):**
```rust
// Return Position impl Trait
fn make_iterator() -> impl Iterator<Item = u32> {
    vec![1, 2, 3].into_iter()
}

// Можем вернуть разные типы, главное чтобы реализовали трейт
fn make_counter(which: bool) -> impl Iterator<Item = u32> {
    if which {
        vec![1, 2, 3].into_iter()
    } else {
        vec![4, 5, 6].into_iter()
    }
}
```

**APIT (Argument Position impl Trait):**
```rust
// Эквивалентно fn print<T: Display>(value: T)
fn print(value: impl Display) {
    println!("{}", value);
}

// С несколькими трейтами
fn process(item: impl Clone + Debug) {
    let copy = item.clone();
    println!("{:?}", copy);
}
```

**Specialization (unstable):**
```rust
#![feature(specialization)]

trait Example {
    fn process(&self) -> String;
}

impl<T> Example for T {
    default fn process(&self) -> String {
        "generic".to_string()
    }
}

impl Example for i32 {
    fn process(&self) -> String {
        format!("i32: {}", self)
    }
}
```

## Type-Level Programming Features

**PhantomData:**
```rust
use std::marker::PhantomData;

struct Slice<'a, T> {
    ptr: *const T,
    len: usize,
    _marker: PhantomData<&'a T>, // Указываем время жизни
}

// Пример с типом-маркером
struct Meters<T> {
    value: T,
    _unit: PhantomData<Meters<()>>,
}

struct Feet<T> {
    value: T,
    _unit: PhantomData<Feet<()>>,
}

// Теперь Meters и Feet - разные типы!
```

**Typestate Pattern:**
```rust
struct Locked;
struct Unlocked;

struct Door<State> {
    _state: PhantomData<State>,
}

impl Door<Locked> {
    fn new() -> Self {
        Door { _state: PhantomData }
    }
    
    fn unlock(self) -> Door<Unlocked> {
        Door { _state: PhantomData }
    }
}

impl Door<Unlocked> {
    fn lock(self) -> Door<Locked> {
        Door { _state: PhantomData }
    }
    
    fn open(&self) {
        println!("Door opened!");
    }
}

// Использование
let door = Door::<Locked>::new();
// door.open(); // Ошибка компиляции!
let door = door.unlock();
door.open(); // OK
```

**Zero-Sized Types:**
```rust
struct MyZst;

// Размер = 0
assert_eq!(std::mem::size_of::<MyZst>(), 0);

// Полезно для маркеров типов
struct Collection<T, S = MyZst> {
    items: Vec<T>,
    _strategy: PhantomData<S>,
}
```

## Lifetime Features

**Lifetime Parameters:**
```rust
struct RefWrapper<'a, T> {
    value: &'a T,
}

fn longest<'a>(x: &'a str, y: &'a str) -> &'a str {
    if x.len() > y.len() { x } else { y }
}
```

**Lifetime Bounds:**
```rust
struct Cache<'a, T: 'a> {
    data: &'a T,
}

fn store_reference<'a, T: 'a>(cache: &mut Cache<'a, T>, value: &'a T) {
    cache.data = value;
}
```

**Lifetime Elision:**
```rust
// Компилятор автоматически выводит времена жизни
fn first_word(s: &str) -> &str {
    // Эквивалентно: fn first_word<'a>(s: &'a str) -> &'a str
    s.split_whitespace().next().unwrap_or("")
}
```

**Variance:**
```rust
use std::marker::PhantomData;

// Ковариантность по 'a
struct Covariant<'a, T> {
    value: &'a T,
}

// Инвариантность по 'a
struct Invariant<'a, T> {
    value: &'a mut T,
}

// Контравариантность (редко)
struct Contravariant<'a, T> {
    func: fn(&'a T),
    _marker: PhantomData<T>,
}
```

## Advanced Type Constructs

**Never Type:**
```rust
fn diverges() -> ! {
    panic!("This never returns!");
}

fn example(condition: bool) -> i32 {
    if condition {
        42
    } else {
        diverges() // ! может приводиться к любому типу
    }
}
```

**DST (Dynamically Sized Types):**
```rust
// [T] и str - DST
fn print_slice<T: Debug>(slice: &[T]) {
    println!("{:?}", slice);
}

fn print_str(s: &str) {
    println!("{}", s);
}

// Трейт-объекты тоже DST
fn call_draw(drawable: &dyn Draw) {
    drawable.draw();
}
```

**?Sized Bound:**
```rust
// T может быть несизированным типом
fn generic<T: ?Sized>(value: &T) {
    // ...
}

// Без ?Sized компилятор требует T: Sized
fn generic_sized<T>(value: &T) {
    // T должен иметь известный размер
}
```

**Function Pointers:**
```rust
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn apply_operation(f: fn(i32, i32) -> i32, x: i32, y: i32) -> i32 {
    f(x, y)
}

let result = apply_operation(add, 5, 3);
```

**Closures:**
```rust
// Fn - может вызываться многократно без изменения окружения
fn apply_fn<F: Fn(i32) -> i32>(f: F, x: i32) -> i32 {
    f(x)
}

// FnMut - может изменять захваченные переменные
fn apply_fn_mut<F: FnMut(i32) -> i32>(mut f: F, x: i32) -> i32 {
    f(x)
}

// FnOnce - может быть вызвано только один раз
fn apply_fn_once<F: FnOnce(i32) -> i32>(f: F, x: i32) -> i32 {
    f(x)
}

// Использование
let y = 10;
let add_y = |x| x + y; // Fn
apply_fn(add_y, 5);

let mut counter = 0;
let mut increment = || { counter += 1; counter }; // FnMut
apply_fn_mut(&mut increment, 0);

let s = String::from("hello");
let consume = move || drop(s); // FnOnce
apply_fn_once(consume, 0);
```

## Type Aliases & Projections

**Type Aliases:**
```rust
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn read_file() -> Result<String> {
    Ok("content".to_string())
}
```

**Associated Type Projections:**
```rust
trait Graph {
    type Node;
    type Edge;
}

struct MyGraph;

impl Graph for MyGraph {
    type Node = u32;
    type Edge = (u32, u32);
}

// Использование проекции
fn process_node(node: <MyGraph as Graph>::Node) {
    println!("Node: {}", node);
}
```

**Qualified Paths:**
```rust
trait Animal {
    fn baby_name() -> String;
}

struct Dog;

impl Dog {
    fn baby_name() -> String {
        String::from("Spot")
    }
}

impl Animal for Dog {
    fn baby_name() -> String {
        String::from("puppy")
    }
}

// Нужен квалифицированный путь
let name = <Dog as Animal>::baby_name(); // "puppy"
let name = Dog::baby_name(); // "Spot"
```

## Const Features

**Const Functions:**
```rust
const fn square(n: i32) -> i32 {
    n * n
}

const VALUE: i32 = square(5); // Вычисляется во время компиляции

const fn fibonacci(n: u32) -> u32 {
    match n {
        0 => 0,
        1 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2),
    }
}
```

**Const Generics в действии:**
```rust
fn print_array<T: Debug, const N: usize>(arr: [T; N]) {
    for item in arr {
        println!("{:?}", item);
    }
}

print_array([1, 2, 3, 4, 5]);
print_array(["a", "b", "c"]);
```

## Type Inference & Coercion

**Turbofish Syntax:**
```rust
let numbers = vec![1, 2, 3];

// Явное указание типа через turbofish
let collected: Vec<_> = numbers.iter().collect::<Vec<_>>();
let parsed = "42".parse::<i32>().unwrap();

// Иногда необходимо
let v = (0..10).collect::<Vec<i32>>();
```

**Deref Coercion:**
```rust
use std::ops::Deref;

struct MyBox<T>(T);

impl<T> Deref for MyBox<T> {
    type Target = T;
    
    fn deref(&self) -> &T {
        &self.0
    }
}

fn print_str(s: &str) {
    println!("{}", s);
}

let my_string = MyBox(String::from("hello"));
print_str(&my_string); // MyBox<String> -> String -> &str
```

**Unsized Coercion:**
```rust
let array: [i32; 3] = [1, 2, 3];
let slice: &[i32] = &array; // [i32; 3] приводится к [i32]

let string = String::from("hello");
let str_slice: &str = &string; // String приводится к str
```

## Pattern Matching на уровне типов

**Sealed Traits:**
```rust
mod sealed {
    pub trait Sealed {}
    
    impl Sealed for i32 {}
    impl Sealed for String {}
}

// Публичный трейт, но реализовать снаружи модуля нельзя
pub trait MyTrait: sealed::Sealed {
    fn do_something(&self);
}

impl MyTrait for i32 {
    fn do_something(&self) {
        println!("i32: {}", self);
    }
}
```

**Extension Traits:**
```rust
trait SliceExt<T> {
    fn first_and_last(&self) -> Option<(&T, &T)>;
}

impl<T> SliceExt<T> for [T] {
    fn first_and_last(&self) -> Option<(&T, &T)> {
        if self.len() >= 2 {
            Some((&self[0], &self[self.len() - 1]))
        } else {
            None
        }
    }
}

// Использование
let arr = [1, 2, 3, 4, 5];
let (first, last) = arr.first_and_last().unwrap();
```

**Newtype Pattern:**
```rust
struct UserId(u64);
struct ProductId(u64);

fn get_user(id: UserId) -> String {
    format!("User {}", id.0)
}

// get_user(ProductId(123)); // Ошибка компиляции!
get_user(UserId(123)); // OK
```

**GADT-like patterns:**
```rust
enum Expression<T> {
    IntLiteral(i32),
    BoolLiteral(bool),
    Add(Box<Expression<i32>>, Box<Expression<i32>>),
    Equals(Box<Expression<T>>, Box<Expression<T>>),
}

// Можно использовать систему типов для гарантий
fn evaluate_int(expr: Expression<i32>) -> i32 {
    match expr {
        Expression::IntLiteral(n) => n,
        Expression::Add(a, b) => evaluate_int(*a) + evaluate_int(*b),
        _ => unreachable!(),
    }
}
```

## Exotic Features (Unstable/Nightly)

**Trait Aliases (unstable):**
```rust
#![feature(trait_alias)]

trait StringLike = AsRef<str> + Into<String>;

fn process<T: StringLike>(s: T) {
    println!("{}", s.as_ref());
}
```

**Inherent Associated Types (unstable):**
```rust
#![feature(inherent_associated_types)]

struct Foo;

impl Foo {
    type Bar = i32;
    
    fn get() -> Self::Bar {
        42
    }
}
```

**Arbitrary Self Types (unstable):**
```rust
#![feature(arbitrary_self_types)]

use std::rc::Rc;

struct MyType;

impl MyType {
    fn method(self: Rc<Self>) {
        // self - это Rc<MyType>
    }
}
```

---

## Полезные ресурсы

- [The Rust Reference - Type System](https://doc.rust-lang.org/reference/types.html)
- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- [The Rustonomicon](https://doc.rust-lang.org/nomicon/) - unsafe Rust и продвинутые концепции
- [The Little Book of Rust Macros](https://veykril.github.io/tlborm/)
