<!-- docs/RECIPE.md -->
# Recipe language reference

A recipe tells export which repo files to copy, which transforms to run on
them, and how watch should bring returned files back. This page is the
reference for the language: its lexical rules, every instruction, how values
substitute, how paths map, how INVOKE composes recipes, and what lint warns
about.

Every example in a `recipe` block uses only the five builtin transforms
(`flatten`, `pack`, `enrichment-injection`, `enrichment-trim`, and
`context-manifest`), so it saves and lints clean on a fresh database. One
example near the end is labeled **illustrative**; it uses custom transforms
and does not save until those exist.

Check any recipe file without saving it:

```bash
flatten recipe lint path/to/file.recipe
```

## A first recipe

This is the shipped default, which `flatten recipe new <name>` copies:

```recipe
ARG repo
COPY_DEFAULT_WITH [enrichment-injection --template-set default]
SOURCE ${repo}:
  COPY . ${repo}/ AS all-files
RUN flatten
RUN context-manifest
```

- `ARG repo` declares a value the binding supplies.
- `COPY_DEFAULT_WITH` sets the per-file transform chain for the COPY blocks
  after it.
- `SOURCE ${repo}:` names the repo to read from. Its COPY block copies the
  whole repo (`.`) under a `${repo}/` prefix, as the block keyed `all-files`.
- The two `RUN` lines run directory transforms over the assembled output.

## Lexical rules

### Files and lines

| Rule | Detail |
|---|---|
| Encoding | UTF-8. The CLI rejects other encodings before parsing. |
| Byte order mark | A leading BOM is skipped and does not count toward line 1 columns. The stored text keeps it. |
| Line endings | LF or CRLF. The stored text is kept verbatim. |
| Positions | 1-based line and column. Columns count characters, not bytes. Positions always refer to physical lines, even inside a continued line. |
| Whitespace | Space and tab separate tokens. |
| Indentation | Leading spaces only. A tab in leading whitespace is an error at that line and column. A tab elsewhere separates tokens; inside quotes it is literal. |
| Blank lines | Blank and comment-only lines produce nothing and do not affect indentation. A comment-only line may be indented with tabs. |
| Empty recipe | Empty text, and text with only ARGs, are valid. |

### Comments

`#` starts a comment to the end of the line when it is outside quotes and
either starts the content or follows whitespace. Inside a bare token it is
literal: `foo#bar` is one token, and `"a # b"` keeps its `#`.

### Continuation

A trailing `\` joins the next physical line:

1. Strip the comment.
2. Trim trailing whitespace.
3. If the line now ends in `\` outside quotes, join the next line. Its
   leading whitespace is dropped, and one space separates the joined parts.

```recipe
COPY_DEFAULT_WITH [enrichment-injection --template-set default]
SOURCE r:
  COPY . out/ AS all:
    EXCLUDE *.log node_modules/ \
      --binary
```

- The logical line's indentation is the first physical line's.
- A quoted string cannot span lines. An open quote at the end of a line is
  an error, even before a `\`.
- A `\` at the end of the file, or before a blank or comment-only line, is
  an error at the `\`.
- A line cannot end in a literal backslash, since that would continue the
  line. Quote it: `ARG sep="\\"`.

### Tokens and quoting

Tokens are separated by whitespace. A token can mix bare, quoted, and
variable parts with no space between them, as in a shell:
`msg="hello world"` is one token.

- **Bare text** is any characters except whitespace, `"`, and the `${`
  opener. Backslash is literal in bare text, so gitignore escapes like `\#`
  survive.
- **Quoted text** is `"..."`. The escapes are `\"`, `\\`, `\n`, and `\t`;
  any other escape is an error.
- **A variable** is `${NAME}`, in bare or quoted text. `NAME` matches
  `[A-Za-z_][A-Za-z0-9_]*`. A `$` not followed by `{` is literal. A literal
  `${` cannot be written.

### Keywords and names

