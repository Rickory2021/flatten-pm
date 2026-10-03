<!-- docs/lessons/05-modules-and-crate-structure.md -->
# Rust Lesson 5: Modules and Crate Structure

## The file system IS the module system

In Python, every `.py` file is a module. In Java, every `.java` file is a class in a package. Rust is closer to Python: every `.rs` file is a module, but you have to explicitly declare the relationship.

The key difference from Python: Python auto-discovers modules when you import. Rust requires you to declare modules before they exist to the compiler. Nothing is implicit.

## Two units of code organization

Rust has two levels:

| Unit | What it is | Analogy |
|------|-----------|---------|
| **Module** (`mod`) | A namespace within a crate. Can be a file or a block. | Python module, Java package |
| **Crate** | A compilation unit. Either a library or a binary. | Python package (with `__init__.py`), Java JAR |

Your repo has three crates, declared in the workspace `Cargo.toml`:

```toml
[workspace]
members = [
  "src-tauri",           # flatten-pm (Tauri app, binary + library)
  "crates/flatten-core",  # flatten-core (library)
  "crates/flatten-cli",   # flatten-cli (binary)
]
```

Each crate compiles independently. Dependencies between them are declared in their respective `Cargo.toml` files.

## Crate roots: lib.rs vs main.rs

Every crate has a root file that the compiler starts from:

| File | Crate type | Purpose |
|------|-----------|---------|
| `src/lib.rs` | Library | Exports code for other crates to use |
| `src/main.rs` | Binary | Has a `fn main()`, produces an executable |

Your repo right now:

```
crates/flatten-core/src/lib.rs    ← library crate root (other crates depend on this)
crates/flatten-cli/src/main.rs    ← binary crate root (produces an executable)
src-tauri/src/lib.rs              ← Tauri app library root (special: both lib + bin)
src-tauri/src/main.rs             ← Tauri app binary root
```

The Tauri app is unusual in having both `lib.rs` and `main.rs`. That's a Tauri convention; `main.rs` is the entry point that calls `lib.rs`'s `run()` function. For flatten-core and flatten-cli, it's the standard pattern: one library, one binary.

## Modules within a crate

As flatten-core grows, you'll split it into modules. There are two ways to declare them:

### Inline modules (small, keeps everything in one file)

```rust
// crates/flatten-core/src/lib.rs

mod scanner {
    pub struct ScanFinding {
        pub line: usize,
        pub message: String,
    }

    pub fn scan_content(content: &str) -> Vec<ScanFinding> {
        vec![]  // placeholder
    }
}

mod exporter {
    pub fn export_files() {
        // ...
    }
}
```

Everything in one file, namespaced by `mod` blocks. Fine for small code, doesn't scale.

### File-based modules (the standard approach)

Each module gets its own file:

```
crates/flatten-core/src/
  lib.rs          ← crate root, declares modules
  scanner.rs      ← scanner module
  exporter.rs     ← exporter module
  watcher.rs      ← watcher module
  error.rs        ← error types
```

In `lib.rs`, you declare the modules exist:

```rust
// crates/flatten-core/src/lib.rs

pub mod scanner;    // tells compiler: load scanner.rs as a module
pub mod exporter;
pub mod watcher;
pub mod error;
```

Each file then contains its module's code:

```rust
// crates/flatten-core/src/scanner.rs

pub struct ScanFinding {
    pub line: usize,
    pub message: String,
}

pub fn scan_content(content: &str) -> Vec<ScanFinding> {
    vec![]
}
```

**The `mod` declaration is mandatory.** If you create `scanner.rs` but don't add `pub mod scanner;` to `lib.rs`, the compiler doesn't know it exists. This is the biggest difference from Python, where creating a `.py` file makes it importable automatically.

### Nested modules (subdirectories)

For deeper organization:

```
crates/flatten-core/src/
  lib.rs
  scanner/
    mod.rs           ← module root for the scanner directory
    rules.rs         ← sub-module
    findings.rs      ← sub-module
```

```rust
// crates/flatten-core/src/lib.rs
pub mod scanner;    // compiler looks for scanner.rs OR scanner/mod.rs

// crates/flatten-core/src/scanner/mod.rs
pub mod rules;      // loads scanner/rules.rs
pub mod findings;   // loads scanner/findings.rs

pub fn scan_content(content: &str) -> Vec<findings::ScanFinding> {
    // ...
}
```

The compiler resolves `pub mod scanner;` by looking for either `scanner.rs` or `scanner/mod.rs`. When a module grows big enough to split, you create the directory and move the code into `mod.rs`. The declaration in `lib.rs` doesn't change.

## Visibility: pub and privacy

