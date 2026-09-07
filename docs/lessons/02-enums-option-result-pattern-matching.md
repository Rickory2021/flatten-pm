<!-- docs/lessons/02-enums-option-result-pattern-matching.md -->
# Rust Lesson 2: Enums, Option, Result, and Pattern Matching

## Enums: not your Java/C enums

In C and Java, enums are just named integers:

```c
// C
enum Color { RED, GREEN, BLUE };  // RED = 0, GREEN = 1, BLUE = 2
```

```java
// Java
enum Color { RED, GREEN, BLUE }   // slightly fancier, but still a fixed set of labels
```

Rust enums can do that too, but they go much further. Each variant can **carry data**:

```rust
enum IpAddress {
    V4(u8, u8, u8, u8),        // carries four bytes
    V6(String),                 // carries a heap-allocated string
}

let home = IpAddress::V4(127, 0, 0, 1);
let loopback = IpAddress::V6(String::from("::1"));
```

One type, two variants, each holding different data. There's no equivalent in C or Java. The closest comparison:

| Language | How you'd model this | Tradeoff |
|----------|---------------------|----------|
| C | Tagged union (`struct` with an `enum` tag + `union` body) | Manual, error-prone, no compiler help |
| Java | Abstract class with subclasses (`Ipv4 extends IpAddress`) | Heap allocation, boilerplate, open to extension (anyone can subclass) |
| Python | Dataclass with a type field, or separate classes | No exhaustiveness checking, duck typing |
| TS | Discriminated union (`{ kind: "v4", ... } \| { kind: "v6", ... }`) | Closest analog; TS checks exhaustiveness with `never` |
| **Rust** | **Enum with data variants** | **Compiler-enforced exhaustiveness, zero overhead** |

TypeScript's discriminated unions are the closest mental model. If you've used those, Rust enums are the same idea but stricter and with zero runtime cost.

### Memory layout

At the assembly level, a Rust enum is a tagged union, the same as C's `struct { enum tag; union { ... } data; }`. The tag is typically 1 byte (or optimized away entirely in some cases). No heap allocation, no vtable, no indirection. Just a tag and inline data on the stack.

## Pattern matching with `match`

`match` is how you destructure enums. Think of it as a `switch` statement that can unpack data and that the compiler checks for completeness.

```rust
fn describe(addr: &IpAddress) -> String {
    match addr {
        IpAddress::V4(a, b, c, d) => format!("{}.{}.{}.{}", a, b, c, d),
        IpAddress::V6(s) => format!("IPv6: {}", s),
    }
}
```

**What's happening:** the `match` checks which variant `addr` is, destructures the inner data into local variables (`a, b, c, d` or `s`), and runs the corresponding arm.

### Exhaustiveness: the compiler forces you to handle every case

```rust
fn describe(addr: &IpAddress) -> String {
    match addr {
        IpAddress::V4(a, b, c, d) => format!("{}.{}.{}.{}", a, b, c, d),
        // COMPILE ERROR: non-exhaustive patterns, `V6(_)` not covered
    }
}
```

In C, a `switch` on an enum without a `default` gives you a warning at best. In Java, same story. In Rust, it's a hard compile error. You must handle every variant.

This matters enormously. When you add a new variant to an enum, the compiler shows you every `match` that needs updating. In Java/Python, adding a subclass or enum value means grepping and hoping you found every `if`/`switch` that cares.

### The wildcard pattern

If you genuinely don't care about some variants:

```rust
match addr {
    IpAddress::V4(a, b, c, d) => format!("{}.{}.{}.{}", a, b, c, d),
    _ => String::from("not IPv4"),  // catches everything else
}
```

`_` is the wildcard. Use it when you intentionally want to ignore remaining cases, not as a lazy default.

## Option\<T\>: Rust's replacement for null

This is one of the most important types in Rust. There is no `null`, `nil`, `None` (in the Python sense), or `undefined` in Rust. Instead, the possibility of absence is encoded in the type system:

```rust
enum Option<T> {
    Some(T),    // there is a value of type T
    None,       // there is no value
}
```

That's it. `Option` is just an enum with two variants. It's in the standard library and used everywhere.

### How each language handles "might not exist"

| Language | Mechanism | Can you forget to check? | Compile-time safety |
|----------|-----------|-------------------------|-------------------|
| C | `NULL` pointer | Yes, and it's a segfault | No |
| Java | `null` reference | Yes, and it's a `NullPointerException` | No (unless you use `Optional`) |
| Python | `None` | Yes, and it's an `AttributeError` at runtime | No |
| TS | `undefined` / `null` | With `strictNullChecks`, partially | Partial |
| **Rust** | **`Option<T>`** | **No. The compiler forces you to handle it.** | **Yes, always** |