| Kind | Rule |
|---|---|
| Keywords | Uppercase, case-sensitive, and fully bare: `ARG`, `COPY_DEFAULT_WITH`, `SOURCE`, `RUN`, `INVOKE`, `WATCH`, `COPY`, `AS`, `EXCLUDE`, `OVERRIDE_WITH`, `DEPTH_TOLERANCE`, `OVERRIDE`. `copy` is an unknown instruction. |
| ARG names | `[A-Za-z_][A-Za-z0-9_]*` |
| Transform and recipe names | `[A-Za-z0-9][A-Za-z0-9_.-]*`, written bare: no quotes, no variables |
| Version pins | `@N` after a name, `N` a decimal >= 1. Only on `RUN` and `INVOKE`. |
| Transform flags | `--` followed by `[a-z0-9][a-z0-9-]*` |
| COPY keys | Any non-empty text with no control characters, checked after substitution |

## Structure

### Indentation

Children are indented more than their parent, with spaces. There is no
fixed width: a block's first child sets the indentation every later sibling
must match. Indenting a line that has no parent, or indenting siblings
unevenly, is an error.

### The block colon

- The block keywords `SOURCE`, `COPY`, `WATCH`, and `OVERRIDE` end with `:`
  when they have children. The colon can touch the last token (`AS k:`) or
  stand alone (`AS k :`). `WATCH:` and `OVERRIDE:` work as single tokens.
- A block keyword with children and no colon is an error at the keyword.
- A block keyword with a colon and no children is allowed. `SOURCE` still
  needs at least one `COPY`.
- On any other instruction a trailing colon is literal: `EXCLUDE foo:`
  excludes the pattern `foo:`.
- A quoted `":"` never opens a block.
- Any other instruction with children is an error at the first child.

### Where instructions go

| Context | Allowed |
|---|---|
| Top level | `ARG`, `COPY_DEFAULT_WITH`, `SOURCE`, `RUN`, `INVOKE`, `WATCH` (at most one) |
| Inside `SOURCE` | `COPY` (at least one) |
| Inside `COPY` | `EXCLUDE` (any number), `OVERRIDE_WITH` (at most one) |
| Inside `WATCH` | `DEPTH_TOLERANCE` (at most one), `OVERRIDE` (at most one) |
| Inside `OVERRIDE` | `<key> <chain>` lines, one per key |

A COPY, EXCLUDE, OVERRIDE_WITH, DEPTH_TOLERANCE, or OVERRIDE outside its
block is an error that names the block it belongs in.

## Instruction reference

### ARG

```text
ARG <name>[=<default>]
```

Declares a value. With no default, the ARG is required: the binding (or an
INVOKE) must supply it. `ARG x=` sets an empty default. A default may use
only ARGs declared above it. Declaring the same ARG twice is an error.

```recipe
ARG repo
ARG sub=src
ARG label=${repo}-${sub}
```

### COPY_DEFAULT_WITH

```text
COPY_DEFAULT_WITH <chain>
```

Sets the chain for every COPY block after it, until the next
COPY_DEFAULT_WITH. Before the first one, the default chain is empty.
`COPY_DEFAULT_WITH []` clears it.

### SOURCE

```text
SOURCE <repo>:
  COPY ...
```

Names the repo the COPY blocks read from. The name may use variables, and
it must not be empty after substitution. Whether the repo exists is checked
when a binding resolves it, not here.

### COPY

```text
COPY <src> <dest> AS <key>[:]
  EXCLUDE ...
  OVERRIDE_WITH ...
```

Copies `src` (in the repo) to `dest` (in the export). Exactly four tokens;
the third is the bare keyword `AS`. See Paths and COPY shapes for what the
paths mean.

The key names the block for watch and for WATCH OVERRIDE. Keys may use
variables and must be unique across the whole recipe after substitution,
including recipes it invokes. A duplicate key is an error that shows both
locations.

### EXCLUDE

```text
EXCLUDE <pattern>... | --binary
```

Gitignore patterns, matched against `src`-relative paths. One line can hold
several patterns, and a COPY can have several EXCLUDE lines.

