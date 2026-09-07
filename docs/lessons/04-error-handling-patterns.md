<!-- docs/lessons/04-error-handling-patterns.md -->
# Rust Lesson 4: Error Handling Patterns

## Recap from lesson 2

Rust has no exceptions. Functions that can fail return `Result<T, E>`:

```rust
enum Result<T, E> {
    Ok(T),    // success
    Err(E),   // failure with error info
}
```

The `?` operator propagates errors early:

```rust
let content = fs::read_to_string("file.txt")?;  // returns Err if it fails
```

This lesson covers how to build error types for a real codebase, how to convert between them, and the patterns you'll use in flatten-core.

## The problem: different operations produce different errors

In flatten-core, a single export operation touches multiple failure modes:

```rust
fn export_profile(profile: &Profile) -> Result<ExportResult, ???> {
    let config = load_config(&profile.config_path)?;   // io::Error or parse error
    let files = walk_directory(&profile.root_dir)?;     // io::Error
    let scanned = scan_content(&files)?;                // ScanError
    let manifest = build_manifest(&files)?;             // io::Error
    Ok(ExportResult { files, manifest, scanned })
}
```

Each `?` might produce a different error type. In Python, you'd just let different exceptions fly and catch them upstream. In Java, you'd either declare `throws IOException, ParseException, ScanException` or catch and wrap. In Rust, the return type must be ONE type. So how do you unify them?

## Approach 1: Custom error enum

The most common pattern. Define an enum where each variant holds a different error type:

```rust
#[derive(Debug)]
enum ExportError {
    Io(std::io::Error),
    Config(ConfigParseError),
    Scan(ScanError),
    InvalidProfile(String),     // variant with just a message
}
```

This is the same enum-with-data pattern from lesson 2, applied to errors. Each variant wraps a different underlying error.

### Implementing Display and Error traits

Rust's error ecosystem expects two traits on your error type:

```rust
use std::fmt;

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportError::Io(e) => write!(f, "IO error: {}", e),
            ExportError::Config(e) => write!(f, "Config error: {}", e),
            ExportError::Scan(e) => write!(f, "Scan error: {}", e),
            ExportError::InvalidProfile(msg) => write!(f, "Invalid profile: {}", msg),
        }
    }
}

impl std::error::Error for ExportError {}
```

`Display` is the human-readable message (like Python's `__str__` or Java's `getMessage()`). The `Error` trait marks it as a proper error type that works with the standard error handling ecosystem.

### Implementing From for the ? operator

Here's the key piece. The `?` operator doesn't just unwrap `Result`; it also converts the error type using the `From` trait. If you implement `From<io::Error> for ExportError`, then `?` on an `io::Error` automatically wraps it:

```rust
impl From<std::io::Error> for ExportError {
    fn from(e: std::io::Error) -> Self {
        ExportError::Io(e)
    }
}

impl From<ConfigParseError> for ExportError {
    fn from(e: ConfigParseError) -> Self {
        ExportError::Config(e)
    }
}

impl From<ScanError> for ExportError {
    fn from(e: ScanError) -> Self {
        ExportError::Scan(e)
    }
}
```

Now the `?` operator just works across all three error types:

```rust
fn export_profile(profile: &Profile) -> Result<ExportResult, ExportError> {
    let config = load_config(&profile.config_path)?;   // io::Error → ExportError::Io via From
    let files = walk_directory(&profile.root_dir)?;     // io::Error → ExportError::Io via From
    let scanned = scan_content(&files)?;                // ScanError → ExportError::Scan via From
    Ok(ExportResult { files, manifest, scanned })
}
```

Each `?` calls `From::from()` to convert the specific error into your unified type. No manual wrapping, no try/catch, no boilerplate at each call site.

### What ? actually desugars to (with From)

```rust
// This:
let config = load_config(&profile.config_path)?;

// Desugars to:
let config = match load_config(&profile.config_path) {
    Ok(val) => val,
    Err(e) => return Err(ExportError::from(e)),  // From conversion happens here
};
```

The `From` trait is what makes `?` work across different error types. Without it, you'd need to manually convert at every call site.

## Approach 2: thiserror crate (same thing, less boilerplate)

Writing `Display`, `Error`, and `From` impls by hand is verbose. The `thiserror` crate generates all of it with derive macros:

```rust
use thiserror::Error;

#[derive(Debug, Error)]
enum ExportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Config error: {0}")]
    Config(#[from] ConfigParseError),

    #[error("Scan error: {0}")]
    Scan(#[from] ScanError),

    #[error("Invalid profile: {0}")]
    InvalidProfile(String),
}
```

That's it. `#[error("...")]` generates the `Display` impl. `#[from]` generates the `From` impl. The `Error` trait impl is automatic. Same result as approach 1, fraction of the code.