Rust defaults to **private**. Everything is hidden unless you explicitly mark it `pub`:

```rust
pub struct Profile {           // type is public
    pub name: String,          // field is public
    root_dir: String,          // field is PRIVATE (no pub)
    include_patterns: Vec<String>,  // private
}

impl Profile {
    pub fn new(name: String, root_dir: String) -> Self {  // public constructor
        Profile {
            name,
            root_dir,
            include_patterns: Vec::new(),
        }
    }

    fn validate(&self) -> bool {  // private method (no pub)
        !self.name.is_empty()
    }

    pub fn export(&self) -> Result<(), ExportError> {  // public method
        if !self.validate() {  // can call private method from within the impl
            return Err(ExportError::InvalidProfile("empty name".into()));
        }
        // ...
        Ok(())
    }
}
```

### Visibility compared across languages

| | Default visibility | Access control |
|--|-------------------|---------------|
| C | Everything visible (header = public API) | Convention only (prefix with `_` for "private") |
| Java | Package-private | `public`, `protected`, `private`, package |
| Python | Everything public | Convention (`_prefix` means "private", not enforced) |
| TS | Public (in modules, export controls visibility) | `public`, `private`, `protected` in classes |
| **Rust** | **Private** | **`pub` to expose, compiler-enforced** |

Rust's default-private is the safest default. You expose only what you intend to. In Python, everything is accessible and "private" is a suggestion. In Rust, private means the compiler rejects access from outside the module.

### pub(crate): visible within the crate but not outside

Sometimes you need something accessible across modules within your crate, but not exposed to external consumers:

```rust
// Visible to all modules in flatten-core, but not to flatten-cli or src-tauri
pub(crate) fn internal_helper() { /* ... */ }
```

| Visibility | Who can see it |
|-----------|---------------|
| (no keyword) | Same module only |
| `pub(crate)` | Anywhere in the same crate |
| `pub` | Everyone, including external crates |

Java's closest analog: default (no keyword) is private, `pub(crate)` is package-private, `pub` is public.

## use: bringing names into scope

`use` is Rust's `import`. It brings names from other modules or crates into the current scope:

```rust
// Absolute path (from crate root)
use crate::scanner::ScanFinding;
use crate::error::ExportError;

// From an external crate
use std::fs;
use std::path::PathBuf;
use std::collections::HashMap;

// From a dependency crate
use serde::{Serialize, Deserialize};
```

### Path prefixes

| Prefix | Meaning | Analogy |
|--------|---------|---------|
| `crate::` | Root of the current crate | Python's absolute import from package root |
| `self::` | Current module | Python's relative import `.` |
| `super::` | Parent module | Python's relative import `..` |
| No prefix | External crate or std | Python's `import package` |

```rust
// In crates/flatten-core/src/scanner.rs:
use crate::error::ExportError;   // from sibling module via crate root
use super::exporter;              // parent module (lib.rs level), then exporter
use self::rules::RuleSet;         // sub-module within scanner
use std::path::Path;              // standard library
```

### Grouping imports

```rust
// Individual
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::BTreeMap;

// Grouped (preferred)
use std::collections::{HashMap, HashSet, BTreeMap};

// Nested grouping
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};
```

Same as Python grouping imports from the same package, just different syntax.

## How your crates connect

Here's how the dependency graph works in your repo:

```
flatten-core (library, no dependencies on other workspace crates)
    ↑                    ↑
    |                    |
flatten-cli          flatten-pm (src-tauri)
(binary)             (Tauri app)
```

### flatten-core depends on nothing in the workspace

```toml
# crates/flatten-core/Cargo.toml
[dependencies]
# external crates only, no workspace dependencies
```

This is the ADR from DESIGN.md: core logic has no Tauri dependencies, consumable by CLI, Tauri, MCP, and tests independently.

### flatten-cli depends on flatten-core

```toml
# crates/flatten-cli/Cargo.toml
[dependencies]
flatten-core = { path = "../flatten-core" }
```

```rust
// crates/flatten-cli/src/main.rs
use flatten_core::scanner::ScanFinding;  // use the library
use flatten_core::exporter::export;

fn main() {
    // CLI entrypoint, calls into flatten-core
}
```

Note the name: `flatten-core` in Cargo.toml (with hyphen), `flatten_core` in Rust code (with underscore). Cargo automatically converts hyphens to underscores for the crate name in code. This catches people off guard.

### src-tauri depends on flatten-core

```toml
# src-tauri/Cargo.toml
[dependencies]
flatten-core = { path = "../crates/flatten-core" }
tauri = { version = "2", features = [] }
serde = { version = "1", features = ["derive"] }
```