- `--binary` excludes files whose extension is in the `binary_extensions`
  setting. It can mix with patterns on one line.
- Any other bare token starting with `--` is an unknown flag. Quote it to
  mean a pattern: `EXCLUDE "--draft"`.
- A pattern must be one the gitignore matcher accepts, and it must match
  something. A blank pattern, or one that starts with `#`, is an error;
  write `\#notes` for a file named `#notes`.

```recipe
COPY_DEFAULT_WITH [enrichment-injection --template-set default]
SOURCE r:
  COPY . out/ AS all:
    EXCLUDE *.log node_modules/ --binary
    EXCLUDE "--draft" \#notes
```

### OVERRIDE_WITH

```text
OVERRIDE_WITH <chain>
```

Replaces the default chain for its COPY only. `OVERRIDE_WITH []` means no
transforms, which lint reports as L005 (the files cannot round-trip).

### RUN

```text
RUN <transform>[@N] [--flag value ...] [--only <glob> ...]
```

Runs a directory transform over the assembled output, at its current
version or pinned with `@N`. Flags follow the chain flag rules.

`--only` limits the run to matching paths. It takes every following token
up to the next `--` flag or the end of the line, and it may repeat. The
globs never reach the transform as an argument.

```recipe
ARG repo
SOURCE ${repo}:
  COPY . ${repo}/ AS all-files:
    OVERRIDE_WITH [enrichment-injection]
RUN pack --format xml --only ${repo}/**/*.rs ${repo}/**/*.toml
RUN flatten
```

### INVOKE

```text
INVOKE <recipe>[@N] [<name>=<value> ...]
```

Expands another recipe in place, at its current version or pinned with
`@N`. Each `name=value` sets one of its ARGs; naming an ARG it does not
declare is an error. See INVOKE for binding and the rest.

### WATCH

```text
WATCH:
  DEPTH_TOLERANCE <n>
  OVERRIDE:
    <key> <chain>
```

At most one per recipe, anywhere at top level.

- **`DEPTH_TOLERANCE`** is how many missing path levels (the file plus its
  parent directories) watch may create when it places a returned file. It
  must be a bare decimal number, with no variables, and 0 is allowed. The
  default is 2.
- **`OVERRIDE`** replaces watch's return path for the named COPY keys. Each
  line is a key (variables allowed) and a chain. Every key must name a COPY
  block.

```recipe
ARG repo
COPY_DEFAULT_WITH [enrichment-injection --template-set default]
SOURCE ${repo}:
  COPY docs/ ${repo}/docs/ AS docs
WATCH:
  DEPTH_TOLERANCE 3
  OVERRIDE:
    docs [enrichment-trim]
```

## Transform chains and flags

```text
chain   := "[" "]" | "[" element ("," element)* "]"
element := NAME flag*
flag    := "--" FLAG ( "=" value | value )?
```

- `--k v` and `--k=v` set `k` to `v`. A bare `--k`, followed by `,`, `]`,
  another flag, or the end of the line, sets `k` to `true`. Values may be
  quoted and may use variables.
- A flag repeated within one element is an error.
- A token that is neither a flag nor a flag's value is an error.
- Version pins are not allowed in chains.
- A missing `]`, a trailing `,`, `,,`, text after `]`, or no chain at all is
  an error. `[]` is the only way to write "no transforms".
- Chain transforms must be file transforms; RUN transforms must be directory
  transforms. Either mismatch is an error naming both scopes.
- Every name must exist when the recipe is saved. An unknown transform or
  recipe is an error, not a warning.

## Substitution

`${NAME}` substitutes in:

- the SOURCE repo
- COPY `src`, `dest`, and key
- EXCLUDE patterns
- flag values, in chains and on RUN
- RUN `--only` globs
- INVOKE values
- WATCH OVERRIDE keys

It does not substitute in keywords, ARG names, transform and recipe names,
version pins, flag names, or `DEPTH_TOLERANCE`. Names must resolve when the
recipe is saved, so they cannot depend on ARG values.

