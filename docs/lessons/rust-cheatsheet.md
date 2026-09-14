<!-- docs/lessons/rust-cheatsheet.md -->
# Rust Cheat Sheet

Accumulated Rust concepts from Flatten PM implementation sessions.
Organized by topic. Append new sections as they come up.

## Table of Contents

**Fundamentals**
1.  [References and Dereferencing](#references-and-dereferencing)
2.  [String vs str](#string-vs-str)
3.  [Slices: &[T] vs Vec<T> vs [T; N]](#slices-t-vs-vect-vs-t-n)
4.  [Type Aliases](#type-aliases)
5.  [Expression Returns (No Semicolon)](#expression-returns-no-semicolon)
6.  [if let Pattern Matching](#if-let-pattern-matching)
7.  [Discard Pattern _](#discard-pattern-_)
8.  [Raw String Literals](#raw-string-literals)

**Ownership and Lifetimes**
9.  [Lifetimes and '_](#lifetimes-and-_)
10. [Heap Allocation: Box vs Rc vs Arc](#heap-allocation-box-vs-rc-vs-arc)
11. [Clone and Ownership Across Threads](#clone-and-ownership-across-threads)

**Functions and Closures**
12. [Closures](#closures)
13. [Closure Traits: Fn, FnMut, FnOnce](#closure-traits-fn-fnmut-fnonce)
14. [Generics and Where Clauses](#generics-and-where-clauses)
15. [dyn -- Dynamic Dispatch](#dyn----dynamic-dispatch)

**Error Handling**
16. [Error Handling](#error-handling)
17. [thiserror](#thiserror)
18. [catch_unwind](#catch_unwind)

**Concurrency**
19. [Threads](#threads)
20. [Send and static](#send-and-static)
21. [Channels (mpsc)](#channels-mpsc)

**Module System**
22. [Module System](#module-system)
23. [use Statements](#use-statements)
24. [Visibility](#visibility)

**Tooling**
25. [Cargo](#cargo)
26. [const and Compile-Time Evaluation](#const-and-compile-time-evaluation)
27. [Test Organization](#test-organization)

**Flatten PM Patterns**
28. [The Type Erasure Trick](#the-type-erasure-trick)
29. [Option take Pattern](#option-take-pattern)
30. [Deref Coercion](#deref-coercion)
31. [call vs call_write Signature Split](#call-vs-call_write-signature-split)

32. [Quick Reference Table](#quick-reference-table)

---

# Fundamentals

---

## References and Dereferencing

`&` and `*` are a pair. `&` creates a reference (address). `*` follows it
(dereferences).

```rust
let x: i32 = 42;
let r: &i32 = &x;     // & = "give me the address of x"
let val: i32 = *r;     // * = "follow the address, get the value"
```

Same symbols as C:

```c
int x = 42;
int *r = &x;      // & = address of
int val = *r;      // * = dereference
```

Difference: C's pointers are unchecked. Rust's references are validated
by the borrow checker at compile time. No dangling references, no
use-after-free.

In the writer code, `*tx` dereferences a Transaction to its inner
Connection via the Deref trait:

```rust
let tx = conn.transaction()?;
// *tx   -> Connection (via Deref)
// &tx   -> &Transaction, auto-coerces to &Connection (via Deref)
// &*tx  -> &Connection (explicit dereference then re-borrow)
```

Python has no equivalent because Python dereferences automatically
everywhere. Every Python variable is already a reference.

---

## String vs str

`String` is owned data on the heap. `&str` is a read-only view into string data.

```rust
let owned: String = String::from("Ricky");  // owns the bytes, heap-allocated
let view: &str = &owned;                     // borrows, just a pointer + length
let literal: &str = "hello";                 // points to data in the binary
```

```
String    = you own the book. You can write in it.
&str      = you're reading someone's book. Look, don't touch.
"literal" = text on the wall. Everyone reads it. Nobody owns it.
```

Functions that only need to read a string should take `&str`. This accepts
both `String` (via auto-deref) and string literals:

```rust
fn greet(name: &str) { println!("Hello {}", name); }

greet(&owned);     // String -> &str automatically
greet("world");    // &str directly
```

`.into()` converts `&str` to `String` (copies to heap):

```rust
Error::Writer("writer thread is gone".into())
//             ^^^^^^^^^^^^^^^^^^^^^^  ^^^^^
//             &str literal            .into() -> String
```

---

## Slices: &[T] vs Vec<T> vs [T; N]

Three ways to hold contiguous data:

```rust
[T; N]     // array: fixed size, stack, known at compile time
Vec<T>     // vector: growable, heap, owned
&[T]       // slice: borrowed view, any length, read-only window
```

```rust
let array: [i32; 3] = [1, 2, 3];       // fixed size, stack
let vec: Vec<i32> = vec![1, 2, 3];      // growable, heap
let slice: &[i32] = &vec;               // borrows vec's data
let slice2: &[i32] = &array;            // borrows array's data
```

Functions that only read should take `&[T]` (accepts both Vec and array):

```rust
fn sum(numbers: &[i32]) -> i32 { numbers.iter().sum() }
sum(&vec);       // Vec -> &[i32] automatically
sum(&array);     // [i32; 3] -> &[i32] automatically
```

`from_slice(&[M::up(...)])` creates an array literal, takes a reference
(promoting it to static in const context), and passes the slice.

---

## Type Aliases

Give a short name to a complex type.

```rust
// Standard library pattern:
pub type Result<T> = std::result::Result<T, Error>;

// Custom type alias:
type Job = Box<dyn FnOnce(&mut rusqlite::Connection) + Send>;
```

No new type is created. It's just a name. The compiler treats them identically.

---

## Expression Returns (No Semicolon)

The last expression in a function without a semicolon is the return value.
Semicolons suppress the return.

```rust
// No semicolon: returns the value
fn add(x: i32, y: i32) -> i32 {
    x + y        // this is the return value
}

// Semicolon: returns nothing ()
fn add(x: i32, y: i32) -> i32 {
    x + y;       // value thrown away, returns ()
}                // COMPILER ERROR: expected i32, got ()
```

`return` keyword exists but is reserved for early exits:

```rust
fn divide(x: i32, y: i32) -> Result<i32> {
    if y == 0 {
        return Err(Error::DivideByZero);  // early exit needs return
    }
    Ok(x / y)    // last expression, no return keyword needed
}
```

`let` is a statement, not an expression. It always requires a semicolon:

```rust
fn add(x: i32, y: i32) -> i32 {
    let result = x + y;    // statement, needs semicolon
    result                  // expression, this is the return value
}
```

---

## if let Pattern Matching

Combines pattern matching with a conditional. "If this value matches
this shape, extract the inner data and run this block."

```rust
if let Some(handle) = self.handle.take() {
//     ^^^^^^^^^^^^   ^^^^^^^^^^^^^^^^^^
//     pattern         value to check
    handle.join();    // handle is the extracted inner value
}
```

Uses `=` (pattern bind), not `==` (comparison). These are different
operations:

```rust
if x == 5 { }                        // comparison: are they equal?
if let Some(handle) = option { }     // pattern: does it match this shape?
```

Shorthand for match when you only care about one variant:

```rust
// if let (shorter):
if let Some(handle) = self.handle.take() {
    handle.join();
}

// match (longer, same behavior):
match self.handle.take() {
    Some(handle) => { handle.join(); }
    None => { }
}
```

The variable declared in the pattern (`handle`) only exists inside the block.

Python's walrus operator `:=` is similar but weaker (assigns and checks,
doesn't destructure):

```python
if (match := re.search(pattern, text)) is not None:
    print(match.group())
```

---

## Discard Pattern _

`_` is a compiler-enforced discard. Not a variable. The value is dropped
immediately. You cannot use it afterward.

```rust
let (reply_tx, _) = mpsc::sync_channel(0);
// reply_rx is gone, dropped immediately

_.recv()   // COMPILER ERROR: _ is not a binding
```

Three options for unwanted values:

```rust
let (tx, _) = channel();        // discard, dropped, gone
let (tx, _rx) = channel();      // kept alive, no unused-variable warning
let (tx, rx) = channel();       // kept alive, compiler warns if unused
```

Different from Python: Python's `_` is a regular variable. You can use it.
Rust's `_` is enforced. The value is genuinely gone.

`let _ = handle.join();` is idiomatic for "call this but I don't care
about the return value." The return value is discarded.

---

## Raw String Literals

`r#"..."#` avoids escaping inside strings. Useful for SQL, regex, JSON.

```rust
// Regular string: must escape quotes
let sql = "SELECT name FROM \"table\"";

// Raw string: no escaping needed
let sql = r#"SELECT name FROM "table""#;

// Multi-line raw string (SQL migrations):
let ddl = r#"
CREATE TABLE IF NOT EXISTS repos (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);
"#;
```

Add more `#` symbols if the content itself contains `"#`:
```rust
r##"contains "# inside"##
```

---

# Ownership and Lifetimes

---

## Lifetimes and '_

The `'` (tick) marks a lifetime parameter. Lifetimes track how long a
reference is valid. They prevent dangling references at compile time.

```rust
Migrations<'_>    // borrows data; compiler infers the lifetime
Migrations<'a>    // same, but you named it 'a (needed when relating lifetimes)
```

`'_` is the anonymous lifetime. "There's a lifetime here, compiler figure
it out." Not garbage, just unnamed.

`'static` means the data lives for the entire program (binary constants,
leaked heap memory). A `const` string literal is `'static`.

When do you name lifetimes explicitly? When you need to relate two: "this
output reference lives as long as this input reference." For standalone
constants, `'_` is all you need.

---

## Heap Allocation: Box vs Rc vs Arc

| | Owners | Threads | Counting | C++ equivalent |
|---|---|---|---|---|
| Stack | one | N/A | none | local variable |
| `Box<T>` | one | transferable | none | `unique_ptr` |
| `Rc<T>` | many | single thread only | yes (non-atomic) | (no equivalent) |
| `Arc<T>` | many | multi-thread safe | yes (atomic) | `shared_ptr` |

```
Box:     owner --> value         (one to one)

Rc/Arc:  owner_a -->
         owner_b -->  value      (many to one)
         owner_c -->
```

Stack:          Heap:
+----------+    +---------------------+
| Box (ptr)|----->  actual value      |
| (8 bytes)|    |  (unknown size)     |
+----------+    +---------------------+

**Box** -- use when the compiler needs heap allocation (unknown size, `dyn`
types, large values) but only one thing owns the value at a time.

**Rc** -- use when multiple things on the same thread need to read the same
value. Reference count increments on clone, decrements on drop, frees at
zero. Python's object system works like this internally.

**Arc** -- same as Rc but safe across threads. Uses atomic CPU instructions
for the count, which is slightly slower. Required when sharing data between
threads.

**Stack (no wrapper)** -- the default. Use for everything with a known size
and single owner. Most of your code.

---

## Clone and Ownership Across Threads

When a value crosses a thread boundary (via `move` closure), it must be
owned. Borrowed references (`&str`, `&Path`) can't cross because the
source might be gone when the thread runs.

```rust
fn send_to_thread(key: &str, value: &str) {
    // Can't send &str to another thread, it borrows from caller's stack

    let key_owned = key.to_string();      // &str -> owned String
    let value_owned = value.to_string();

    writer.call_write(move |conn| {
        // key_owned and value_owned moved into the closure
        conn.execute("...", params![value_owned, key_owned])?;
        Ok(())
    })?;

    // Original &str references still valid here (not moved)
    println!("{key}: {value}");
}
```

`.clone()` duplicates an owned value. `.to_string()` converts `&str` to
owned `String`. Both create independent copies the closure can own.

Rule: clone/to_string the minimum needed for the closure. Keep the
originals for code that runs after the closure.

---

# Functions and Closures

---

## Closures

A function defined inline that can see variables from the surrounding scope.
Regular functions can only see their parameters. Closures "close over"
their environment.

```rust
let multiplier = 10;
let scale = |x: i32| -> i32 { x * multiplier };  // captures multiplier
scale(5)   // returns 50
```

Python equivalent: lambdas and nested functions.
Java equivalent: lambda expressions (Java 8+).

```python
multiplier = 10
scale = lambda x: x * multiplier
```

Rust closures can be multi-line (unlike Python lambdas):

```rust
let process = |x| {
    let doubled = x * 2;
    let adjusted = doubled + multiplier;
    adjusted
};
```

The compiler infers which closure trait (Fn, FnMut, FnOnce) based on what
the body does with captured variables. See the Closure Traits section.

### move Closures

`move` tells the closure to take ownership of captured variables instead
of borrowing them.

```rust
let name = String::from("Ricky");

// Without move: closure borrows name
let greet = || println!("{}", name);
println!("{}", name);   // fine, name is still here

// With move: closure takes ownership, name is gone from this scope
let greet = move || println!("{}", name);
// println!("{}", name);   // COMPILER ERROR: name was moved
```

Rule of thumb: any closure going to another thread needs `move`. The other
thread can't safely borrow from your stack because your stack might be gone
by the time the thread runs.

```rust
// Thread closure almost always needs move
std::thread::spawn(move || {
    println!("{}", name);   // name was moved in, lives with the closure
});
```

In the writer, the Job closure uses `move` to take ownership of `f` (the
caller's closure) and `reply_tx` (the reply channel sender). Both travel
to the worker thread inside the closure.

---

## Closure Traits: Fn, FnMut, FnOnce

The compiler infers which trait a closure implements based on what the body
does with captured variables. You never label the closure itself.

| Trait | Captures | Calls | Inferred when body... |
|---|---|---|---|
| `Fn` | borrows immutably | unlimited | only reads captures |
| `FnMut` | borrows mutably | unlimited | mutates captures |
| `FnOnce` | takes ownership | exactly once | moves/consumes captures |

```rust
let name = String::from("Ricky");

|| println!("{}", name);    // reads name      -> Fn
|| names.push("hi");        // mutates names   -> FnMut
|| drop(name);              // consumes name   -> FnOnce
```

Hierarchy (each includes the one above):
```
Fn        most restrictive for closure, most flexible for caller
  ^
FnMut     also satisfies FnOnce
  ^
FnOnce    widest door, any closure qualifies
```

You label the **receiving side**, not the closure:
```rust
fn run_once(f: impl FnOnce())    // "I'll call this once"
fn run_many(f: impl Fn())        // "I'll call this many times"
```
The compiler checks that the closure you pass meets the requirement.

---

## Generics and Where Clauses

Generics on a **struct** -- fixed for the lifetime of the struct.
Generics on a **method** -- different each time you call it.

```rust
// Generic on struct: every Job through the channel has the same T.
// NOT what we want.
struct Database<T> { ... }

// Generic on method: each call picks its own T.
// This is correct.
impl Database {
    fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    { }
}
```

`where` clauses constrain generics. They go after the return type and
before the opening brace. Same as `<T: Send + 'static>` in the angle
brackets, but more readable when bounds are long.

---

## dyn -- Dynamic Dispatch

`dyn` means "I don't know the concrete type, just that it implements this
trait." Produces an unsized type, so it needs to live behind a pointer
(`Box`, `&`, `Arc`).

```rust
// Concrete: compiler knows exactly which closure this is
let f = |x| x + 1;

// Dynamic: could be ANY closure matching this signature
let f: Box<dyn FnOnce(i32) -> i32> = Box::new(|x| x + 1);
```

`Box` = where it lives (heap).
`dyn` = whether you know the concrete type (you don't).
They solve different problems, often paired together.

---

# Error Handling

---

## Error Handling

Rust splits errors into two systems. Python and Java combine them.

**Result** -- expected, recoverable errors. Visible in function signatures.
Compiler forces you to handle them.

```rust
fn read_file(path: &Path) -> Result<String, io::Error> {
    // caller MUST handle the error
}
```

**panic** -- unexpected bugs. Should crash the program (or be caught at
a thread boundary with catch_unwind).

```rust
vec![1, 2, 3][99];          // panic: index out of bounds
None::<i32>.unwrap();        // panic: called unwrap on None
```

The mapping:

```
Python try/except       ->  match on Result (expected errors)
                            catch_unwind (unexpected panics, rare)

raise ValueError(...)   ->  Err(Error::BadInput(...))
                            (returns an error, no stack unwinding)

raise (unintentional)   ->  panic!()
                            (kills the thread unless caught)
```

The key advantage over Python/Java: in Rust, the return type tells you
a function can fail. You can't forget to handle it. In Python, any
function can throw anything and you discover it at runtime.

### The ? Operator

Early return on error. Unwraps Ok or returns Err from the current function.

```rust
conn.pragma_update(None, "busy_timeout", 5000)?;
//                                              ^
// Ok(()) -> unwrap, continue
// Err(e) -> convert via From, return Err from this function
```

Without `?`:
```rust
match conn.pragma_update(None, "busy_timeout", 5000) {
    Ok(val) => val,
    Err(e) => return Err(e.into()),
}
```

The `.into()` is why `#[from]` matters on error enums. `?` calls `From`
to convert the source error into your error type automatically.

Missing `?` is the most common Rust beginner bug. The compiler warns
("unused Result that must be used") but doesn't error. Always check:
does this function return Result? Add `?`.

---

## thiserror

Derive macro that generates `Display` and `Error` trait impls.

```rust
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]          // Display format string
    RuSQLite(#[from] rusqlite::Error),       // #[from] generates From impl

    #[error("writer thread closed")]
    Writer(String),                           // no #[from], manual construction

    #[error("job panicked: {0}")]
    Panicked(String),
}
```

`#[error("...")]` -- what prints when the error is displayed.
`#[from]` -- generates `From<ThatType>` so `?` auto-converts.
`{0}` -- interpolates the first unnamed field.

Pair with a Result alias:
```rust
pub type Result<T> = std::result::Result<T, Error>;
```

---

## catch_unwind

Catches panics instead of letting them kill the thread.

```rust
let result = std::panic::catch_unwind(
    std::panic::AssertUnwindSafe(|| {
        some_function_that_might_panic();
    })
);

match result {
    Ok(value) => { /* normal return */ }
    Err(panic) => { /* panic caught, extract message */ }
}
```

`AssertUnwindSafe` -- needed when the closure captures non-unwind-safe
references (like `&mut Connection`). You're promising the compiler that
you won't reuse corrupted state after a panic.

The panic payload is `Box<dyn Any>`. Use `downcast_ref` to extract:
```rust
let msg = panic
    .downcast_ref::<&str>().map(|s| s.to_string())
    .or_else(|| panic.downcast_ref::<String>().cloned())
    .unwrap_or_else(|| "unknown panic".to_string());
```

---

# Concurrency

---

## Threads

`std::thread::spawn` creates a real OS thread, not an async task.

```rust
use std::thread;

let handle: thread::JoinHandle<()> = thread::spawn(move || {
    // this runs on a separate OS thread
    // `move` transfers ownership of captured variables into the thread
});

handle.join().unwrap();  // block until the thread finishes
```

OS threads vs async tasks:

- OS thread: real thread, own stack, scheduled by the operating system.
  Blocking is fine (the thread just sleeps).
- Async task: lightweight, multiplexed onto fewer OS threads, needs a
  runtime (tokio). Blocking is bad (stalls the whole executor).

flatten-core uses OS threads, no async runtime.

---

## Send and static

Two trait bounds that appear on anything crossing a thread boundary.

**`Send`** -- this value can be moved to another thread safely.
Most types are Send. `Rc` is not (its reference count isn't atomic).

**`'static`** -- this value contains no short-lived borrows. It can live
as long as needed. Required because the other thread might outlive any
local scope.

```rust
// Both needed because T crosses from worker thread back to caller:
T: Send + 'static

// Both needed because F is boxed and sent to the worker thread:
F: FnOnce(&mut Connection) -> Result<T> + Send + 'static
```

`'static` does NOT mean "lives forever." It means "doesn't borrow from
anything that could be dropped." Owned values like String, Vec, i32 are
all `'static`.

---

## Channels (mpsc)

"Multi-producer, single-consumer" -- a thread-safe queue.
Multiple senders push in, one receiver pops out.

```rust
use std::sync::mpsc;

// Unbounded channel (infinite buffer):
let (tx, rx) = mpsc::channel::<String>();

// Bounded channel (fixed buffer):
let (tx, rx) = mpsc::sync_channel::<String>(10);

// Rendezvous channel (buffer = 0, sender blocks until receiver reads):
let (tx, rx) = mpsc::sync_channel::<String>(0);
```

`Sender<T>` -- write end. Can be cloned (multi-producer).
`Receiver<T>` -- read end. Cannot be cloned (single-consumer).

Channels carry any `T: Send`, not just closures. Strings, integers, structs,
enums, anything that's safe to move across threads.

When the last `Sender` drops, the channel closes.
The receiver's `for item in rx` loop exits.

Python equivalent: `queue.Queue`.
Go equivalent: `chan`.

---

# Module System

---

## Module System

Rust does NOT auto-discover files. Every `.rs` file must be declared in the
module tree or it doesn't exist to the compiler.

```
lib.rs              crate root, declares top-level modules
  pub mod db;         -> points to db/mod.rs (directory module)
    pub mod error;    -> points to db/error.rs
    pub mod writer;   -> points to db/writer.rs
  pub mod runtime;    -> points to runtime.rs (file module)
```

`pub mod X;` -> X is visible outside this module.
`mod X;`     -> X is private to this module.

Adding a new file? Add a `mod` line or the compiler and rust-analyzer ignore it.

---

## use Statements

`use` is a shortcut, not a requirement. It brings a path into local scope.

```rust
// Without use -- full path every time (fine if used once):
#[derive(Debug, thiserror::Error)]

// With use -- short name (worthwhile if used many times):
use std::sync::mpsc::Sender;
let (tx, rx) = mpsc::sync_channel(0);
```

Rule of thumb: use full paths for one-off references, `use` for anything
repeated in the same file.

---

## Visibility

The full spectrum from most open to most closed:

```rust
pub fn open()              // anyone can call (external API)
pub(crate) const MIGRATIONS  // only this crate can see (internal plumbing)
pub(super) fn helper()     // only the parent module can see
fn private()               // only this module can see
```

```rust
pub enum Error { }      // visible outside this module
enum Error { }           // private to this module

pub struct Database { }  // type visible, but fields still private
```

Enum variant fields in a `pub` enum are public by default.
Struct fields require their own `pub` keyword.

Use `pub(crate)` for things like MIGRATIONS that writer.rs needs but
flatten-cli should not call directly. External consumers go through
`Writer::open()`.

---

# Tooling

---

## Cargo

```bash
# Dependency management
cargo add rusqlite -p flatten-core --features bundled  # add dependency
cargo add tempfile -p flatten-core --dev               # add dev dependency

# Building and checking
cargo check -p flatten-core     # type-check one crate (fast, no binary)
cargo check --workspace         # type-check everything, update Cargo.lock
cargo build -p flatten-cli      # compile one crate

# Testing
cargo test -p flatten-core      # run tests for one crate

# Running
cargo run -p flatten-cli -- db init --db /tmp/t.db   # run binary with args

# Lock file
cargo generate-lockfile          # update Cargo.lock without compiling
```

`-p` = `--package`. Targets one crate in a workspace.
`--` separates cargo args from binary args.
`--dev` adds to `[dev-dependencies]` (test only, not shipped).

rust-analyzer not picking up new deps?
1. Run `cargo check` first
2. VS Code command palette: "rust-analyzer: Restart server"

---

## const and Compile-Time Evaluation

`const` values are evaluated at compile time and inlined wherever used.
`static` values live at a fixed memory address for the program's lifetime.

```rust
const MAX: i32 = 100;                    // inlined at every use site
static COUNTER: AtomicI32 = AtomicI32::new(0);  // one address, shared

// const with complex types:
const MIGRATIONS: Migrations<'_> = Migrations::from_slice(SLICE);
```

Not everything can be `const`. The type must have no non-trivial destructor
issues. If const evaluation fails, use `static` with `LazyLock`:

```rust
use std::sync::LazyLock;
static MIGRATIONS: LazyLock<Migrations<'static>> = LazyLock::new(|| {
    Migrations::new(vec![...])
});
```

`const fn` marks a function as callable at compile time:

```rust
pub const fn up(sql: &str) -> Self     // can be used in const context
pub fn up(sql: &str) -> Self           // runtime only
```

The two-step const pattern avoids destructor issues with temporary arrays:

```rust
// Step 1: const slice (rvalue static promotion, no drop needed)
const MIGRATIONS_SLICE: &[M<'_>] = &[
    M::up(r#"..."#).foreign_key_check(),
];

// Step 2: const Migrations wrapping the slice
pub(crate) const MIGRATIONS: Migrations<'_> = Migrations::from_slice(MIGRATIONS_SLICE);
```

---

## Test Organization

Unit tests live at the bottom of the source file they test:

```rust
// at the bottom of writer.rs
#[cfg(test)]     // only compiled during cargo test
mod tests {
    use super::*;    // imports everything from the parent module

    /// Doc comment explains what the test proves.
    #[test]
    fn fk_rejects_bad_reference() {
        // arrange, act, assert
    }
}
```

Convention:
- `#[cfg(test)] mod tests { }` at the bottom of each file
- `use super::*` for access to the module being tested
- `use crate::other::module` for cross-module access (still unit tests)
- `///` doc comments on each test function
- Test names read as assertions: `fk_rejects_bad_reference`, not `test_fk`

Integration tests (external crate perspective) go in `tests/` directory.
Not needed until you want to verify the public API surface specifically.

---

# Flatten PM Patterns

---

## The Type Erasure Trick

Problem: the channel carries `Job` (no generic T), but each call returns
a different T. How does T survive?

Answer: T lives INSIDE the closure, not in the channel type.

```
Caller                          Channel              Worker
  |                               |                    |
  |  creates reply_tx<Result<T>>  |                    |
  |  creates Job closure that     |                    |
  |    captures reply_tx          |                    |
  |  --- sends Job -------------->|                    |
  |                               |-- delivers Job -->|
  |                               |                    | runs closure
  |                               |                    | calls f(conn)
  |                               |                    | sends Result<T>
  |  <--- receives Result<T> --------------------------| via reply_tx
  |                                                    |
```

The channel only knows about `Job` (no T). But the reply channel inside
each Job knows about T. Type preserved per-call, channel type stays fixed.

---

## Option take Pattern

Used in Drop when you need to consume a field from `&mut self`.

```rust
impl Drop for Database {
    fn drop(&mut self) {
        // Can't do: let s = self.sender;   -- moves out of borrowed struct
        // Can do:   let s = self.sender.take(); -- replaces with None

        self.sender.take();   // drops the sender, closes the channel
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();  // wait for worker thread
        }
    }
}
```

`.take()` on an `Option`: replaces the field with `None`, returns the
inner value. The value is then dropped (if not bound) or used (if bound).

---

## Deref Coercion

```rust
let tx = conn.transaction()?;   // tx: Transaction
let result = f(&mut *tx)?;      // &mut *tx: &mut Connection
```

`*tx`       -- dereference Transaction to its inner Connection (via Deref trait).
`&mut *tx`  -- take a mutable borrow of that Connection.

Transaction wraps a Connection. `&mut *tx` lets you pass it where
`&mut Connection` is expected.

---

## call vs call_write Signature Split

The writer has two methods with different closure signatures. This is
forced by the type system, not arbitrary.

```rust
// call: f receives &mut Connection
//   Full access. Can open transactions, savepoints.
db.call(|conn| {
    let tx = conn.transaction()?;   // needs &mut self, works
    tx.execute("...", [])?;
    tx.commit()
})?;

// call_write: f receives &Connection (not &mut)
//   Already inside BEGIN IMMEDIATE. Can read and write.
//   Cannot open nested transactions or savepoints.
db.call_write(|conn| {
    conn.execute("INSERT ...", [])?;   // execute takes &self, works
    conn.execute("UPDATE ...", [])?;
    Ok(())
})?;
```

Why the split:
- `transaction_with_behavior(&mut self)` borrows Connection mutably
- While Transaction exists, you can't also give `&mut Connection` to f
- Transaction implements `Deref<Target = Connection>` but NOT `DerefMut`
- So f can only receive `&Connection` inside a transaction
- All rusqlite write methods take `&self`, so `&Connection` is sufficient
- `call` keeps `&mut` so callers can open their own transactions when needed

Limitations inside call_write:
- No nested checked transactions (needs `&mut Connection`)
- No savepoints (needs `&mut Transaction`)
- For those, use `call` with manual transaction management

This matches rusqlite-isle's pattern and is confirmed correct by the
rusqlite 0.40.2 source code.

---

## Quick Reference Table

| Concept | Python equivalent | When to use |
|---|---|---|
| `&` / `*` | (automatic in Python) | reference / dereference |
| `String` | `str` (mutable) | owned string data, heap-allocated |
| `&str` | `str` (immutable view) | borrowed string data, read-only |
| `&[T]` | no equivalent (list is always owned) | borrowed view of contiguous data |
| `Vec<T>` | `list` | growable owned list |
| `r#"..."#` | raw strings `r"..."` | SQL, regex, strings with quotes |
| `_` | `_` (convention only) | compiler-enforced discard |
| no semicolon | `return` | expression-based return value |
| `if let` | walrus operator `:=` (weaker) | pattern match + extract in condition |
| `'_` | no equivalent | anonymous lifetime, compiler infers |
| `Box<T>` | default (everything is heap) | unknown size, `dyn` types |
| `Rc<T>` | Python's refcount (under the GIL) | shared ownership, one thread |
| `Arc<T>` | (no direct equivalent) | shared ownership, across threads |
| `.clone()` | (automatic in Python) | duplicate owned data for thread transfer |
| `move` closure | (not needed in Python) | closure crossing thread boundary |
| `FnOnce` | any callable (no restriction in Python) | one-shot closure |
| `FnMut` | (no restriction in Python) | reusable closure that mutates state |
| `Fn` | (no restriction in Python) | reusable read-only closure |
| `dyn Trait` | duck typing / protocols | type erasure |
| `Box<dyn Trait>` | normal object reference | heap + type erasure |
| `?` operator | (no equivalent, try/except) | early return on error |
| `thiserror` | (no equivalent, exceptions) | define error types |
| `catch_unwind` | try/except for panics | protect threads from panics |
| `mpsc::channel` | `queue.Queue` | thread communication |
| `thread::spawn` | `threading.Thread` | real OS thread |
| `pub(crate)` | no equivalent (all public) | internal API, not for external consumers |
| `const` | no equivalent (no compile-time eval) | compile-time constants |
| `cargo check` | no equivalent | fast type-check without building |
| `#[cfg(test)]` | `if __name__ == "__main__"` (roughly) | test-only code |
| `ExitCode` | `sys.exit(n)` | explicit process exit code from main |