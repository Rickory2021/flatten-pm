<!-- docs/lessons/01-ownership-and-borrowing.md -->
# Rust Lesson 1: Ownership and Borrowing

## Where Rust sits in the landscape

Every language you've used makes a tradeoff on memory management:

| Language | Strategy | Who decides when to free? | Runtime cost |
|----------|----------|---------------------------|-------------|
| C | Manual (`malloc`/`free`) | You | Zero |
| Java | Garbage collector (tracing GC) | JVM, at unpredictable intervals | GC pauses, heap overhead |
| Python | Reference counting + cycle collector | Runtime, on every assignment/scope exit | Refcount bump on every ref change |
| TypeScript/JS | Garbage collector (generational GC) | V8/engine | Same class as Java |
| Assembly | You manage registers and stack directly | You | Zero |
| **Rust** | **Ownership rules, enforced at compile time** | **The compiler, deterministically** | **Zero** |

Rust occupies the same row as C and assembly: no runtime, no GC, zero overhead. The difference is that the compiler proves your memory usage is safe before producing a binary. If it compiles, there are no use-after-free bugs, no double-frees, no dangling pointers, no data races. That's the contract.

In C, you write `malloc`, you write `free`, and if you get it wrong, you get a segfault at runtime (if you're lucky) or silent memory corruption (if you're not). Rust eliminates that entire class of bugs by making them compile-time errors.

## The three ownership rules

Everything in Rust traces back to three rules:

1. Every value has exactly one **owner** (a variable binding).
2. When the owner goes out of scope, the value is **dropped** (memory freed).
3. There can only be one owner at a time.

If you've written C, rule 2 is like an automatic `free()` at the closing brace of the scope that owns the allocation. Except you can't forget it, call it twice, or call it on the wrong pointer. The compiler handles it.

If you've written Java, think of it this way: imagine if the JVM could prove at compile time exactly when every object becomes unreachable, and inserted the deallocation right there. No GC needed. That's what ownership does.

## Stack vs heap: same model as C

Rust has the same stack/heap model you know from C and assembly:

- **Stack:** fixed-size, LIFO, fast. Function locals, integers, booleans, fixed-size structs. Freed automatically when the function returns (stack pointer moves back, same as C/assembly).
- **Heap:** dynamic-size, manual layout, slower. Anything whose size isn't known at compile time or that needs to outlive the current stack frame.

In C, you choose: local variable on the stack, `malloc` for the heap. In Rust, the type determines it:

```rust
let x: i32 = 42;                    // stack (like C's `int x = 42;`)
let s: String = String::from("hi"); // heap-allocated buffer, metadata on stack
                                     // (like C's `char *s = strdup("hi");`)
```

The difference: in C, you must `free(s)`. In Rust, `s` is freed when it goes out of scope. Automatically, deterministically, at the exact right time. No GC, no refcount.

## Move semantics

This is the first thing that will feel wrong coming from Python/TS/Java.

```rust
fn main() {
    let name = String::from("Ricky");
    let other = name;          // ownership MOVES to `other`
    println!("{}", name);      // COMPILE ERROR: value used after move
}
```

**What each language would do here:**

| Language | What `other = name` does | Both usable after? |
|----------|--------------------------|-------------------|
| Python | `other` gets a new reference to the same object, refcount incremented | Yes |
| Java | `other` gets a copy of the reference, GC tracks both | Yes |
| TS/JS | Same as Java (reference copy, GC tracked) | Yes |
| C | `other` gets a bitwise copy of the struct/pointer; both point to same memory; double-free is your problem | Technically yes, but dangerous |
| **Rust** | **Ownership transfers; `name` is invalidated** | **No. `name` is dead.** |

Rust's move is closest to C's behavior (bitwise copy of the stack data), but with a critical addition: the compiler marks the source as invalid. C lets you use both and hopes you don't double-free. Rust prevents the bug by making the old variable unusable.

**Why?** Because if both `name` and `other` owned the data, when they both go out of scope, the data gets freed twice. Double-free. In C, that's undefined behavior. Rust prevents it structurally.

### At the assembly level