TypeScript with `strictNullChecks` is the closest; if a value is `string | undefined`, you must narrow before using it as `string`. Rust's `Option<T>` is that pattern applied universally, with no escape hatch.

### Using Option

```rust
fn find_user(id: u32) -> Option<String> {
    if id == 1 {
        Some(String::from("Ricky"))
    } else {
        None
    }
}

fn main() {
    let result = find_user(1);

    // You CANNOT do this:
    // println!("{}", result);  // COMPILE ERROR: Option<String> doesn't implement Display

    // You MUST handle both cases:
    match result {
        Some(name) => println!("Found: {}", name),
        None => println!("User not found"),
    }
}
```

The compiler won't let you use an `Option<String>` as a `String`. You must explicitly unpack it and handle the `None` case. Every. Single. Time.

In Java, `String findUser(int id)` returns a `String` that might be `null`, and nothing in the type signature tells you. In Rust, `Option<String>` in the return type makes the possibility of absence visible and enforced.

### Common Option methods (shortcuts for match)

You'll see these constantly:

```rust
let name = find_user(1);

// unwrap: get the value or PANIC (crash) if None
// Use only when you're certain it's Some, or in quick prototyping
let n = name.unwrap();

// unwrap_or: get the value or use a default
let n = name.unwrap_or(String::from("Unknown"));

// unwrap_or_else: get the value or compute a default (lazy)
let n = name.unwrap_or_else(|| String::from("computed default"));

// is_some / is_none: boolean check
if name.is_some() { /* ... */ }

// map: transform the inner value if it exists
let upper: Option<String> = name.map(|n| n.to_uppercase());

// and_then: chain operations that also return Option (like flatMap)
let greeting: Option<String> = name.and_then(|n| {
    if n.is_empty() { None } else { Some(format!("Hello, {}!", n)) }
});
```

`map` and `and_then` will feel familiar from Java streams, Python's `map()`, or TS array methods. Same concept: transform the value inside the wrapper without unwrapping it manually.

### if let: pattern match for a single variant

When you only care about one case:

```rust
// Instead of:
match find_user(1) {
    Some(name) => println!("Found: {}", name),
    None => {},  // do nothing
}

// You can write:
if let Some(name) = find_user(1) {
    println!("Found: {}", name);
}
```

Reads as: "if this pattern matches, bind the inner value and run the block." Cleaner than a full `match` when you only care about one variant.

## Result\<T, E\>: Rust's replacement for exceptions

Rust has no `try`/`catch`. No exceptions. No `throws` declarations. Instead, operations that can fail return a `Result`:

```rust
enum Result<T, E> {
    Ok(T),     // success, carrying a value of type T
    Err(E),    // failure, carrying an error of type E
}
```

Again, just an enum. Two variants. The type system forces you to handle both.

### How each language handles errors

| Language | Mechanism | Can you forget to handle it? |
|----------|-----------|------------------------------|
| C | Return codes (`-1`, `errno`) | Yes, trivially (just ignore the return value) |
| Java | Checked exceptions | Theoretically no, but `catch (Exception e) {}` is rampant |
| Java | Unchecked exceptions (`RuntimeException`) | Yes, they propagate silently |
| Python | Exceptions | Yes, unhandled exceptions crash at runtime |
| TS/JS | Exceptions (thrown, untyped) | Yes, nothing in the type system tracks them |
| **Rust** | **`Result<T, E>`** | **No. Compile error if you ignore it.** |

Java's checked exceptions tried to solve the same problem: force the caller to deal with errors. But the escape hatches (catch-all, unchecked exceptions, `throws Exception`) made it optional in practice. Rust's `Result` has no escape hatch. You either handle it or the code doesn't compile (the compiler warns on unused `Result` values).

### Using Result

```rust
use std::fs;

fn main() {
    let content: Result<String, std::io::Error> = fs::read_to_string("config.txt");

    match content {
        Ok(text) => println!("File contents: {}", text),
        Err(e) => println!("Failed to read file: {}", e),
    }
}
```

`fs::read_to_string` returns `Result<String, io::Error>`. The type signature tells you: this can fail, and if it does, you get an `io::Error`. No surprises, no unchecked exceptions flying out of nowhere.

### The ? operator: error propagation