Rules:

- Lines run in file order. Using an ARG before its declaration is an error
  at the variable.
- Values are never rescanned, so a value containing `${x}` stays literal.
- Every value is substituted before it is checked, and export records the
  final strings.

### Saving vs exporting

A recipe is checked twice:

- **When saved, shown, or linted**, required ARGs have no value yet. They
  stay symbolic: `${repo}` passes through as text, and checks that depend on
  the value wait. For example, an EXCLUDE pattern that still holds a
  variable is not validated yet.
- **At export**, every ARG has a value, and every check runs on the final
  strings.

Saving never rejects a recipe that a binding supplying its required ARGs
would accept.

**ARG defaults must be valid on their own.** When a recipe is saved, a
default is a real value, and every check runs on it. A binding can override
a default, and export checks the new value. To say "the binding must supply
this", declare the ARG with no default.

## Paths and COPY shapes

### Normalization

`src` and `dest` are relative paths, normalized after substitution:

| Written | Result |
|---|---|
| `.`, `./`, `.//` | the root (empty) |
| `./src//app/` | `src/app/` |
| `src/./app` | `src/app` |
| `src/.` | `src/` (a trailing `.` means "directory") |
| `src/app` | `src/app` (no slash is added) |

These are errors:

- an empty path
- a control character
- a backslash
- a leading `/`, or a drive-letter prefix like `C:`
- any `..` segment

Case and Unicode are kept as written.

### Prefix and exact

The trailing slash carries meaning, so normalization keeps it:

- **Prefix:** a COPY is a directory prefix when either side is empty or
  either side ends in `/`. Every file under `src` maps to the same remainder
  under `dest`.
- **Exact:** with both sides non-empty and neither ending in `/`, the COPY
  maps one exact path.

A prefix COPY records both sides with a trailing `/` (an empty side stays
empty), so `src` plus a remainder always meets on a segment boundary:

| As written | Recorded `src` | Recorded `dest` | Match |
|---|---|---|---|
| `COPY . ${repo}/` | (empty) | `${repo}/` | prefix |
| `COPY . out` | (empty) | `out/` | prefix |
| `COPY src dest/` | `src/` | `dest/` | prefix |
| `COPY src/ dest` | `src/` | `dest/` | prefix |
| `COPY src .` | `src/` | (empty) | prefix |
| `COPY README.md .` | `README.md/` | (empty) | prefix |
| `COPY README.md docs/` | `README.md/` | `docs/` | prefix |
| `COPY README.md docs/README.md` | `README.md` | `docs/README.md` | exact |

**To copy one file, name it on both sides.** `COPY README.md .` and
`COPY README.md docs/` become directory prefixes, which match nothing when
`README.md` is a file.

## INVOKE

`INVOKE base` expands recipe `base` where the line is, as if its
instructions were written there, but in its own scope.

### Binding

Each ARG in the invoked recipe, in its file order, takes the first of:

1. a `name=value` on the INVOKE line, substituted in the caller
2. the caller's ARG of the same name, if declared above the INVOKE
3. the invoked recipe's own default
4. otherwise an error at the INVOKE line, naming both recipes and where the
   invoked ARG is declared

A symbolic caller value (a required ARG with no value yet) passes through as
symbolic.

The shipped default's required `repo` binds from the caller's own ARG here
(step 2):

```recipe
ARG repo
INVOKE shipped-default
```

and from the INVOKE line here (step 1):

```recipe
INVOKE shipped-default repo=archive
```

Invoking one recipe twice works when its COPY keys substitute. The
`base.recipe` fixture keys its COPY `${repo}-files`, so `INVOKE base repo=a`
followed by `INVOKE base repo=b` produces `a-files` and `b-files`. The
shipped default's key is the literal `all-files`, so invoking it twice is a
duplicate key.

### Isolation