```rust
// src-tauri/src/lib.rs
use flatten_core::exporter;

#[tauri::command]
fn export_profile(name: &str) -> Result<String, String> {
    // Tauri command that calls into flatten-core
    Ok(format!("exporting {}", name))
}
```

The Tauri app is a thin shell: it handles the GUI, registers commands, and delegates to flatten-core for all real logic.

## Cargo.toml: the manifest

Every crate has a `Cargo.toml` that declares metadata and dependencies. This is Rust's equivalent of `package.json` (Node), `pyproject.toml` (Python), or `pom.xml` (Java/Maven):

```toml
[package]
name = "flatten-core"
version = "0.1.0"
edition = "2024"

[dependencies]
thiserror = "2"                    # from crates.io (Rust's npm/PyPI)
serde = { version = "1", features = ["derive"] }
regex = "1"

[dev-dependencies]                 # only for tests
tempfile = "3"
```

| Cargo.toml | package.json | pyproject.toml |
|-----------|-------------|---------------|
| `[dependencies]` | `dependencies` | `[project.dependencies]` |
| `[dev-dependencies]` | `devDependencies` | `[project.optional-dependencies.dev]` |
| `features = ["derive"]` | N/A (different mechanism) | `extras` |
| `version = "1"` | `^1.0.0` (SemVer) | `>=1.0,<2.0` |

`cargo build` fetches and compiles dependencies. `cargo run` builds and runs. `cargo test` builds and runs tests. Same workflow as `npm install && npm run` or `pip install && python -m pytest`.

## Tests live next to the code

Rust convention puts unit tests in the same file as the code they test:

```rust
// crates/flatten-core/src/scanner.rs

pub fn scan_content(content: &str) -> Vec<ScanFinding> {
    // implementation
}

#[cfg(test)]           // only compiled when running tests
mod tests {
    use super::*;      // import everything from the parent module

    #[test]
    fn test_empty_content() {
        let findings = scan_content("");
        assert!(findings.is_empty());
    }

    #[test]
    fn test_finds_secret() {
        let content = "api_key = \"sk-1234567890\"";
        let findings = scan_content(content);
        assert_eq!(findings.len(), 1);
    }
}
```

`#[cfg(test)]` means the test module is only compiled during `cargo test`. It doesn't exist in the release binary. `use super::*` imports everything from the parent module, including private functions, so you can test internals.

This is different from Java (tests in a separate `test/` directory) and Python (tests in `tests/` or alongside). Rust puts them right next to the code. Integration tests go in a top-level `tests/` directory.

### Your current test in lib.rs

```rust
// crates/flatten-core/src/lib.rs (current state)
pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
```

This is the default Cargo scaffold. Same pattern you'll use as you build out the real modules.

## Putting it together: what flatten-core will look like

As you implement the export pipeline, the module structure will grow:

```
crates/flatten-core/src/
  lib.rs              ← pub mod declarations, re-exports
  error.rs            ← ExportError, WatchError (thiserror)
  profile.rs          ← Profile struct, loading, validation
  exporter.rs         ← export pipeline (walk, flatten, write)
  scanner.rs          ← content safety scanning
  manifest.rs         ← SHA256 manifest, _CONTEXT.yaml
  modifier.rs         ← include/exclude cascade logic
  watcher.rs          ← watch pipeline (later)
```

```rust
// crates/flatten-core/src/lib.rs
pub mod error;
pub mod profile;
pub mod exporter;
pub mod scanner;
pub mod manifest;
pub mod modifier;

// Optional: re-export key types at the crate root for convenience
pub use error::ExportError;
pub use profile::Profile;
pub use exporter::ExportResult;
```

The `pub use` re-exports let consumers write `use flatten_core::Profile` instead of `use flatten_core::profile::Profile`. Convenience, not necessity.

## Summary

| Concept | Python | Java | Rust |
|---------|--------|------|------|
| Module | `.py` file (auto-discovered) | Class in a package | `.rs` file (must be declared with `mod`) |
| Package/Crate | Directory with `__init__.py` | JAR / package hierarchy | Crate with `Cargo.toml` |
| Imports | `from x import y` | `import x.y.Z` | `use crate::x::Y` |
| Default visibility | Public | Package-private | Private |
| Dependency management | pip / pyproject.toml | Maven / Gradle | Cargo / Cargo.toml |
| Test location | `tests/` directory or `test_*.py` | `src/test/` directory | Same file, `#[cfg(test)] mod tests` |
| Workspace / monorepo | N/A (maybe Poetry workspaces) | Multi-module Maven/Gradle | `[workspace]` in root Cargo.toml |
| File → module name | Automatic | Automatic (file must match class name) | Manual (`mod name;` declaration required) |
