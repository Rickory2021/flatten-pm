<!-- docs/lessons/03-structs-impl-traits.md -->
# Rust Lesson 3: Structs, Impl Blocks, and Traits

## Structs: Rust's data containers

A struct is a named collection of fields. If you've written C, you already know what a struct is. If you've written Java or Python, think of it as a class with only data, no inheritance.

```rust
struct Profile {
    name: String,
    root_dir: String,
    include_patterns: Vec<String>,
    exclude_patterns: Vec<String>,
}
```

### How it maps across languages

```c
// C: almost identical
struct Profile {
    char *name;
    char *root_dir;
    // no dynamic arrays without manual work
};
```

```java
// Java: a class with fields
public class Profile {
    private String name;
    private String rootDir;
    private List<String> includePatterns;
    private List<String> excludePatterns;
    // constructor, getters, setters, equals, hashCode, toString...
}
```

```python
# Python: dataclass
@dataclass
class Profile:
    name: str
    root_dir: str
    include_patterns: list[str]
    exclude_patterns: list[str]
```

```typescript
// TS: interface or class
interface Profile {
    name: string;
    rootDir: string;
    includePatterns: string[];
    excludePatterns: string[];
}
```

Rust structs are closest to C structs in memory layout (fields are stored inline, no object header, no vtable pointer) and closest to Python dataclasses or TS interfaces in ergonomics.

### Creating instances

```rust
let profile = Profile {
    name: String::from("my-project"),
    root_dir: String::from("/home/ricky/project"),
    include_patterns: vec![String::from("src/**")],
    exclude_patterns: vec![String::from("node_modules")],
};
```