Writing `match` for every Result is verbose. The `?` operator is syntactic sugar for "if this is `Err`, return the error immediately; if `Ok`, unwrap the value":

```rust
use std::fs;
use std::io;

fn read_config() -> Result<String, io::Error> {
    let content = fs::read_to_string("config.txt")?;  // ? propagates error
    Ok(content.to_uppercase())
}
```

What `?` desugars to:

```rust
fn read_config() -> Result<String, io::Error> {
    let content = match fs::read_to_string("config.txt") {
        Ok(val) => val,
        Err(e) => return Err(e),   // early return with the error
    };
    Ok(content.to_uppercase())
}
```

**The rule:** `?` can only be used in functions that return `Result` (or `Option`). The function's return type must be compatible with the error being propagated.

You can chain `?` for clean, readable error propagation:

```rust
fn load_and_parse_config() -> Result<Config, AppError> {
    let raw = fs::read_to_string("config.txt")?;    // might fail (io error)
    let parsed = parse_config(&raw)?;                 // might fail (parse error)
    let validated = validate(parsed)?;                // might fail (validation error)
    Ok(validated)
}
```

Compare this to Java:

```java
Config loadAndParseConfig() throws IOException, ParseException, ValidationException {
    String raw = Files.readString(Path.of("config.txt"));
    Config parsed = parseConfig(raw);
    Config validated = validate(parsed);
    return validated;
}
```

Similar flow, but Rust's error path is explicit in the return type (`Result<Config, AppError>`), not in a `throws` declaration that callers routinely ignore. And there's no invisible stack unwinding; `?` is a normal return, same cost as `return Err(e)`.

### C comparison

In C, you'd check error codes after every call:

```c
char *raw = read_file("config.txt");
if (raw == NULL) return -1;

Config *parsed = parse_config(raw);
if (parsed == NULL) { free(raw); return -1; }

Config *validated = validate(parsed);
if (validated == NULL) { free(parsed); free(raw); return -1; }
```

Rust's `?` operator gives you the same early-return-on-error pattern, but without the manual cleanup (ownership handles the frees) and without the risk of forgetting to check a return code.

## Option and Result are the same pattern

Notice the symmetry:

| | Has a value | No value |
|--|------------|----------|
| **Option\<T\>** | `Some(T)` | `None` |
| **Result\<T, E\>** | `Ok(T)` | `Err(E)` |

`Option` is for "might not exist." `Result` is for "might fail, and here's why." Both are enums. Both use `match`. Both support `?`, `map`, `and_then`, `unwrap`, and the same family of combinators.

If a Python function returns `value_or_None`, that's `Option`. If it `raise`s an exception, that's `Result`. Rust just makes both explicit in the type system instead of implicit in the runtime behavior.

## Connecting to flatten-core

When you start building the export pipeline, Result and Option will be everywhere:

- **Reading files:** `fs::read_to_string()` returns `Result<String, io::Error>`. Every file read is a potential failure.
- **Finding directory comments:** scanning lines for a comment pattern returns `Option<String>`. The comment might not be there; that's a quarantine condition.
- **Parsing modifier configs:** YAML/TOML parsing returns `Result`. Config might be malformed.
- **Walking directories:** each entry in a directory walk can fail (permissions, broken symlinks). The iterator yields `Result<DirEntry, Error>`.
- **Content safety scanning:** regex matches return `Option<Match>`. A finding might or might not exist.

The `?` operator will be your main tool for threading errors upward through the pipeline without boilerplate. You'll define an `AppError` or similar enum that unifies the various error types (IO, parse, config, scan) so `?` works across different failure modes.

## Summary table

| Concept | C | Java | Python | TS | Rust |
|---------|---|------|--------|------|------|
| Enum with data | Tagged union (manual) | Sealed class hierarchy | Dataclass + type field | Discriminated union | `enum` with variants |
| Exhaustive check | No | No (unless sealed + when in Kotlin) | No | Partial (with `never`) | Yes, always |
| Null/absence | `NULL` pointer | `null` reference | `None` | `undefined`/`null` | `Option<T>` |
| Null safety | None | None (Optional is opt-in) | None | `strictNullChecks` (partial) | Total (no null exists) |
| Error handling | Return codes | Exceptions (checked + unchecked) | Exceptions | Exceptions (untyped) | `Result<T, E>` |
| Error propagation | Manual `if` checks | `throws` / `try-catch` | `try-except` | `try-catch` | `?` operator |
| Forgetting to handle errors | Silent (ignored return code) | Silent (unchecked exceptions) | Runtime crash | Runtime crash | Compile error |