- The invoked recipe's ARGs are not visible to the caller.
- COPY_DEFAULT_WITH flows neither in nor out: the invoked recipe starts with
  an empty default chain, and its own setting ends with it.
- COPY keys are global: a key defined in an invoked recipe collides with the
  same key anywhere else.

### Cycles and limits

- **Cycles.** An INVOKE that reaches a recipe version already on the current
  expansion path is a cycle, reported with the whole path:
  `INVOKE cycle: a@2 -> b@1 -> a@2`. The recipe being saved is on the path
  from the start, so a recipe that invokes itself is a cycle
  (`cyc@pending -> cyc@pending`), not an unknown recipe.
- **Not cycles.** Invoking the same recipe twice in sequence is legal, and so
  is `a@2` invoking `a@1`.
- **Depth.** INVOKE nests at most 100 levels.
- **Size.** One recipe expands to at most 10,000 SOURCE, COPY, and RUN
  instructions in total. That limit catches fan-out (A invokes B twice, B
  invokes C twice, and so on), which depth alone does not.

### WATCH across INVOKE

- **DEPTH_TOLERANCE.** An invoked recipe's DEPTH_TOLERANCE is ignored, with
  warning L006. Only the recipe you export sets it.
- **OVERRIDE.** OVERRIDE entries from every recipe merge into one map. For
  each key, the entry closest to the recipe you export wins. Between two
  recipes at the same depth, the one expanded first wins. Every entry that
  loses is warning L007. A caller wins even when its WATCH comes after its
  INVOKE.
- **Keys.** An OVERRIDE key must name a COPY block somewhere in the
  expansion. An unknown key is an error at the entry, in whichever recipe
  holds it.

## Lint warnings

Warnings never stop a save. `flatten recipe lint` prints them on stdout.
`add`, `new`, `edit`, `show`, and `rollback` print them on stderr as
`warning: ...`.

| Code | Means |
|---|---|
| L001 | A transform in a COPY chain has nothing that reverses it, so watch cannot undo it when a file returns. |
| L002 | A WATCH OVERRIDE drops `enrichment-trim` while the chain injects enrichment, so returned files keep their enrichment. |
| L003 | A WATCH OVERRIDE differs from the default return path. The message shows both. |
| L004 | A RUN transform declares that it reverses something. Directory transforms are one-way. |
| L005 | A COPY chain has no `enrichment-injection`, so watch cannot match its files and they will not round-trip. |
| L006 | An invoked recipe's DEPTH_TOLERANCE was ignored. |
| L007 | A WATCH OVERRIDE entry was replaced by one closer to the exported recipe (or expanded earlier). |
| L008 | A transform has several reversers, so its return path is ambiguous. The message names every candidate. |

### The default return path

L001, L002, L003, and L008 compare a COPY chain with what watch runs when a
file comes back, with no OVERRIDE:

1. **Trim.** If the chain has `enrichment-injection` (without
   `--reversible=false`), `enrichment-trim` runs first.
2. **Reverse.** Then, for each other transform in the chain, last to first,
   watch runs the transform that reverses it. A transform with no reverser
   is skipped (L001). One with several is ambiguous (L008).

An OVERRIDE replaces both steps for its key. L003 compares it with that
path, in that order, and leaves out skipped transforms. A path with an
ambiguous step is not compared.

Because the trim comes first, an override written in pure reversal order
draws L003. This is the design doc's example, **illustrative (uses custom
transforms)**: it does not save on a fresh database, because
`strip-vendor-headers` and `custom-vendor-restore` do not exist yet.

```text
SOURCE ${repo}:
  COPY vendor/ ${repo}/vendor/ AS vendor-files:
    OVERRIDE_WITH [strip-vendor-headers, enrichment-injection --template-set vendor]
WATCH:
  OVERRIDE:
    vendor-files [custom-vendor-restore, enrichment-trim --template-set vendor]
```

If `custom-vendor-restore` reverses `strip-vendor-headers`, the default
return path is `[enrichment-trim, custom-vendor-restore]`, and this override
draws L003. Write it trim first to match.