If you think about what the compiler emits: `let other = name` is a `memcpy` of the stack metadata (pointer, length, capacity for a `String`) followed by the compiler simply not emitting any drop code for `name`. The heap allocation isn't touched. It's a bookkeeping change, not a data operation.

## Copy types: the exception

Small, stack-only types implement the `Copy` trait. For these, assignment copies instead of moving:

```rust
let x: i32 = 5;
let y = x;           // x is COPIED (like C's value semantics for int)
println!("{}", x);   // fine, x is still valid
```

**Types that are `Copy`:** all integer types (`i32`, `u64`, etc.), `f32`, `f64`, `bool`, `char`, tuples of `Copy` types, fixed-size arrays of `Copy` types.

**Types that are NOT `Copy`:** `String`, `Vec<T>`, any heap-allocated type, any type with a custom destructor.

The mental model from C: if the type fits entirely on the stack and has no pointer to heap data, it's `Copy`. If it owns a heap allocation (like `String` owns a `char*` buffer), it moves.

In Java terms: primitives (`int`, `double`, `boolean`) are value types, everything else is reference types. Rust's `Copy` types are analogous to Java's primitives.

## Functions and ownership

In C, passing a pointer to a function is ambiguous: does the function take ownership (expected to `free` it) or just borrow it (caller still owns it)? The answer is "read the docs and hope." In Rust, the function signature makes it explicit:

### Taking ownership (consuming the value)

```rust
fn consume(s: String) {
    println!("Got: {}", s);
}   // s is dropped here, heap memory freed

fn main() {
    let name = String::from("Ricky");
    consume(name);              // ownership moves into the function
    // name is now invalid; trying to use it is a compile error
}
```

C equivalent: passing a `char *` to a function that `free()`s it. Except in C, nothing stops you from using the pointer after the call. In Rust, the compiler does.

Java equivalent: there isn't one. Java references are always shared, GC handles cleanup. You can't express "this function takes exclusive ownership" in Java's type system.

### Returning ownership

```rust
fn create_greeting(who: &str) -> String {
    format!("Hello, {}!", who)
}

fn main() {
    let greeting = create_greeting("Ricky");  // ownership moves to caller
    println!("{}", greeting);                  // we own it, we can use it
}   // greeting dropped here
```

The function creates a `String` on the heap, then transfers ownership to the caller via the return value. No copies, no GC. At the assembly level, the caller allocates space, the function fills it, and the pointer is returned. Same as C's `return strdup(...)` pattern, but the compiler tracks who must free it.

## Borrowing: references without ownership transfer

This is Rust's equivalent of "pass a pointer but don't transfer ownership." The syntax is `&`.

### Immutable borrow (`&T`)

```rust
fn print_length(s: &String) {  // borrows s, does not own it
    println!("Length: {}", s.len());
}   // the reference goes out of scope, but the data isn't dropped (we don't own it)

fn main() {
    let name = String::from("Ricky");
    print_length(&name);         // lend a reference
    println!("{}", name);        // still valid, we still own it
}
```

**C parallel:** `void print_length(const char *s)`. The `const` signals "I won't modify this." But in C, `const` is advisory; you can cast it away. In Rust, `&T` is an enforced contract: the borrow is truly immutable.

**Java parallel:** passing an object reference. Except Java has no way to say "this function promises not to mutate the object." Rust's `&T` is that promise, compiler-enforced.

### Mutable borrow (`&mut T`)

```rust
fn add_exclamation(s: &mut String) {  // mutable borrow
    s.push_str("!");
}

fn main() {
    let mut name = String::from("Ricky");  // variable must be declared `mut`
    add_exclamation(&mut name);             // mutable borrow
    println!("{}", name);                   // prints "Ricky!"
}
```

**C parallel:** `void add_excl(char **s)` or `void add_excl(struct string *s)`. The pointer gives write access. In C, nothing prevents another thread from reading through a different pointer at the same time. In Rust, the borrowing rules prevent that.

**Java parallel:** any method that mutates an object via a reference. The difference is Java can't prevent data races; Rust can.

Note: both the variable declaration (`let mut`) and the borrow site (`&mut`) must agree that mutation is happening. This is intentional. Mutation is always visible and explicit, never hidden.