This is the standard approach in the Rust ecosystem for library crates. You'll likely use it in flatten-core.

## Approach 3: anyhow crate (for applications, not libraries)

When you don't need callers to match on specific error variants (CLI tools, scripts, quick prototyping):

```rust
use anyhow::{Result, Context};

fn export_profile(profile: &Profile) -> Result<ExportResult> {
    let config = load_config(&profile.config_path)
        .context("Failed to load config")?;

    let files = walk_directory(&profile.root_dir)
        .context("Failed to walk directory")?;

    Ok(ExportResult { files })
}
```

`anyhow::Result<T>` is shorthand for `Result<T, anyhow::Error>` where `anyhow::Error` can hold any error type. `.context()` adds a human-readable message to the error chain.

### When to use which

| Approach | Use when | Example |
|----------|----------|---------|
| Custom enum (manual) | Learning, small projects, full control | Understanding the pattern |
| `thiserror` | Library crates where callers need to match error variants | flatten-core |
| `anyhow` | Application code, CLIs, where you just display the error | flatten-cli |

The rule of thumb: if someone else calls your code and might need to handle different errors differently, use `thiserror` (typed errors). If you're the top-level consumer and just need to print or log the error, use `anyhow` (erased errors).

For flatten-pm: `thiserror` in flatten-core (the library), `anyhow` in flatten-cli (the binary).

## Comparison across languages

### C: manual error codes

```c
int export_profile(Profile *p, ExportResult *out) {
    char *config = load_config(p->config_path);
    if (config == NULL) return ERR_IO;           // error code

    File *files = walk_dir(p->root_dir);
    if (files == NULL) { free(config); return ERR_IO; }  // manual cleanup

    int scan_result = scan_content(files);
    if (scan_result < 0) { free(files); free(config); return ERR_SCAN; }

    // build result, hope we didn't forget a free()
    return 0;
}
```

Every call site checks, every error path cleans up manually, and if you forget one `free()` you have a leak. Rust's `?` plus ownership eliminates both problems.

### Java: exceptions

```java
ExportResult exportProfile(Profile p) throws IOException, ConfigException, ScanException {
    Config config = loadConfig(p.configPath);       // throws ConfigException
    List<File> files = walkDirectory(p.rootDir);     // throws IOException
    ScanResult scanned = scanContent(files);         // throws ScanException
    return new ExportResult(files, scanned);
}
```

Cleaner than C, but: checked exceptions are routinely caught and swallowed (`catch (Exception e) {}`), unchecked exceptions don't appear in the signature at all, and the error types are classes in a hierarchy rather than a flat enum.

### Python: exceptions (untyped)

```python
def export_profile(profile: Profile) -> ExportResult:
    config = load_config(profile.config_path)   # might raise IOError
    files = walk_directory(profile.root_dir)     # might raise OSError
    scanned = scan_content(files)                # might raise ScanError
    return ExportResult(files, scanned)
    # caller has no idea what exceptions might come out
    # unless they read the source or docstring
```

Nothing in the signature indicates failure modes. The caller has to guess or read docs. Rust's `Result<T, ExportError>` makes every failure mode visible in the type.

### Rust: Result with ?

```rust
fn export_profile(profile: &Profile) -> Result<ExportResult, ExportError> {
    let config = load_config(&profile.config_path)?;
    let files = walk_directory(&profile.root_dir)?;
    let scanned = scan_content(&files)?;
    Ok(ExportResult { files, scanned })
}
```

Same clean flow as Python, but: every error type is explicit in the return type, the compiler forces callers to handle the Result, and `?` handles conversion automatically via `From`.

## Pattern: matching on error variants

The caller can decide how to handle each error type:

```rust
match export_profile(&profile) {
    Ok(result) => println!("Exported {} files", result.files.len()),
    Err(ExportError::Io(e)) => eprintln!("File system error: {}", e),
    Err(ExportError::Config(e)) => eprintln!("Bad config: {}", e),
    Err(ExportError::Scan(e)) => {
        eprintln!("Security scan failed: {}", e);
        // maybe still export but flag it
    }
    Err(ExportError::InvalidProfile(msg)) => eprintln!("Fix profile: {}", msg),
}
```

Exhaustive matching means adding a new variant to `ExportError` forces every caller to handle it. In Java, adding a new checked exception to `throws` does something similar, but unchecked exceptions bypass this entirely. In Python, nothing forces the caller to handle anything.

## Pattern: converting errors with map_err

Sometimes `From` doesn't apply cleanly, and you need a custom conversion at a specific call site:

```rust
fn load_profile(path: &str) -> Result<Profile, ExportError> {
    let raw = fs::read_to_string(path)?;  // io::Error → ExportError::Io via From

    // serde_yaml returns its own error type. map_err converts manually.
    let profile: Profile = serde_yaml::from_str(&raw)
        .map_err(|e| ExportError::Config(ConfigParseError::Yaml(e)))?;

    Ok(profile)
}
```

`map_err` transforms the error before `?` propagates it. Think of it like `Option::map` but for the error side of a Result. Use it when the conversion doesn't fit a generic `From` impl.

## Pattern: adding context

Sometimes the raw error isn't enough. "No such file or directory" doesn't tell you which file in a 500-file export:

```rust
// With anyhow:
let content = fs::read_to_string(&path)
    .context(format!("Failed to read {}", path.display()))?;

// Without anyhow, using map_err:
let content = fs::read_to_string(&path)
    .map_err(|e| ExportError::Io {
        source: e,
        path: path.to_path_buf(),
    })?;
```

The second form requires your error enum to carry the extra context:

```rust
#[derive(Debug, Error)]
enum ExportError {
    #[error("IO error reading {path}: {source}")]
    Io {
        source: std::io::Error,
        path: PathBuf,
    },
    // ...
}
```

Named fields in enum variants (like struct fields) let you attach rich context to errors.

## Pattern: early return for validation

Not every error comes from a function call. Sometimes you're validating and need to produce an error yourself:

```rust
fn validate_profile(profile: &Profile) -> Result<(), ExportError> {
    if profile.name.is_empty() {
        return Err(ExportError::InvalidProfile("name cannot be empty".into()));
    }

    if !profile.root_dir_path().exists() {
        return Err(ExportError::InvalidProfile(
            format!("root dir does not exist: {}", profile.root_dir)
        ));
    }

    Ok(())  // all checks passed
}
```

`return Err(...)` is the manual version of what `?` does automatically. Use it when you're generating errors, not propagating them.

## The error handling decision tree

```
Is this a library that other code calls?
├── Yes → thiserror, define a typed error enum
│         Callers can match on variants
│
└── No → Is this application-level code (CLI, main, handlers)?
    ├── Yes → anyhow for convenience
    │         .context() for readable messages
    │
    └── Prototyping? → .unwrap() / .expect("reason")
                        Panics on error (crashes the program)
                        Fine for tests and throwaway code
                        Never in production paths
```

## unwrap and expect: the escape hatches

```rust
// unwrap: panic with a generic message if Err
let content = fs::read_to_string("file.txt").unwrap();

// expect: panic with YOUR message if Err
let content = fs::read_to_string("file.txt")
    .expect("config file must exist at this point");
```

Both crash the program on error. The difference:

- `unwrap()` prints the error but gives no context about why you thought it was safe.
- `expect("reason")` documents your assumption. When it panics, you see the reason.

```
// unwrap panic output:
thread 'main' panicked at 'called `Result::unwrap()` on an `Err` value: Os { ... }'

// expect panic output:
thread 'main' panicked at 'config file must exist at this point: Os { ... }'
```

`expect` is always preferred over `unwrap` because it communicates intent. But both are escape hatches for prototyping and tests, not production error handling.

### The Python/Java parallel

```python
# Python: similar to unwrap (crash with a traceback if it fails)
content = open("file.txt").read()  # FileNotFoundError, no handling

# Python: similar to expect (crash with a custom message)
assert os.path.exists("file.txt"), "config file must exist at this point"
```

```java
// Java: similar to unwrap
String content = Files.readString(Path.of("file.txt"));  // throws, unhandled

// Java: similar to expect
assert Files.exists(Path.of("file.txt")) : "config file must exist";
```

The difference in Rust: the compiler warns you when you have an unused `Result`. You have to actively choose to ignore it. In Python and Java, ignoring a potential failure is the default behavior.

## Summary

| Concept | C | Java | Python | Rust |
|---------|---|------|--------|------|
| Error representation | Int codes, errno | Exception class hierarchy | Exception class hierarchy | `Result<T, E>` enum |
| Unifying errors | Convention | Common base exception or throws list | Catch broad `Exception` | Custom error enum + `From` |
| Propagation | Manual `if` checks | `throw` (implicit stack unwinding) | `raise` (implicit stack unwinding) | `?` operator (explicit, zero cost) |
| Adding context | Comments, maybe fprintf | Exception chaining (`initCause`) | `raise X from Y` | `.context()` or `map_err` |
| Forced handling | No | Checked exceptions (routinely bypassed) | No | Yes, always (compiler enforces) |
| Escape hatch | `assert` | `catch (Exception e) {}` | `except: pass` | `.unwrap()` / `.expect()` |
| Library convention | Return code per project | Custom exception hierarchy | Custom exception hierarchy | `thiserror` derive |
| App convention | `perror` and exit | `catch` at top level | `try/except` at top level | `anyhow` with `.context()` |