### Where warnings point

- **COPY-level warnings** (L001, L002, L003, L005, and L008) point at the
  COPY block, not at an OVERRIDE line, because the merged WATCH settings do
  not keep line positions. The message names the key.
- **L004** points at the RUN line, **L006** at the ignored DEPTH_TOLERANCE,
  and **L007** at the replaced OVERRIDE entry.
- **In invoked recipes**, warnings carry the recipe and version:
  `L006 base@2 5:3: ...`.
- **Order.** Warnings sort with the exported recipe first, then invoked
  recipes in the order they were first expanded, then by position.
- **Duplicates.** Identical warnings collapse to one. A recipe invoked twice
  reports its ignored DEPTH_TOLERANCE once.

## Errors

An error stops the save or export. It prints as `error: <where>: <what>`:

```text
error: 2:1: unknown instruction 'COPIE'
error: 3:10: invalid path "../out/": '..' segments are not allowed
error: 2:1: COPY must be inside a SOURCE block
error: 4:17: duplicate COPY key "dup" (first defined at 3:17)
error: 3:1: INVOKE shipped-default@1: required ARG repo (declared at 1:1) has no value; bad-invoke-unbound neither passes repo= nor declares ARG repo
error: 3:1: INVOKE cycle: cyc@pending -> cyc@pending
```

(These are the `bad-*` fixtures in `fixtures/recipes/`. Each starts with a
comment line, so the errors start at line 2.)

### Where errors point

- **In the recipe itself:** `line:col`, as `3:5`.
- **In an invoked recipe:** that recipe, its version, and the INVOKE lines
  that led there, starting from the recipe you saved:
  `base@2 3:5 (via INVOKE at top 7:1)`, or nested,
  `leaf@1 2:1 (via INVOKE at top 7:1 -> base@2 4:1)`. The first INVOKE site
  shows the recipe being saved by name only, with no version (`<input>` when
  linting a file).
- **Specific positions:**
  - An unknown or wrong-scope RUN transform points at `RUN` (column 1).
  - A chain error about one element (unknown name, wrong scope, pin) points
    at that element's name.
  - A trailing comma points at the `]`.
  - A missing `]`, or no chain at all, points at the keyword.
  - A cycle, an unknown recipe, or an unbound ARG points at the INVOKE line.
  - A naming error in an INVOKE assignment points at the assignment.

### JSON errors

With `--json`, errors go to stderr as one object with a `kind` (`domain`,
`database`, `io`, or `usage`). A parse error adds `detail`:

```json
{"detail":{"col":1,"line":2,"recipe":null,"version":null},"error":"2:1: unknown instruction 'COPIE'","kind":"domain"}
```

`recipe` and `version` name the invoked recipe that holds the error. They
are `null` when the error is in the recipe itself.

Exit codes: 0 on success (warnings included), 1 on an error, and 2 on a
usage error.

## Saving and versions

| Command | Does |
|---|---|
| `flatten recipe add <file> --name <n>` | Saves a new recipe at version 1 |
| `flatten recipe new <n>` | Saves a new recipe from the shipped default |
| `flatten recipe edit <n> <file>` | Saves the next version and makes it current; identical text changes nothing |
| `flatten recipe rollback <n> <version>` | Makes an earlier version current, then reports any problem it now has without failing |
| `flatten recipe show <n> [--raw]` | Shows the resolved recipe, or with `--raw`, the stored text byte for byte |
| `flatten recipe args <n>` | Lists the recipe's ARGs |
| `flatten recipe history <n>` | Lists versions, marking the current one |
| `flatten recipe ls` | Lists recipes |
| `flatten recipe rm <n>` | Removes a recipe; its name stays taken |
| `flatten recipe lint <target>` | Lints a file, or a saved recipe by name |

Saved text is kept exactly as written, comments and all. Recipe names follow
the transform name rule and stay taken after `rm`, so a removed recipe's
history is never reused under a new recipe. The shipped default cannot be
removed, but it can be edited.