No `new` keyword. No constructor call. You just fill in the fields. (You can write a constructor function by convention; we'll get to that.)

### Accessing fields

```rust
println!("{}", profile.name);           // dot access, like every language
println!("{}", profile.root_dir);
```

### Ownership applies to fields

This is where Rust diverges from everything else. Each field is owned by the struct. When the struct is dropped, all its fields are dropped:

```rust
{
    let profile = Profile { /* ... */ };
    // profile owns name, root_dir, include_patterns, exclude_patterns
}   // profile dropped here; all four Strings and the Vec are freed
```

In C, you'd need to manually free each field. In Java/Python, the GC handles it. In Rust, ownership propagates: the struct owns its fields, scope owns the struct, end of scope frees everything. Recursive, automatic, deterministic.

### Tuple structs and unit structs

Two less common forms:

```rust
// Tuple struct: fields by position, not name
struct Color(u8, u8, u8);
let red = Color(255, 0, 0);
println!("{}", red.0);  // access by index

// Unit struct: no fields, used as a marker or type-level tag
struct Placeholder;
```

Tuple structs are useful for newtypes (wrapping a single value to give it a distinct type). Unit structs show up in trait implementations. You'll see both, but named structs are the default.

## Impl blocks: adding behavior to structs

In Java/Python, methods live inside the class definition. In Rust, data (struct) and behavior (impl) are separate:

```rust
struct Profile {
    name: String,
    root_dir: String,
    include_patterns: Vec<String>,
    exclude_patterns: Vec<String>,
}

impl Profile {
    // Associated function (no self): like a static method / constructor
    fn new(name: String, root_dir: String) -> Self {
        Profile {
            name,
            root_dir,
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
        }
    }

    // Method (takes &self): borrows the struct immutably
    fn file_count(&self) -> usize {
        self.include_patterns.len()
    }

    // Method (takes &mut self): borrows the struct mutably
    fn add_include(&mut self, pattern: String) {
        self.include_patterns.push(pattern);
    }

    // Method (takes self): consumes/moves the struct
    fn into_name(self) -> String {
        self.name  // ownership of name moves out; rest of struct is dropped
    }
}
```

### The self parameter

This is the Rust version of Java's `this` or Python's `self`, but explicit about ownership:

| Signature | Meaning | Analogy |
|-----------|---------|---------|
| `fn method(&self)` | Borrows immutably, read-only access | Java's normal method (but actually enforced as read-only) |
| `fn method(&mut self)` | Borrows mutably, can modify fields | Java's setter/mutating method |
| `fn method(self)` | Takes ownership, struct consumed after call | No direct analog; closest is a builder's `.build()` that consumes the builder |
| `fn associated()` (no self) | No instance needed, called on the type | Java's `static` method, Python's `@staticmethod` |

In Java, every method implicitly has mutable access to `this`. There's no way to say "this method promises not to modify the object." In Rust, `&self` vs `&mut self` makes that promise explicit and enforced.

### Calling methods and associated functions

```rust
// Associated function: called on the TYPE with ::
let mut profile = Profile::new(
    String::from("my-project"),
    String::from("/home/ricky/project"),
);

// Methods: called on the INSTANCE with .
profile.add_include(String::from("src/**"));
let count = profile.file_count();
println!("Patterns: {}", count);  // 1

// Consuming method: profile is moved, can't use it after
let name = profile.into_name();
// profile is now invalid
```

`::` for associated functions (like `Profile::new`), `.` for methods (like `profile.file_count()`). Same distinction as Java's `ClassName.staticMethod()` vs `instance.method()`.

### The `new` convention

Rust has no constructors. `Profile::new()` is just a convention: an associated function named `new` that returns `Self`. The compiler doesn't treat it specially. You could name it `create` or `build` or anything. The community convention is `new` for the primary constructor.

```rust
// These are equivalent. new() is just convention.
let p1 = Profile::new(name, root_dir);
let p2 = Profile { name, root_dir, include_patterns: vec![], exclude_patterns: vec![] };
```

### Multiple impl blocks

You can split impl blocks across the file or even across modules. This is unusual coming from Java/Python where everything goes in one class body:

```rust
// Core construction and data access
impl Profile {
    fn new(name: String, root_dir: String) -> Self { /* ... */ }
    fn file_count(&self) -> usize { /* ... */ }
}

// Export-related behavior
impl Profile {
    fn export(&self) -> Result<ExportResult, ExportError> { /* ... */ }
}
```

Why you'd do this: organization, conditional compilation, or when trait implementations (next section) are in separate files. It's not common for basic code but becomes useful as a codebase grows.

## Traits: Rust's version of interfaces

Traits define shared behavior. If you know Java interfaces, you're 80% there. If you know Python's ABCs or TS interfaces, same ballpark.

```rust
trait Scannable {
    fn scan(&self) -> Vec<ScanFinding>;

    // Default implementation (optional)
    fn has_findings(&self) -> bool {
        !self.scan().is_empty()
    }
}
```

A trait says: "any type that implements me must provide these methods." Types opt in explicitly:

```rust
struct FileContent {
    path: String,
    content: String,
}

impl Scannable for FileContent {
    fn scan(&self) -> Vec<ScanFinding> {
        // scan self.content for secrets/PII
        vec![]  // placeholder
    }
    // has_findings() comes free from the default implementation
}
```

### How traits compare across languages

| Feature | C | Java | Python | TS | Rust |
|---------|---|------|--------|------|------|
| Mechanism | Function pointer tables (manual vtable) | `interface` / `abstract class` | ABC / duck typing | `interface` | `trait` |
| Explicit opt-in | N/A | `implements` | `class Foo(ABC)` / nothing for duck typing | `implements` (classes) / structural | `impl Trait for Type` |
| Default methods | N/A | Since Java 8 | Yes (in ABC) | No (interfaces are structural) | Yes |
| Multiple | Many vtables | Multiple interfaces | Multiple ABCs + duck typing | Multiple interfaces | Multiple traits |
| Inheritance | N/A | Single class + multiple interfaces | Multiple (diamond problem) | N/A | No struct inheritance, ever |
| Compile-time or runtime | Runtime (function pointers) | Runtime (vtable dispatch) | Runtime (duck typing) | Compile-time (erased) | **Both** (monomorphized default, `dyn Trait` for runtime) |

The key difference from Java interfaces: Rust traits are resolved at compile time by default. When you call `file.scan()`, the compiler knows the concrete type and inlines the call. No vtable lookup, no indirection. Zero-cost abstraction.

### Trait bounds: constraining generics

This is where traits become powerful. You can say "this function works on any type, as long as it implements this trait":

```rust
fn report_findings<T: Scannable>(item: &T) {
    let findings = item.scan();
    for f in &findings {
        println!("Finding: {:?}", f);
    }
}
```

`T: Scannable` is a **trait bound**. It says: "T can be any type, but it must implement `Scannable`." The compiler generates a specialized version of the function for each concrete type you call it with (monomorphization). Zero runtime cost.

Java equivalent:

```java
<T extends Scannable> void reportFindings(T item) {
    List<ScanFinding> findings = item.scan();
    // ...
}
```

Same concept, but Java uses runtime dispatch (vtable). Rust generates a separate compiled function per type (like C++ templates, but with the type safety of Java generics).

### The `where` clause (same thing, cleaner syntax)

When trait bounds get complex, move them to a `where` clause:

```rust
// These are identical:
fn process<T: Scannable + Serialize>(item: &T) { /* ... */ }

fn process<T>(item: &T)
where
    T: Scannable + Serialize,
{ /* ... */ }
```

The `where` form is preferred when bounds are long or there are multiple type parameters. Readability choice.

### Common standard library traits you'll use

Rust's standard library defines traits for common behavior. You'll see these constantly:

```rust
#[derive(Debug, Clone, PartialEq)]
struct ScanFinding {
    line: usize,
    severity: Severity,
    message: String,
}
```

`#[derive(...)]` is a macro that auto-implements traits:

| Trait | What it does | Equivalent in other languages |
|-------|-------------|-------------------------------|
| `Debug` | Format for debugging (`{:?}` in println) | Java's `toString()`, Python's `__repr__` |
| `Clone` | Explicit deep copy (`.clone()`) | Java's `.clone()`, Python's `copy.deepcopy()` |
| `PartialEq` | Equality comparison (`==`) | Java's `.equals()`, Python's `__eq__` |
| `Eq` | Full equality (extends `PartialEq`, adds reflexivity guarantee) | Marker trait, no extra methods |
| `Hash` | Hashable (for use as HashMap key) | Java's `hashCode()`, Python's `__hash__` |
| `Display` | Human-readable formatting (`{}` in println) | Java's `toString()` for users, Python's `__str__` |
| `Default` | Provides a default value (`.default()`) | Python's default class attributes |
| `Serialize` / `Deserialize` | Serde serialization (not std, but ubiquitous) | Python's Pydantic models, Java's Jackson annotations |

`derive` saves you from writing boilerplate. In Java, you'd write `equals()`, `hashCode()`, `toString()` by hand (or use Lombok). In Python, `@dataclass` auto-generates similar methods. Rust's `derive` is the same idea.

### Display vs Debug

This trips people up. Two different formatting traits:

```rust
#[derive(Debug)]
struct Point {
    x: f64,
    y: f64,
}

// Debug: auto-derived, for developers
let p = Point { x: 1.0, y: 2.0 };
println!("{:?}", p);   // Point { x: 1.0, y: 2.0 }

// Display: must be manually implemented, for users
use std::fmt;
impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}
println!("{}", p);     // (1.0, 2.0)
```

`Debug` (`{:?}`) is derivable and shows the struct's internal shape. `Display` (`{}`) is for human-readable output and must be implemented by hand because the compiler can't know how you want to present data to users.

Python parallel: `Debug` is `__repr__`, `Display` is `__str__`.

## No inheritance. Ever.

This is the biggest mental shift from Java. Rust has **no struct inheritance**. You cannot do this:

```rust
// THIS DOES NOT EXIST IN RUST
struct Animal { name: String }
struct Dog extends Animal { breed: String }  // not a thing
```

Java's world is built on class hierarchies. Rust rejects that model entirely. The alternatives:

### Composition over inheritance

```rust
struct Animal {
    name: String,
}

struct Dog {
    animal: Animal,       // HAS-A, not IS-A
    breed: String,
}

impl Dog {
    fn name(&self) -> &str {
        &self.animal.name  // delegate to inner struct
    }
}
```

This is what the Gang of Four book recommended all along. Rust just doesn't give you the option to cheat.

### Traits for shared behavior (polymorphism without inheritance)

```rust
trait Speaks {
    fn speak(&self) -> String;
}

struct Cat { name: String }
struct Dog { name: String, breed: String }

impl Speaks for Cat {
    fn speak(&self) -> String {
        format!("{} says meow", self.name)
    }
}

impl Speaks for Dog {
    fn speak(&self) -> String {
        format!("{} says woof", self.name)
    }
}
```

No shared base class. No hierarchy. Each type independently implements the trait. The compiler checks that each implementation satisfies the contract.

### Static dispatch (default, zero cost)

```rust
fn announce<T: Speaks>(animal: &T) {
    println!("{}", animal.speak());
}

announce(&cat);  // compiler generates announce_Cat at compile time
announce(&dog);  // compiler generates announce_Dog at compile time
```

The compiler generates separate machine code for each type. No vtable, no indirection. This is monomorphization, like C++ templates but type-safe.

### Dynamic dispatch (when you need it)

When you need a collection of different types that share a trait, or you want to avoid monomorphization code bloat:

```rust
fn announce_all(animals: &[&dyn Speaks]) {
    for animal in animals {
        println!("{}", animal.speak());
    }
}

let cat = Cat { name: String::from("Whiskers") };
let dog = Dog { name: String::from("Rex"), breed: String::from("Corgi") };
announce_all(&[&cat, &dog]);
```

`dyn Speaks` means "some type that implements Speaks, determined at runtime." This uses a vtable, same as Java's interface dispatch. You pay for the indirection only when you ask for it.

| Dispatch | Syntax | Cost | When to use |
|----------|--------|------|-------------|
| Static | `fn foo<T: Trait>(x: &T)` | Zero (inlined) | Default. When you know the type at compile time. |
| Dynamic | `fn foo(x: &dyn Trait)` | Vtable lookup | When you need heterogeneous collections or to reduce binary size. |

In Java, every method call is dynamic dispatch (virtual by default). In Rust, you choose. Static is the default; dynamic is opt-in when you need it.

## Connecting to flatten-core

Here's how these concepts land in the codebase you're building:

**Structs for data models:**
- `Profile` (export config: name, root dir, patterns, options)
- `ExportResult` (list of exported files, manifest, stats)
- `ScanFinding` (line number, severity, message, rule ID)
- `WatchEvent` (file path, event type, timestamp)

**Impl blocks for behavior:**
- `Profile::new()`, `Profile::from_yaml()`, `Profile::export()`
- `ScanFinding::is_high_severity()`
- `WatchEvent::needs_quarantine()`

**Traits for shared contracts:**
- A `Scannable` trait if multiple content types need scanning
- A `Placeable` trait for the detection cascade (directory comment, JSON path, embedded path)
- Standard derives everywhere: `Debug`, `Clone`, `PartialEq`, `Serialize`, `Deserialize`

**Composition over inheritance:**
- `Profile` contains a `ScanConfig` struct, a `ModifierCascade` struct, etc.
- No base classes. Shared behavior via traits.

## Summary

| Concept | Java | Python | Rust |
|---------|------|--------|------|
| Data container | Class (data + behavior + inheritance) | Dataclass / class | Struct (data only) |
| Methods | Inside the class body | Inside the class body | In a separate `impl` block |
| Constructor | `new ClassName()` (special language construct) | `__init__` (dunder) | `Type::new()` (convention, just a function) |
| Interface / contract | `interface` (runtime dispatch) | ABC / duck typing | `trait` (compile-time default, runtime opt-in) |
| Inheritance | Single class + multiple interfaces | Multiple inheritance | None. Composition + traits. |
| Polymorphism | Subtype via class hierarchy | Duck typing | Trait bounds (static) or `dyn Trait` (dynamic) |
| Auto-generated methods | Lombok / IDE generate | `@dataclass` | `#[derive(...)]` |
| Method access to instance | Implicit `this`, always mutable | Explicit `self`, always mutable | Explicit `self` with ownership level: `&self`, `&mut self`, `self` |