## The borrowing rules

At any given time, a value can have:

- **Any number of `&T` (immutable/shared borrows),** OR
- **Exactly one `&mut T` (mutable/exclusive borrow)**

Never both simultaneously.

```rust
let mut s = String::from("hello");

let r1 = &s;         // ok: first immutable borrow
let r2 = &s;         // ok: multiple immutable borrows allowed
let r3 = &mut s;     // COMPILE ERROR: can't borrow mutably while immutable borrows exist
```

```rust
let mut s = String::from("hello");

let r1 = &mut s;     // ok: one mutable borrow
let r2 = &mut s;     // COMPILE ERROR: can't have two mutable borrows
```

### The concurrency connection

If you've studied concurrency (and with your CS degree, you have): these rules are the reader-writer lock pattern, enforced at compile time.

| Borrowing rule | Concurrency equivalent |
|----------------|----------------------|
| Multiple `&T` allowed | Multiple readers, no writers (shared read lock) |
| One `&mut T` only | Exclusive write lock |
| Can't mix `&T` and `&mut T` | Can't read while writing |

In Java, you'd use `synchronized` or `ReadWriteLock` at runtime and hope you got it right. In C, you'd use pthreads mutexes and hope you didn't miss a code path. In Rust, the compiler rejects the program if the pattern is violated. Data races are structurally impossible in safe Rust.

## Lifetimes: the short version

Rust needs to prove that references don't outlive the data they point to. In C, a dangling pointer (returning a pointer to a local variable) compiles fine and crashes at runtime:

```c
// C: compiles, crashes
char* get_name() {
    char name[6] = "Ricky";
    return name;  // dangling pointer to stack memory that's gone
}
```

Rust prevents this at compile time:

```rust
// Rust: does NOT compile
fn get_name() -> &str {
    let name = String::from("Ricky");
    &name  // COMPILE ERROR: `name` does not live long enough
}          // name is dropped here, reference would dangle
```

Most of the time, the compiler figures out lifetimes automatically (called **lifetime elision**). When it can't, you write explicit lifetime annotations like `'a`. We'll cover the syntax in a later lesson; for now, the mental model is: when the compiler complains about lifetimes, it's saying "I can't prove this reference will still be valid when you try to use it."

## Connecting to flatten-core

When you start implementing the export pipeline in `flatten-core`, here's where ownership shows up in practice:

- **File content as `String`:** when you read a file, you get a `String` (owned, heap-allocated). Passing it to a processing function means deciding: does the function need ownership (consuming the content), or just a view (`&str`, a borrowed string slice)?
- **Modifier cascades:** the cascade config will likely be a struct. Functions that apply modifiers borrow the config (`&Config`) rather than taking ownership; the config is read many times, consumed zero times.
- **Walking directories:** iterator patterns that yield paths. Each path is owned by the iterator; your processing code borrows it or takes ownership depending on whether it needs to store the path.

The pattern you'll use most: functions take `&self` (borrow the struct) or `&str` (borrow a string) and return owned types (`String`, `Vec<PathBuf>`). Borrow the inputs, own the outputs.

## Summary

| Concept | C equivalent | Java equivalent | Rust |
|---------|-------------|-----------------|------|
| Stack allocation | Local variable | Primitive types | `let x: i32 = 5;` |
| Heap allocation | `malloc` | `new Object()` | `String::from("hi")` |
| Deallocation | `free()` (manual) | GC (automatic, non-deterministic) | Drop at scope exit (automatic, deterministic) |
| Pass without copy | `func(ptr)` (ambiguous ownership) | `func(obj)` (shared ref, GC tracked) | `func(&val)` (borrow, compiler-checked) |
| Transfer ownership | Convention only | Not expressible | `func(val)` (move) |
| Prevent mutation | `const` (advisory, castable) | `final` (ref only, not contents) | `&T` (enforced, deep) |
| Prevent data races | Mutex (runtime, hope-based) | `synchronized` (runtime) | Borrow rules (compile-time) |
| Dangling pointer | Compiles, crashes | Impossible (GC) | Compile error |
| Double free | Compiles, undefined behavior | Impossible (GC) | Compile error (move prevents it) |
