# Spec 50: crapts Oxc analysis and names

**Status:** Implemented
**Effort:** Large
**Module:** `crates/crapts/{Cargo.toml,src/lib.rs,src/discover.rs,src/analyze.rs,src/lines.rs,src/names.rs,tests/analysis.rs,tests/discovery.rs,tests/names.rs,tests/properties.rs}`, `Cargo.toml`, `Cargo.lock`, `tests/workspace.rs`, `tests/fixtures/spec45-package-ids.txt`, `tests/golden/config.rs`, `tests/golden/gates.rs`, `scripts/gates/parser-guard.sh`, `scripts/gates/test.sh`, `.github/workflows/ci.yml`, `.github/dependabot.yml`

## Context

A maintainer needs to recognize the same JavaScript or TypeScript function after moving it, formatting it or editing a callback inside it. A later coverage reader needs to know which source regions belong to that function. The `crapts` analyzer supplies one row per scored unit, with its file, source location, owned regions and statement lines, cyclomatic complexity (CC) and scheme-1 name. A file can contain zero rows. This contract takes source files as input and can be tested without coverage or a CLI.

## Acceptance Tests

### Scenario: Discover the ordinary Git file set

```gherkin
Given a worktree with tracked "src/a.ts", deleted tracked "src/gone.ts",
  untracked "src/b.js" and ignored untracked "src/tmp.js"
And src/sub is a gitlink and src/embedded is an untracked nested repository, each containing child.ts
And .gitignore now matches tracked "src/a.ts" and .gitattributes marks it linguist-generated
When discovery runs from the repository root with literal roots "src" and "src/a.ts"
Then the paths are exactly ["src/a.ts", "src/b.js"], in that order
And gitlinks and untracked embedded repositories contribute no descendants
And a linguist-generated attribute on "src/a.ts" leaves it eligible
```

An existing regular file tracked in the outer index remains eligible when its directory acquires a `.git` entry. Sort displayed paths in UTF-16 code-unit order.

### Scenario: Resolve input roots and preserve path spelling

```gherkin
Given a worktree with "pkg/[x]/a b.ts", "pkg/x/a b.ts", and "shared/café.ts"
When discovery starts in "pkg" with roots "[x]", "[x]/a b.ts" and "../shared"
Then the sorted, slash-separated paths are ["../shared/café.ts", "[x]/a b.ts"]
And specifying the same roots as absolute paths gives the same result
And with no roots the result is ["[x]/a b.ts", "x/a b.ts"]
```

### Scenario: Walk regular files outside a Git worktree

```gherkin
Given an ordinary directory containing "real/a.ts", ".git/info/a.ts",
  and "deps/node_modules/a.ts"
When discovery walks "." with an empty list of exclusions
Then the result is ["real/a.ts"]
And running the same scan with git removed from PATH also returns ["real/a.ts"]
```

### Scenario: Skip symbolic links to files and directories

```gherkin
Given "real/a.ts", a file symlink "link.ts" to it, and a directory symlink "alias" to "real"
When scanning "." with and without a Git worktree
Then each scan selects only "real/a.ts"
And selecting the two links as explicit roots selects zero files
```

Run this fixture on platforms with symlink creation enabled. All other discovery fixtures also run on Windows without symlink privileges.

Check the named leaf with filesystem metadata. Outside Git, an explicit `alias/a.ts` below a directory link selects the regular file with that spelling. In a worktree, an outer-index path still selects a regular file if its parent becomes a link. A link loop that prevents a leaf lookup returns an input error.

Resolve roots from the invocation directory. When a physical cwd is under `/private/tmp` and the absolute argument uses `/tmp`, preserve its lexical spelling. Outside Git the display path begins with the necessary `../` components. A Git worktree rejects that absolute argument when the spelling puts it outside the repository. A symlinked invocation inside a worktree also accepts Git's physical spelling of an absolute root.

### Scenario: Select the eight source extensions case-sensitively

```gherkin
Given one file "a.<suffix>" for each suffix in
  ts, mts, cts, tsx, js, mjs, cjs, jsx, json, d.ts, d.mts, d.cts, d.ts.js
And a separate file "b.TS"
When the directory is discovered
Then the result is ["a.cjs", "a.cts", "a.d.ts.js", "a.js", "a.jsx",
  "a.mjs", "a.mts", "a.ts", "a.tsx"]
```

### Scenario: Apply default and caller-supplied exclusions

```gherkin
Given the following default path globs
  **/node_modules/**   **/dist/**       **/build/**      **/coverage/**
  **/*.test.*          **/*.spec.*      **/*.stories.*   **/*.min.js
  **/__tests__/**      **/__mocks__/**
And a worktree containing "src/main.ts", "src/gen/a.ts", and these ten regular files
  node_modules/p/a.js   pkg/dist/a.ts     build/b.ts        coverage/a.js
  lib/a.test.ts         a.spec.mts        src/.hidden/test.stories.tsx
  src/a.min.js          __tests__/a.cts   src/__mocks__/a.jsx
When discovery applies those defaults and an extra "src/gen/**" from the working directory
Then only "src/main.ts" is selected
And replacing the defaults with an empty list admits those ten default matches
  while "src/gen/a.ts" stays excluded
```

### Scenario: Explicit file roots use the same filters as directory roots

```gherkin
Given regular files "only.json" and "one.spec.ts" and the default exclusions
When discovery is called with those two files as its roots
Then discovery succeeds with an empty file list
```

### Scenario: Match path globs and account for unused exclusions

```gherkin
Given files "src/.gen/a.ts", "src/A.ts", "pkg/src/.gen/b.ts", "src/tag.md" and "dist/a.ts"
And defaults exclude "dist/**"
And the extra patterns in order are "src/*/[ab].{ts,js}", "**/.gen/a.ts",
  "src/?.ts", "src/tag.md", "dist/**" and "absent/**"
When discovery filters the paths relative to the working directory
Then the only selected file is "pkg/src/.gen/b.ts"
And the unused extra patterns in order are "src/tag.md", "dist/**" and "absent/**"
```

Use case-sensitive, whole-path globset matching on `/`-separated paths, including dot-prefixed components. `*` and `?` stay within a component. A `**` component can cross directories. Character classes, brace alternatives and backslash escapes use globset syntax. A leading `!` is literal. Validate all patterns, even for an empty scan. Every extra pattern matching an otherwise eligible file counts as used, including overlapping matches. Each unused entry retains its pattern and origin for the caller.

A recursive wildcard can cross ordinary directories but cannot consume a leading `..` component. A head component starting with `.*` or containing a brace group also cannot consume it. From `pkg`, the rules `**/gen/**`, `**/.*/**`, `.*/**` and `{..,x}/**` leave `../shared/gen/a.ts` eligible. The rule `../shared/gen/**` excludes it. Apply the same rule to historical paths and to unused-pattern accounting.

### Scenario: Apply discovery scope to old paths

```gherkin
Given root "src" in a worktree with selected "src/live.ts", ignored "src/cache/**",
  an embedded repository "src/vendor", and the default exclusions
When the discovery result checks current and absent paths against this scope
Then "src/live.ts" and absent "src/old.ts" are in scope
And "other/old.ts", "src/cache/old.ts", "src/vendor/old.ts", "src/gone.test.ts"
  and "src/old.d.ts" are out of scope
```

Spec 50 supplies the selection predicate and unused-pattern data. A file root matches only itself. A directory root also admits eligible former paths beneath it, subject to the same ignores, exclusions and traversal boundaries. Outside Git, an absent path beneath a directory symlink under a directory root is outside scope, just as it is in a worktree. Spec 53 presents discovery notices and Spec 54 applies the predicate to baseline rows.

### Scenario Outline: Report discovery failures

```gherkin
Given a discovery request with <condition>
When discovery runs
Then it returns an input error naming <subject>

Examples:
  | condition                                  | subject                       |
  | an explicit path "gone.ts" that is absent  | gone.ts                       |
  | a root "../other" outside the worktree     | ../other and the worktree root |
  | the malformed glob "src/[" in an empty dir | src/[                         |
  | the disposable worktree has a corrupt index | git ls-files                  |
```

### Scenario: Choose the source grammar from the path

```gherkin
Given "const B = () => <b/>" in separate .js, .mjs, .cjs, .jsx and .tsx files
And "function f(x: number) { return x }" in separate .ts, .mts and .cts files
When each file is analyzed in its own source grammar
Then each JSX fixture returns B and each typed fixture returns f, all with zero warnings
And an empty .js file succeeds with zero rows and zero warnings
```

Use JavaScript with JSX for `.js`, `.mjs`, `.cjs` and `.jsx`, TypeScript without JSX for `.ts`, `.mts` and `.cts`, and TSX for `.tsx`. Preserve the extension's module kind.

### Scenario: Identify arrow functions for coverage attribution

```gherkin
Given "const concise = () => 1; const block = () => {}; const expr = function expr() {};" in arrows.ts
And "function decl() {}" in the same file
When the analyzer emits rows for these four functions
Then is_arrow is true for concise and block
And is_arrow is false for expr and decl
```

### Scenario: Reject syntax that belongs to a different source grammar

```gherkin
Given "const B = () => <b/>" in "wrong.ts" and "function f(x: number) {}" in "wrong.js"
When each file is analyzed in strict mode on the head side
Then each returns a parse error naming head and its own path
```

### Scenario: Reject invalid UTF-8 source bytes

```gherkin
Given the bytes 66 80 0A in "invalid.ts"
When the analyzer reads those bytes in ordinary or strict mode
Then it returns a decoding error identifying "invalid.ts" and invalid UTF-8
And a strict error also identifies the requested base or head side
```

### Scenario: A UTF-8 BOM counts toward source columns

```gherkin
Given the bytes EF BB BF followed by the UTF-8 text "function f() {}"
When the analyzer reads bom.ts
Then it returns f at [1:1, 1:16) with zero warnings
```

### Scenario Outline: Count each decision in its owning parameters or body

```gherkin
Given a .ts file containing "function f(x, y, xs) { <body> }"
When the analyzer measures f
Then its cyclomatic complexity is <CC>

Examples:
  | body                                               | CC |
  | return x                                           |  1 |
  | if (x) {} else if (y) {} else {}                   |  3 |
  | return x ? y : 0                                   |  2 |
  | for (;;) { break }                                 |  2 |
  | for (const k in xs) {}                             |  2 |
  | for (const v of xs) {}                             |  2 |
  | while (x) { break }                                |  2 |
  | do {} while (x)                                    |  2 |
  | switch (x) { case 1: case 2: break; default: }     |  3 |
  | try {} catch {} finally {}                         |  2 |
  | return x && y                                      |  2 |
  | return x \|\| y                                    |  2 |
  | return x ?? y                                      |  2 |
  | x &&= y                                            |  2 |
  | x \|\|= y                                          |  2 |
  | x ??= y                                            |  2 |
  | x = y; return x?.a?.b                              |  1 |
  | type R<T> = T extends string ? 0 : 1; return x     |  1 |
```

Gherkin table cells escape a literal `|` as `\|`. The source program contains the operator `||` or `||=`.

### Scenario: Separate nested decisions and skip declarations without bodies

```gherkin
Given the following TypeScript
  """
  declare function f(s: string): void;
  function outer(x = a ?? b) {
    function local(s: string): void;
    function local(s: string) { if (s && x) return }
    return (y = x ? 1 : 2) => y
  }
  abstract class P { abstract work(): void }
  """
When the file is analyzed
Then the complete set of (name, CC) is (outer, 2), (outer::local, 3), (outer::<return>, 2)
And outer owns exactly statement-start lines 4 and 5
And the bodyless declarations contribute no statement lines or ordinals
```

### Scenario: Distinguish source regions from their outer line range

```gherkin
Given the following source with the first line numbered 1
  """
  // header
  function outer() {
    return () => {
      if (ready) run()
    }
  }
  """
When the rows and their owned statement-start lines are requested
Then outer has span [2:0, 6:1), body region [2:17, 6:1), CC 1 and statement lines [3]
And outer::<return> has span [3:9, 5:3), body region [3:15, 5:3), CC 2 and statement lines [4]
And in "/*😀*/() => 1" the arrow has span [1:6, 1:13) and concise-body region [1:12, 1:13)
```

Positions use one-based lines and zero-based UTF-16 columns. Spans and regions include their start and exclude their end. Strip common Gherkin doc-string indentation before counting. Each row records the sorted, distinct start lines of executable statements encountered in its own body. Blocks, empty statements, type-only declarations and declarations without bodies contribute no starts. A nested function declaration with a body contributes its declaration line to the enclosing body. Statements inside that function belong to its own row. A concise arrow expression or field initializer also contributes its expression start.

### Scenario Outline: Count JavaScript line terminators

```gherkin
Given three source lines "function f() {", "  return 1" and "}" separated by <break>
When the analyzer measures the return statement and function end
Then the statement starts at 2:2 and f ends at 3:1

Examples:
  | break              |
  | LF                 |
  | CRLF as one break  |
  | CR                 |
  | U+2028             |
  | U+2029             |
```

Score function declarations and expressions with bodies, arrows, methods, constructors, getters, setters, static blocks and static initializer expressions. A direct function-valued or class-valued field supplies a function row or a class container. Other static initializer expressions each get a field row. Every initialized instance field contributes a region to the constructor. Create an implied constructor exactly when a class has an initialized instance field and lacks a written constructor with a body. A written constructor's reported span is its own declaration through its closing brace. Instance initializer regions before or after that span still belong to it.

### Scenario: Assign each class initializer to its scoring owner

```gherkin
Given this class, with its declaration on line 1
  """
  class C {
    a = p ? q : r
    static s = build(() => flag && ready)
    constructor() { if (flag) go() }
    b = xs.map(x => x ?? 0)
    static { while (p) break }
  }
  """
When analysis measures C
Then the complete set of (name, CC) is
  C::constructor                3
  C::static s                   1
  C::static s::build()          2
  C::b::xs.map()                2
  C::<static-block>             2
And C::constructor spans [4:2, 4:34) and its regions, in source order, are
  [2:6, 2:15), [4:16, 4:34), [5:6, 5:25)
And the callback on line 5 takes its name from field b, and the static block occupies [6:9, 6:28) from its opening brace
```

### Scenario: Locate an implied constructor across discontiguous initializers

```gherkin
Given this class, with its declaration on line 1
  """
  class C {
    a = p && q
    static f = () => 9
    handler = () => p ? 1 : 2
    b = r ?? s
  }
  """
When analysis measures C
Then C::constructor spans the first initializer on line 2 through the last on line 5
  and has CC 3
And the other rows are C::static f (CC 1) and C::handler (CC 2)
And a class with only uninitialized instance fields and static members has no implied constructor
```

### Scenario: Score auto-accessor initializers as fields

```gherkin
Given "class K { accessor x = f(() => 1); static accessor y = a ?? b }"
When analyzed as TypeScript
Then the complete (name, CC) set is (K::constructor, 1), (K::x::f(), 1), (K::static y, 2)
```

### Scenario: Count class definition expressions in the surrounding execution context

```gherkin
Given "function outer() { class K extends (a ? A : B) { @dec(b ? 1 : 2) [c ? u : v]() {} } }"
When the analyzer counts decisions in the heritage, decorator and computed key
Then outer has CC 4 and outer::K::(c%3Fu%3Av) has CC 1
And if the class stands at file scope, the method still has CC 1 and the file gets no synthetic row
```

Each decision belongs to the row executing that expression. A callback inside the expression owns its own body. Decorators on a method remain part of its source span.

A decorated `declare` field or auto-accessor evaluates its decorators and computed key in the surrounding context. Its initializer contributes no row, region or decision. An undecorated declared field contributes none of those definition expressions.

### Scenario: Direct class-valued fields supply a class name

```gherkin
Given "class C { static A = class B { f() {} }; other = class { g() {} } }"
When the class is analyzed
Then the only names are C::static A::f, C::other::g and C::constructor
And each row has CC 1
```

### Scenario: Return recovered rows with one warning

```gherkin
Given "recover.js" contains "const x; let [a]; function kept() { if (x) go() }"
When ordinary analysis reads the file
Then it returns kept with CC 2
And returns exactly one warning naming recover.js and the first Oxc diagnostic,
  "Missing initializer in const declaration"
```

### Scenario: Skip a fatal parse and continue the ordinary scan

```gherkin
Given "broken.ts" contains "function partial() { if (x) {" and "good.ts" contains "const ok = () => 1"
When ordinary analysis reads both files
Then only ok is returned
And the result for broken.ts carries one warning naming broken.ts and its parse failure
```

### Scenario Outline: A changed-file comparison requires a clean parse on both sides

```gherkin
Given <side> supplies "src/input.ts" with a <parse> parse
When that side is analyzed for comparison
Then the analyzer returns an input error identifying <side> and "src/input.ts"

Examples:
  | side | parse       |
  | base | recoverable |
  | base | fatal       |
  | head | recoverable |
  | head | fatal       |
```

Spec 55 maps this input error to exit status 2 at the changed-gate CLI boundary.

### Scenario: Compose scheme-1 names from ancestors and sites (N1-N3)

```gherkin
Given a row enclosed by rows, classes, TypeScript namespace or module blocks, and naming sites
When its scheme-1 name is requested
Then each enclosing row, class, namespace component and unabsorbed site contributes
  its segment, outermost first, joined by "::"
And namespace A.B contributes A::B and a string module "pkg-x" contributes "pkg-x" with its quotes
And blocks, object and array literals, spreads, conditionals, logical expressions and await
  contribute no segments of their own
```

A binding site is a variable identifier, the local name of a defaulted binding element or parameter, a marked object member or class field key, the written target of `=`, `&&=`, `||=` or `??=`, a default export (`default`), or an export assignment (`module.exports`). A destructuring declarator as a whole contributes nothing. Role sites are arguments of calls and constructions, tagged-template substitutions, JSX attributes (`<Tag attr>`) and JSX expression children (`<Tag children>`). JSX preserves dotted tag names and spells a fragment `Fragment`.

### Scenario: Skip explicitly declared modules and name rows in other string modules

```gherkin
Given a .ts file with 'declare module "x" { function f() {} }'
When ordinary analysis receives the recoverable Oxc tree
Then it skips the declared module and returns zero rows plus one warning containing
  "An implementation cannot be declared in ambient contexts."
And replacing that body with a signature 'function f(): void;' yields zero rows and zero warnings
And strict analysis of the original returns a parse error with its side and path
```

Implementations inside an explicitly declared module or namespace contribute zero rows, including when the parser recovers a body. For a string module without the `declare` modifier, a recovered implementation can supply a row. In `module "x" { function f() {} }` the name is `"x"::f`. In `module "p/q" { function f() {} }` it is `"p%2Fq"::f`. String module segments always retain their quotes and apply N11 and N12 to their decoded content. The declaration-file suffix filter applies before parsing.

A `global { ... }` block supplies a `global` segment, including inside another namespace. `global { function f(){} }` yields `global::f`. An explicitly declared `declare global` block follows the ambient skip rule above.

### Scenario: Choose and absorb the first applicable segment (N4)

```gherkin
Given a declaration or expression occupying one or more naming sites
When its own segment is chosen
Then the first applicable rule below supplies that segment
And an absorbed site appears exactly once in the full name
And parentheses, JSX expression containers, "as", "satisfies", angle-bracket type
  assertions and non-null assertions preserve a direct value
```

| Priority | Construct | Own segment |
|---|---|---|
| 1 | Function or class declaration, method or accessor | Declared name or marked key. Anonymous default declaration uses `default`. |
| 2 | Function, arrow or class expression directly at a binding | The binding, even if the expression has its own name. Absorb the site. |
| 3 | Other named function or class expression | Its declared name. Keep an enclosing role. |
| 4 | Anonymous expression directly at a role | The role. Absorb the site. |
| 5 | Function used as the callee of a call or construction | `<iife>` |
| 6 | Function returned directly or used as a concise arrow body | `<return>` |
| 7 | Remaining class or function | `<class>` or `<anonymous>`, respectively |

Both constructor kinds use `constructor`. Static blocks use `<static-block>`. A static initializer row absorbs its field site. A direct function or class value uses the field binding. An instance initializer supplies its field segment to any nested functions regardless of which constructor owns its execution.

### Scenario: Preserve member identity when the spelling changes (N5-N6)

```gherkin
Given a class or object member
When its key is named
Then markers appear in the order "static ", then "get " or "set ", then the key including any private "#"
And async, generator, override, readonly, accessor, abstract and accessibility modifiers add no text
And identifier and private identifier escapes are decoded
```

For a string key, use the decoded value bare if it is an ECMAScript IdentifierName or exactly equals `ToString(ToNumber(value))`. Otherwise enclose it in double quotes. Numeric and BigInt literals use their ECMAScript decimal property-key string. Computed string, no-substitution template, Number and BigInt literals use the corresponding literal key. Other computed keys enclose N9 text in parentheses.

### Scenario: Reduce property chains and label call roles (N7-N8)

```gherkin
Given an assignment target, call, construction or tagged template
When its site contributes a segment
Then a written chain joins identifiers, this, super and accesses with dots,
  folds optional accesses to ordinary accesses, and drops parentheses and assertion wrappers
And a literal subscript adds "." and its N6 key, while any other adds "." and its parenthesized N9 text
And for a root other than an identifier, this or super the chain starts at the first access after the root
And a call site uses callee(label), with the written callee or tag, no type arguments,
  "<call>" for an empty chain and a "new " prefix for construction
```

Build one call-role segment for each enclosing argument site, through transparent spread expressions as well as direct arguments. A later argument's site takes its label from argument zero when that argument is an eligible literal. In `Math.max(0, ...xs.map((x) => x))`, the outer site's label is `0` and the arrow absorbs the inner `xs.map()` role, yielding `Math.max(0)::xs.map()`. String literals and no-substitution templates give a quoted label. A numeric literal gives its canonical number. All other cases use empty parentheses. Extra arguments, callback indices and hook dependency lists contribute no text.

### Scenario: Preserve the analyzer's lexer tokens in computed text (N9)

```gherkin
Given a non-literal computed key or subscript
When the analyzer builds its expression text
Then it keeps the source spelling of the analyzer's lexer tokens, dropping whitespace and comment trivia
And it inserts one ASCII space only between adjacent word-like tokens (identifiers, keywords or numeric literals)
And it replaces each embedded function expression, arrow or class expression with one "<function>" or "<class>" token
```

### Scenario: Apply ordinals after the full names are known (N10)

```gherkin
Given two top-level functions dup, each containing a function inner, in one script
When names are assigned in row start-offset order
Then the names are dup, dup::inner, dup#2, dup::inner#2 in that order
And the same script in a second file starts its numbering again
And appending a third dup assigns dup#3 without renaming earlier rows
```

A terminal `#` followed by decimal digits belongs only to an ordinal. Removing that suffix recovers the stem. Repeated hook roles and equal shortened names follow the same rule.

### Scenario: Encode segment content before joining segments (N11)

```gherkin
Given a segment containing any of the following characters
  percent, colon, slash, backslash, star, question mark, either square or curly
  bracket, vertical bar, backtick, C0 or C1 control, DEL, whitespace other than
  U+0020, a Unicode 16.0.0 General_Category=Cf character, or a double quote within quoted content
When the name is formed
Then each such character becomes its UTF-8 bytes, each written as % and two uppercase hex digits
And a lone UTF-16 surrogate becomes %u followed by four uppercase hex digits
And ordinary spaces and other Unicode characters retain their spelling without Unicode normalization
```

### Scenario Outline: Shorten delimited content in code points (N12)

```gherkin
Given a <site> whose decoded content is <content>
When the analyzer measures and encodes that content
Then the delimited part of the name is <result>

Examples:
  | site       | content               | result                      |
  | call label | a×47 + /              | "a×47%2F"                   |
  | call label | a×48 + /              | "a×40"~65942819             |
  | call label | a×40 + 😀×8           | "a×40😀×8"                  |
  | call label | a×40 + 😀×9           | "a×40"~1e76f166             |
  | call label | a×39 + 😀×10          | "a×39😀"~a3312e2f            |
  | quoted key | a×47 + lone D800      | "a×47%uD800"                |
  | quoted key | a×48 + lone D800      | "a×40"~fc8d9416             |
  | quoted key | a×48 + hyphen         | "a×40"~3552814c             |
  | N9 text    | k×48 followed by +1   | (k×40)~15107db7             |
```

In this table `c×n` means exactly `n` repetitions of the indicated character. `lone D800` is a single code unit produced by a `\uD800` string escape. Measure the unencoded content in code points, counting a lone surrogate once. Through 48 keep all the encoded content. Above 48 encode the first 40, close the delimiter and append `~` plus the first eight lowercase hex digits of SHA-256 of the **full N11-encoded content** as UTF-8. A call's final `)` follows this suffix. Identifiers and full written chains have no length cap.

### Scenario Outline: Pin the 78 compatibility examples

```gherkin
Given a separate .tsx file containing <source>
When the analyzer names every row
Then its complete row multiset equals <names>, including the indicated source-order ordinals

Examples:
  | source | names |
  | function load() {} | ["load"] |
  | export function load() {} | ["load"] |
  | export default function load() {} | ["load"] |
  | export default function () {} | ["default"] |
  | export default () => {} | ["default"] |
  | export default class { render() {} } | ["default::render"] |
  | const parse = (s) => {} | ["parse"] |
  | const retry = function again() {} | ["retry"] |
  | let f; f = () => {} | ["f"] |
  | module.exports = function () {} | ["module.exports"] |
  | module.exports.f = function () {} | ["module.exports.f"] |
  | exports.f = () => {} | ["exports.f"] |
  | X.prototype.y = function () {} | ["X.prototype.y"] |
  | module.exports = { run() {} } | ["module.exports::run"] |
  | export = function () {} | ["module.exports"] |
  | obj[key] = () => {} | ["obj.(key)"] |
  | obj["a-b"] = () => {} | ["obj.\\"a-b\\""] |
  | class Cart { add() {} } | ["Cart::add"] |
  | class Cart { constructor() {} static create() {}} | ["Cart::constructor","Cart::static create"] |
  | class Cart { constructor() {} #total() {}} | ["Cart::constructor","Cart::#total"] |
  | class Cart { constructor() {} static #seed() {}} | ["Cart::constructor","Cart::static #seed"] |
  | class Cart { constructor() {} get size() {} set size(v) {}} | ["Cart::constructor","Cart::get size","Cart::set size"] |
  | class Cart { constructor() {} static get empty() {}} | ["Cart::constructor","Cart::static get empty"] |
  | class Cart { constructor() {} get #secret() {}} | ["Cart::constructor","Cart::get #secret"] |
  | class Cart { constructor() {} async *rows() {}} | ["Cart::constructor","Cart::rows"] |
  | class Cart { constructor() { this.handle = () => {} }} | ["Cart::constructor","Cart::constructor::this.handle"] |
  | class Cart { constructor() {} items = xs.map((x) => x)} | ["Cart::constructor","Cart::items::xs.map()"] |
  | class Cart { constructor() {} onAdd = () => {}} | ["Cart::constructor","Cart::onAdd"] |
  | class Cart { constructor() {} static cache = build(() => x)} | ["Cart::constructor","Cart::static cache","Cart::static cache::build()"] |
  | class Cart { constructor() {} static {} static {}} | ["Cart::constructor","Cart::<static-block>","Cart::<static-block>#2"] |
  | class P { x = 1 } | ["P::constructor"] |
  | class Q { onAdd = () => {} } | ["Q::constructor","Q::onAdd"] |
  | const A = class B { go() {} } | ["A::go"] |
  | run(class { go() {} }) | ["run()::go"] |
  | register([class { go() {} }]) | ["register()::<class>::go"] |
  | foo({ bar() {} }) | ["foo()::bar"] |
  | const api = { get() {}} | ["api::get"] |
  | const api = { get value() {}} | ["api::get value"] |
  | const api = { nested: { save: () => 1 } } | ["api::nested::save"] |
  | const api = { "content-type": () => {}} | ["api::\\"content-type\\""] |
  | const api = { "valid"() {}} | ["api::valid"] |
  | const api = { 42() {}, 0x2A() {}, "42"() {}} | ["api::42","api::42#2","api::42#3"] |
  | const api = { [1.50]() {}} | ["api::1.5"] |
  | const api = { [Symbol.iterator]() {}} | ["api::(Symbol.iterator)"] |
  | const api = { "a*b"() {}} | ["api::\\"a%2Ab\\""] |
  | const api = { "a::b"() {}} | ["api::\\"a%3A%3Ab\\""] |
  | const api = { "line\\nbreak"() {}} | ["api::\\"line%0Abreak\\""] |
  | arr.map((x) => x * 2) | ["arr.map()"] |
  | Math.max(0, ...xs.map((x) => x)) | ["Math.max(0)::xs.map()"] |
  | const ids = xs.map((x) => x.id) | ["ids::xs.map()"] |
  | promise.then((v) => v, (e) => e) | ["promise.then()","promise.then()#2"] |
  | fetch(u).then((r) => r.json()) | ["then()"] |
  | useEffect(function sync() {}) | ["useEffect()::sync"] |
  | emit(42, () => {}) | ["emit(42)"] |
  | app.get("/orders", auth, (req, res) => {}) | ["app.get(\\"%2Forders\\")"] |
  | describe("Cart", () => { it("adds", () => {}) }) | ["describe(\\"Cart\\")","describe(\\"Cart\\")::it(\\"adds\\")"] |
  | it("returns the cached value when the key has not expired yet", () => {}) | ["it(\\"returns the cached value when the key ha\\"~e390cc88)"] |
  | it("returns the cached value when the key has expired already", () => {}) | ["it(\\"returns the cached value when the key ha\\"~9f8065a4)"] |
  | new Promise((resolve) => {}) | ["new Promise()"] |
  | const Button = React.memo((props) => {}) | ["Button::React.memo()"] |
  | function outer() { function inner() {} } | ["outer","outer::inner"] |
  | function make() { return () => {} } | ["make","make::<return>"] |
  | const add = (a) => (b) => a + b | ["add","add::<return>"] |
  | function make() { return { run() {} } } | ["make","make::run"] |
  | (function () {})() | ["<iife>"] |
  | const mod = (() => ({ run() {} }))() | ["mod::<iife>","mod::<iife>::run"] |
  | const handlers = [() => a, () => b] | ["handlers::<anonymous>","handlers::<anonymous>#2"] |
  | const pick = ok ? () => 1 : () => 2 | ["pick::<anonymous>","pick::<anonymous>#2"] |
  | function f(cb = () => {}) {} | ["f","f::cb"] |
  | const { onError = () => {} } = opts | ["onError"] |
  | namespace NS { export function f() {} } | ["NS::f"] |
  | namespace A.B { function g() {} } | ["A::B::g"] |
  | function Toolbar() { return <Button onClick={() => save()} />} | ["Toolbar","Toolbar::<Button onClick>"] |
  | function Toolbar() { return <UI.Button onClick={() => go()} />} | ["Toolbar","Toolbar::<UI.Button onClick>"] |
  | function Toolbar() { return <List>{(item) => <li />}</List>} | ["Toolbar","Toolbar::<List children>"] |
  | function Toolbar() { return <>{() => x}</>} | ["Toolbar","Toolbar::<Fragment children>"] |
  | function dup() { function inner() {} } function dup() { function inner() {} } | ["dup","dup::inner","dup#2","dup::inner#2"] |
  | function parse(s: string): void; function parse(s) {} | ["parse"] |
```

### Scenario Outline: Resolve key, role and wrapper edge cases

```gherkin
Given a separate .ts file containing <source>
When its rows are named
Then the names include every literal string in <expected>

Examples:
  | source | expected |
  | const { server: local = function impl() {} } = opts | ["local"] |
  | let [f = () => 1] = xs | ["f"] |
  | const f = ((function inner() {}) as F)! satisfies F | ["f"] |
  | const f = <F>(() => 1) | ["f"] |
  | obj.k &&= () => 1 | ["obj.k"] |
  | obj.k ??= () => 1 | ["obj.k"] |
  | obj.k \|\|= () => 1 | ["obj.k"] |
  | (obj!.next as X)?.on<T>("save", () => 1) | ["obj.next.on(\\"save\\")"] |
  | (a + b)(() => 0) | ["<call>()"] |
  | new (function () {})() | ["<iife>"] |
  | tag`a${() => 1}b` | ["tag()"] |
  | useEffect(() => a, [a]); useEffect(() => b, [b]) | ["useEffect()","useEffect()#2"] |
  | class C { #\\u0061() {} } | ["C::#a"] |
  | const o = { "042"() {} } | ["o::\\"042\\""] |
  | const o = { 0xan() {} } | ["o::10"] |
  | const o = { [1e21]() {} } | ["o::1e+21"] |
  | const o = { [KEYS /* ignored */ . save]() {} } | ["o::(KEYS.save)"] |
  | const o = { [x instanceof Thing]() {} } | ["o::(x instanceof Thing)"] |
  | const o = { [key(() => 1)]() {} } | ["o::(key(<function>))","o::(key(<function>))::key()"] |
  | const o = { [key(class Local {})]() {} } | ["o::(key(<class>))"] |
  | const o = { [x + 0x2A]() {} } | ["o::(x+0x2A)"] |
  | const o = { "a/%2F\\"b"() {} } | ["o::\\"a%2F%252F%22b\\""] |
  | const o = { "a\\u0085\\u00A0\\u200Db"() {} } | ["o::\\"a%C2%85%C2%A0%E2%80%8Db\\""] |
  | const o = { "\\uD800"() {} } | ["o::\\"%uD800\\""] |
  | const o = { "\\u00e9"() {}, "e\\u0301"() {} } | ["o::é","o::é"] |
  | const o = { [1e-7]() {} } | ["o::1e-7"] |
  | const o = { [10n]() {} } | ["o::10"] |
  | const o = { "\\u{E0061}"() {} } | ["o::\\"%F3%A0%81%A1\\""] |
  | const o = { "\|`\\x7f\\\\?[]{}"() {} } | ["o::\\"%7C%60%7F%5C%3F%5B%5D%7B%7D\\""] |
```

JSON arrays in these two tables list literal names. Gherkin cell escapes are decoded before reading the source or the JSON. The last accent example distinguishes U+00E9 from U+0065 followed by U+0301.

A callback in a computed method key inherits the method name before its own call role. The decision count for evaluating the key still belongs to the enclosing execution context.

### Scenario: Encode both parts of namespaced JSX roles

```gherkin
Given a .tsx file containing "const x = <svg:rect xlink:href={() => 1}>{() => 2}</svg:rect>"
When its two arrow functions are named
Then the complete names are x::<svg%3Arect xlink%3Ahref> and x::<svg%3Arect children>
```

### Scenario: Include a method's decorator in its source span

```gherkin
Given a class with "@dec(1)" on line 2 and "work() {}" on line 3
When the method is analyzed
Then its span starts on line 2 and ends on line 3
```

### Scenario: Name an arrow after its direct binding across body edits

```gherkin
Given three successive versions of one row
  A  const f = () => 1
  B  const f = () => 2
  C  const g = () => 2
When all three are analyzed
Then their names are f, f and g respectively
```

### Scenario: Name stability under edits that preserve naming sites

```gherkin
Given any generated valid source with nested rows and a chosen row
When trivia changes, the file moves, or an edit is confined to a nested body
And the edit preserves the chosen row's naming sites and the source order of equal stems
Then the chosen row keeps its name
And renaming only its declared identifier changes its name when that identifier supplies its own segment
```

### Scenario: Generated names can be used literally

```gherkin
Given any valid generated file with repeated stems and arbitrary string-key content,
  including escaped UTF-16 code units
When all rows receive scheme-1 names
Then names are unique within the file and numbering each equal stem in start order
  gives exactly the unsuffixed stem, then #2, #3 and so on
And splitting at "::" recovers whole segments
And using any complete emitted name as a case-sensitive, anchored globset name glob
  matches exactly that row in the file
```

Compile scheme-1 name globs with case sensitivity and backslash escaping on every platform. The later allow matcher uses the same options as the generated-name test.

### Scenario: Keep the package and its parser independently buildable

```gherkin
Given Spec 45 has landed beneath this change, including its lockfile, Rust 1.88 job and parser guards
When CI tests the workspace against the committed lockfile
Then the new crapts package runs its library tests on Rust 1.98 and the existing Rust 1.88 checks pass
And extending the parser guard to crapts rejects a dependency line starting with "syn " or "cargo-crap "
  in the tree of normal and build dependencies on every target with all features enabled
And the guard excludes test-only and proc-macro dependency paths
And the existing cargo-crap/core guards and byte-for-byte Spec 44 goldens pass with snapshot updates disabled
```

### Scenario: Add the analysis library and its pinned dependencies

```gherkin
Given the landed Spec 45 manifest and lockfile
When workspace metadata, the lockfile diff and the Dependabot configuration are inspected
Then crapts is a library package with publishing disabled and inherits the seven exact Oxc pins
And T15 adds sha2 for N12 name suffixes, exactly pinned in workspace dependencies with default features disabled
And T23 adds the existing workspace globset for path exclusions
And the member's direct non-development dependencies are exactly the seven pins plus the helpers introduced by those tasks
And the Cargo dependency-update group covers only the seven Oxc packages
And every package ID (name, version, source) from the base lockfile also exists in the new lockfile
And every added package belongs to the new member or its dependency closure
```

---

## Tasks

Run T1 through T25 in order after the Spec 45 handoff and rebase. Each scenario below belongs to one task. The Spec 55 appendix is reference material.

- [x] **T1 - Add the unpublished crapts library to the workspace.**
  Scenarios: _Add the analysis library and its pinned dependencies_.
  Tests: Acceptance against Cargo metadata, the final Spec 45 lockfile and the Dependabot group. Extend the existing membership assertion in the same change.

- [x] **T2 - Run the library on Rust 1.98 and enforce its parser boundary.** Needs: T1.
  Scenarios: _Keep the package and its parser independently buildable_.
  Tests: Acceptance through the parser-guard script with allowed and forbidden dependency trees, the CI configuration checks, the Rust 1.88 checks and the locked Spec 44 goldens.

- [x] **T3 - Analyze a clean file in its extension's grammar.** Needs: T1.
  Scenarios: _Choose the source grammar from the path_, _Name an arrow after its direct binding across body edits_.
  Tests: Acceptance through `analyze` for each source grammar, an empty file, a function declaration and a directly bound arrow across body and binding edits.

- [x] **T4 - Score decisions in a function's parameters and body.** Needs: T3.
  Scenarios: _Count each decision in its owning parameters or body_.
  Tests: Unit tests for the decision table and acceptance through the emitted CC, including the expressions that add zero.

- [x] **T5 - Return diagnosable errors for invalid bytes and wrong-grammar input.** Needs: T4.
  Scenarios: _Reject syntax that belongs to a different source grammar_, _Reject invalid UTF-8 source bytes_.
  Tests: Acceptance through ordinary and strict decoding and through the strict wrong-grammar fixtures. Assert the path and comparison side where specified.

- [x] **T6 - Distinguish recovered and fatal parses in both modes.** Needs: T5.
  Scenarios: _Return recovered rows with one warning_, _Skip a fatal parse and continue the ordinary scan_, _A changed-file comparison requires a clean parse on both sides_.
  Tests: Acceptance through the file analysis result. Check status, retained rows, first diagnostic and continued ordinary scanning, then strict errors on each comparison side.

- [x] **T7 - Map source bytes to JavaScript line and column positions.** Needs: T6.
  Scenarios: _A UTF-8 BOM counts toward source columns_, _Count JavaScript line terminators_, _Identify arrow functions for coverage attribution_.
  Tests: Unit tests for the line index and acceptance for the BOM, CRLF as one break, lone CR, LF and both Unicode separators. Check arrow identity for concise and block arrows, an ordinary function expression and a declaration.

- [x] **T8 - Compose enclosing segments and choose the row's own segment.** Needs: T7.
  Scenarios: _Compose scheme-1 names from ancestors and sites (N1-N3)_, _Choose and absorb the first applicable segment (N4)_.
  Tests: Unit tests for precedence and transparent wrappers. Acceptance for traversal from the outermost site, absorption exactly once and fallback names.

- [x] **T9 - Collect nested bodies, spans and owned statement lines.** Needs: T8.
  Scenarios: _Separate nested decisions and skip declarations without bodies_, _Distinguish source regions from their outer line range_.
  Tests: Acceptance on the complete nested row sets, parameter decisions, declaration-only skips, concise-body bounds and UTF-16 columns. Assert exact sorted statement lines for each row.

- [x] **T10 - Encode reserved segment content.** Needs: T9.
  Scenarios: _Encode segment content before joining segments (N11)_.
  Tests: Unit tests for reserved characters, Unicode format controls, UTF-8 percent encoding and lone UTF-16 surrogates. Acceptance checks encoded segments in emitted names.

- [x] **T11 - Apply namespace and module rules to retained implementations.** Needs: T10.
  Scenarios: _Skip explicitly declared modules and name rows in other string modules_.
  Tests: Acceptance for ambient signatures and recovered implementations in ordinary and strict modes, plus the names of retained functions in quoted modules.

- [x] **T12 - Build computed name text from lexer tokens.** Needs: T11.
  Scenarios: _Preserve the analyzer's lexer tokens in computed text (N9)_.
  Tests: Unit tests for token adjacency and embedded expression replacement. Acceptance for computed names with comments, significant token spelling and nested callbacks.

- [x] **T13 - Canonicalize class and object member names.** Needs: T12.
  Scenarios: _Preserve member identity when the spelling changes (N5-N6)_.
  Tests: Unit tests for decoded identifiers, literal keys and ECMAScript numeric spellings. Acceptance for marker order, private members, accessors and computed keys.

- [x] **T14 - Name calls, constructions, tagged templates and JSX roles.** Needs: T13.
  Scenarios: _Reduce property chains and label call roles (N7-N8)_, _Encode both parts of namespaced JSX roles_.
  Tests: Acceptance for written and optional chains, labels, transparent spreads, nested call sites and encoded tag and attribute namespaces.

- [x] **T15 - Shorten long labels, quoted keys and computed text.** Needs: T14.
  Scenarios: _Shorten delimited content in code points (N12)_.
  Tests: Unit tests at the 48/49-code-point boundary and for the first 40 code points. Acceptance through the analyzer for every N12 fixture, including lone surrogates, computed keys and the exact suffix from the full encoded content.

- [x] **T16 - Assign class initializers to constructors and static rows.** Needs: T15.
  Scenarios: _Assign each class initializer to its scoring owner_, _Locate an implied constructor across discontiguous initializers_.
  Tests: Acceptance for written and implied constructors. Assert the complete named row sets and CC, ordered regions, display spans and static-block bounds.

- [x] **T17 - Handle auto-accessors and direct class-valued fields.** Needs: T16.
  Scenarios: _Score auto-accessor initializers as fields_, _Direct class-valued fields supply a class name_.
  Tests: Acceptance for instance and static auto-accessors, named and anonymous class expressions and their child methods.

- [x] **T18 - Attribute class definition expressions and locate decorated methods.** Needs: T17.
  Scenarios: _Count class definition expressions in the surrounding execution context_, _Include a method's decorator in its source span_.
  Tests: Acceptance for heritage, decorator and key decisions at file scope and inside a function. Check that the method span starts at its decorator.

- [x] **T19 - Assign source-order ordinals and verify the naming corpus.** Needs: T18.
  Scenarios: _Apply ordinals after the full names are known (N10)_, _Pin the 78 compatibility examples_, _Resolve key, role and wrapper edge cases_.
  Tests: Unit tests for repeated full stems and per-file reset. Acceptance replays all 78 compatibility fixtures and 29 key, role and wrapper fixtures with their individual expected names. Exercise individual cases in the earlier rule tests as each rule lands.

- [x] **T20 - Check name stability and literal matching on generated source.** Needs: T19.
  Scenarios: _Name stability under edits that preserve naming sites_, _Generated names can be used literally_.
  Tests: Property tests through the analyzer and the anchored glob matcher. A file move, trivia edit or nested body edit preserving naming sites and stem order preserves the chosen name. A supplying identifier rename changes it. Equal stems number in source order, segments split intact and each emitted glob matches exactly its own row.

- [x] **T21 - Discover Git paths relative to literal input roots.** Needs: T1.
  Scenarios: _Discover the ordinary Git file set_, _Resolve input roots and preserve path spelling_.
  Tests: Acceptance in disposable Git worktrees for tracked, deleted, ignored and untracked files, embedded repositories, gitlinks, absolute roots, overlapping roots and exact sorted path spelling.

- [x] **T22 - Walk regular files and skip symbolic links.** Needs: T21.
  Scenarios: _Walk regular files outside a Git worktree_, _Skip symbolic links to files and directories_.
  Tests: Acceptance in temporary directories with and without Git, including explicit symlink roots and a scan with Git absent from PATH.

- [x] **T23 - Filter directory and file roots by suffix and exclusion.** Needs: T22.
  Scenarios: _Select the eight source extensions case-sensitively_, _Apply default and caller-supplied exclusions_, _Explicit file roots use the same filters as directory roots_.
  Tests: Unit tests for the suffix and declaration-file rules. Acceptance for default, replacement and appended exclusions on directory roots and named files.

- [x] **T24 - Account for matching extra globs and reuse discovery scope on old paths.** Needs: T23.
  Scenarios: _Match path globs and account for unused exclusions_, _Apply discovery scope to old paths_.
  Tests: Acceptance for overlapping matches, ordered unused patterns with their origins, and present or absent paths across root, ignore and repository boundaries.

- [x] **T25 - Surface discovery errors with the failing path, pattern or Git command.** Needs: T24.
  Scenarios: _Report discovery failures_.
  Tests: Acceptance for a missing root, an outside-worktree root, a malformed glob on an empty scan and a corrupt disposable Git index.

---

## Implementation Notes

- Spec 45 supplies the workspace shell. Add `crates/crapts` with `publish = false` and a library target for this stage. Spec 53 adds the executable and Spec 57 owns publication. Extend the workspace-membership and parser-guard fixtures added by Spec 45 to include the new member. Spec 49 owns the core move. The set of public `cargo_crap` exports is identical before and after Spec 50.
- `discover(roots, options)` returns `Discovery` with sorted `files`, the `scope` predicate and `unused_excludes`. `analyze(path, bytes, mode) -> Result<FileAnalysis, InputError>` accepts `Ordinary` or `Strict(Base | Head)`. `FileAnalysis` carries `status` (`Clean`, `Recovered` or `SkippedFatal`), `rows` and `warnings`. Warnings are returned data for the caller to display. Ordinary input decoding failures and strict parse or decoding failures return `InputError`. Each `Row` carries `file`, `name`, `span`, `body_regions`, `statement_lines`, `cyclomatic` and `pub is_arrow: bool`. Spec 52 uses `is_arrow` for coverage attribution. It is true for block and concise arrow functions and false for every other scored unit, including ordinary function expressions. Regions use the half-open positions specified above.
- Pin `oxc_allocator`, `oxc_ast`, `oxc_ast_visit`, `oxc_parser`, `oxc_span`, `oxc_str` and `oxc_syntax` to `=0.159.0` in the workspace manifest and group their updates in `.github/dependabot.yml`. Use the parser's lexer tokens for the text of computed names. Attribute decisions to the row that executes each expression.
- Resolve the new dependency closure from the landed lockfile. Compare package IDs against that base. Dependency lists in existing records can gain version qualifiers when a second version enters the lockfile.
- Keep the naming fixtures and generated-source checks under `crates/crapts/tests/`. Add the Rust 1.98 job and extend Spec 45's `cargo tree -p <package> --all-features --target all -e normal,build,no-proc-macro --prefix none --locked` guard to crapts. Run the workspace gates with `INSTA_UPDATE=no`.

### Reference behavior and intentional parser differences

At reference revision `5e1ae62f`, 264 individual CLI inputs with empty Istanbul coverage produced 357 rows. The revised contract matches all 78 compatibility names, all 29 naming edge cases and all 354 comparable CC values. The row sets agree on 261 inputs. The two wrong-grammar files and the unclosed function account for the other three. Fourteen location, region or statement-line checks used the same build's analyzer result because the CLI JSON exposes only the starting line.

The frozen TypeScript CLI and the Oxc front end have different syntax-error and decoding policies. These are the exceptions for the input fixtures above.

| Input | TypeScript ordinary scan | Required Rust behavior and reason |
|---|---|---|
| Explicit `declare module` with a function body | Zero rows, no parse warning | Zero rows. Ordinary analysis returns the first Oxc diagnostic. Strict analysis returns the side and path because Oxc diagnoses the ambient implementation. |
| `class C { declare accessor x = f(()=>1) }` | Zero rows, no parse warning | Zero rows. Ordinary analysis reports one warning and `Recovered` because Oxc diagnoses the modifier combination. Strict analysis returns an input error naming the side and path. |
| String module without `declare` containing an implementation | Named row, no parse warning | Ordinary analysis names the retained row and returns the first diagnostic if Oxc reports one. |
| `const x; let [a];` before `kept` | `kept`, no parse warning | Oxc recovers the tree and diagnoses the declarations. Ordinary analysis returns `kept` plus one warning. |
| Unclosed `partial` function | Returns `partial` plus a warning | A fatal Oxc parse has no usable tree. Skip the file with a warning in ordinary mode. |
| JSX in `wrong.ts` or TS annotations in `wrong.js` | Retains `B` with a warning in the first, `f` without a warning in the second | Strict analysis rejects a diagnostic on either comparison side. An ordinary TypeScript scan cannot exercise this strict API. |
| Invalid UTF-8 bytes `66 80 0A` | Decodes with replacement, returns zero rows and a binary-file warning | Reject invalid bytes with an input error so reported positions refer to the original source. |
| `class C { static constructor(){} x = 1 }` | Emits one constructor row containing the field initializer | Oxc parses the static member as an ordinary method. Emit `C::static constructor` and charge the instance field to the implied `C::constructor`. |

Spec 51 report rows omit `fingerprint`. Spec 54 uses names and paths for baseline move detection. It pairs eligible `(file, name)` entries with the stem-count guard and the path-suffix and unique-name passes. Fingerprint computation and the `fingerprint` report field arrive together in Spec 55. Its changed-function gate is the first reader. The author will settle matching and verdict rules in that spec.

### Build order and the golden-test pins

This draft started on Spec 46 T1 (`kqqoyrnt`), alongside Spec 46 T2 (`krtxkooq`). After approval and the Spec 45 `ALL-DONE` handoff, rebase only the Spec 50 working change onto the confirmed final Spec 45 revision before building. The intermediate Spec 45 workspace tip is not a build base.

Spec 44 T1 pins two tests in `tests/golden/config.rs`, `tests/golden/gates.rs`. Preserve both when adding the new job and dependencies.

- `the_goldens_build_against_the_committed_lock` checks that `Cargo.lock` exists, is tracked and is not ignored. The CI `test` job verifies tracking before Cargo runs, then runs one default and one all-features nextest across all targets plus one doctest run, all locked. Keep the crapts-only Rust 1.98 run in its own job.
- `just_test_and_ci_verify_without_writing` pins the verify-only Justfile test environment, the CI `test` job's `INSTA_UPDATE: "no"` and its Rust-change condition, output and required path-filter entries, including `tests/snapshots/**`. Extend the Spec 45 workspace coverage without narrowing those checks.
- The root `Cargo.toml` gains a member and exact Oxc dependencies. T1's manifest-related contract is the committed resolution used by those locked runs. T1 has no assertion on the root manifest text. Spec 45's workspace metadata checks cover the membership change.

### Non-goals

Coverage ingestion, hit attribution and CRAP scoring belong to later integration. Reports, CLI and config wiring, baseline pairing, Git diffs, changed-function verdicts, release packaging and corpus benchmarks are outside this spec.

## Appendix for Spec 55 - Oracle-checked row fingerprints

At frozen TypeScript revision `5e1ae62f`, all 16 exact digest fixtures and all 61 fingerprint comparisons matched the reference. This appendix preserves the token streams, normalization rules and observed comparisons for Spec 55. Spec 50's acceptance tests end above.

The reference also gave `function f() {}` the same fingerprint with and without an initial UTF-8 BOM.

### Hash the row's own token sequence

```gherkin
Given a row with a declared identifier, method, accessor or field key where applicable
When its fingerprint is computed from the analyzer's lexer tokens
Then take its own tokens in source order, omit its name tokens and trivia,
  and substitute one "<row>" token per directly nested row
And apply the token-normalization rules below, then join the remaining texts
  with one U+0000 between neighbors and no leading or trailing separator
And the result is SHA-256 over those UTF-8 bytes, represented by 64 lowercase hex digits
```

For fingerprints, ordinary tokens retain their source spelling. Apply these rules before hashing (N9 computed names still use source spelling).

- Serialize each decoded string literal and each cooked template part as ECMAScript `JSON.stringify` of that string. Each template head, middle or tail supplies one value token. Quote style, equivalent escapes and physical CRLF versus LF in a template then give the same bytes.
- Drop whitespace-only JSX text. For other JSX text use its source spelling with ECMAScript `trim()` applied at both ends. Interior characters, including spaces and physical line breaks, remain part of a significant JSX text token.
- Drop a trailing comma immediately before `)`, `]` or `}`. Retain a comma that represents an omitted array element or binding slot. Ignore zero-width tokens synthesized during recovery.

Select the following tokens for each row, ordered by their original source offsets.

- For a declaration, function expression or arrow, take the function syntax from its first token through its body. Omit a function's declared identifier even if a different binding supplies its row name. An enclosing variable declaration, assignment, export of an expression or call contributes no tokens to that function. A declaration's own modifiers, including `export` and `default`, are tokens of that declaration. References to the identifier inside the body still count.
- For a method, getter, setter or written constructor, take the decorated member through the end of its body. Keep modifiers, `get`, `set` and `*`. Omit the entire method or accessor key, including brackets around a computed key. A written constructor retains the syntax keyword `constructor`. A nested row inside an omitted key still receives its own fingerprint.
- For each instance field assigned to the constructor, take only the initializer expression. Merge those expressions with the written constructor's own tokens in lexical order. An implied constructor consists solely of those expressions. Field keys, `=` signs, field terminators and intervening members contribute no tokens to these regions. A static field row likewise takes only its initializer expression. A static block contributes the block from `{` through `}`.
- Within the selected regions, substitute a single `<row>` at the first selected token of each directly nested row and omit that child's remaining tokens, including its descendants. For disjoint regions preserve intervening tokens belonging to other rows according to their own ownership. A row's display span can cross code that contributes nothing to its fingerprint.

### Pin token boundaries and digests for each scoring unit

```gherkin
Given a separate .ts file containing <source>
When the analyzer fingerprints <row>
Then its hash input is the UTF-8 encoding of <tokens> joined with U+0000
And its lowercase SHA-256 is <digest>

Examples:
  | source | row | tokens | digest |
  | function f() { return 7; } | f | ["function","(",")","{","return","7",";","}"] | 4af25d2ede8038d8005027c94023967275843d12c7fdf71170f0e3577702a8a7 |
  | function g ( ) { /* note */ return 7; } | g | ["function","(",")","{","return","7",";","}"] | 4af25d2ede8038d8005027c94023967275843d12c7fdf71170f0e3577702a8a7 |
  | function f() { function inner() { return 9; } return 7; } | f | ["function","(",")","{","<row>","return","7",";","}"] | 85e3a15560d34bc8f2d67ab05ffab0a799d29f6061eaaa7e401deaedab5e986e |
  | export default function load() {} | load | ["export","default","function","(",")","{","}"] | 6b25a06cc6bd703946589d774a280938545056ac2dbf5e60656e17b1eb355418 |
  | const cb = () => 7 | cb | ["(",")","=>","7"] | eaa20f90a26d223958bb73e2a47203b29b080b4c20e4556c87543cdcbf51a734 |
  | function f() { return "é"; } | f | ["function","(",")","{","return","\\"é\\"",";","}"] | da7f0f26db0909610025816c6e0c9d26a836e26ad27ef6807da37ad91dc20734 |
  | const retry = function again(x) { return x } | retry | ["function","(","x",")","{","return","x","}"] | 742d240258b67753723e8df7f2ae7009c7e1ba81fff6bd071a5c60fd84ed7ecd |
  | class C { static async *items() { yield 1 } } | C::static items | ["static","async","*","(",")","{","yield","1","}"] | fc23a5911512be25c9cc6f832e9a6923bf5f34f2549e0a250f0e1ec0b2a848eb |
  | class C { static get size() { return 7 } } | C::static get size | ["static","get","(",")","{","return","7","}"] | 8ef751bc97808c7b941ea94be66301e92e5b8c9018ed41c78c4ce6fd8083801e |
  | class C { set size(v) { use(v) } } | C::set size | ["set","(","v",")","{","use","(","v",")","}"] | 462133c6b3d4c746f31b19e549c00c53f342ea8613d46f606cecfccfd3aeeaf4 |
  | class C { @dec(1) work() {} } | C::work | ["@","dec","(","1",")","(",")","{","}"] | a17a257ead6de846a42ff0115766a46595485ec46ac188f035686dc31280189b |
  | class C { [k + 1]() { return 2 } } | C::(k+1) | ["(",")","{","return","2","}"] | 0c2f3e2fa84c1857a0dad7f6b4a86d2a8a765feb01a24ab463ee367b7af2be7a |
  | class C { a = 1; m() {} constructor() { run() } b = 2 } | C::constructor | ["1","constructor","(",")","{","run","(",")","}","2"] | f04e918c960b28f0209157959a29ac317aa62056a686bf83ec911fc111fc262d |
  | class C { a = 1; static s = 9; b = x => x } | C::constructor | ["1","<row>"] | 67485f951fc56fd27a957889b4cd5d3390d296c72c4cae7ede3da34f51011661 |
  | class C { static p = init(2); } | C::static p | ["init","(","2",")"] | 69497ea6d4668bfa73c1eca4d48560f96af37ee4eba766f91ca7a7942642086d |
  | class C { static { go() } } | C::<static-block> | ["{","go","(",")","}"] | b39029fd370b2f72b2fd2c81345a677ff69330900b08e79351c4478ceea95407 |
```

The `"é"` literal contains U+00E9. In the arrow fixture, `const cb =` supplies a name but is outside the hashed expression. A direct nested arrow in an instance field contributes one `<row>` token to the implied constructor.

### Normalize formatting while retaining literal content

```gherkin
Given a .tsx file containing "const f = () =>" followed by <first>
When its expression changes to <second>
Then the fingerprints of f are <comparison>

Examples:
  | first                | second              | comparison |
  | <p>  hi </p>         | <p> hi </p>         | equal      |
  | <p>  {x}  </p>       | <p>{x}</p>          | equal      |
  | <p>a b</p>           | <p>a  b</p>         | different  |
  | 'it\\u0065m'         | "item"              | equal      |
  | `a\\x20${x}\\u0062` | `a ${x}b`          | equal      |
  | "value"              | `value`             | equal      |
  | g(x,)                | g(x)                | equal      |
  | [x,]                 | [x]                 | equal      |
  | ({k:x,})             | ({k:x})             | equal      |
  | [x,,]                | [x,]                | different  |
  | ([x,,] = xs)         | ([x,] = xs)         | different  |
```

### Compare physical line endings inside tokens

```gherkin
Given <source> in a .tsx file, with a physical LF in place of {LF}
When its LF is replaced by CRLF or CR
Then the fingerprint of f is <comparison> against the LF version

Examples:
  | source | comparison |
  | const f = () => tag`a${v} c{LF}${w}` | equal |
  | const f = () => <p>a{LF}  b</p> | different |
```

The latter break is internal to significant JSX text. The reference trims its edges and hashes its interior verbatim. A line-ending conversion in trivia or in the cooked value of a template has no such effect.

### Distinguish edits to owned code from edits to other rows

```gherkin
Given a .ts file containing <original source> and a copy with <edit>
When both versions are analyzed
Then their rows satisfy <fingerprints>

Examples:
  | original source                                   | edit                         | fingerprints                        |
  | function f(x = 1) { return x }                     | default 1 to 2               | f differs                           |
  | function f() { return 0x07 }                       | 0x07 to 7                    | f differs                           |
  | function f() { return "a b" }                      | remove the space in "a b"    | f differs                           |
  | const f = x => /a b/u.test(x)                      | remove the space in /a b/u   | f differs                           |
  | function f() { function g() { return 1 } }         | 1 to 2                       | f equal, g in f differs             |
  | function f() { function g() { return 1 } }         | insert function h() {} after g inside f | f differs, g in f equal    |
  | class C { get value() { return 1 } }               | key value to size            | corresponding getters equal         |
  | class C { static init = make(1) }                  | key init to cache            | corresponding static rows equal     |
  | class C { x = a ? 1 : 2; m() { return 0 } }        | 2 to 3                       | constructor differs, m equal        |
  | class C { x = a ? 1 : 2; m() { return 0 } }        | 0 to 1                       | constructor equal, m differs        |
  | class C { constructor() {} y = xs.map(x => x+1) }  | 1 to 2                       | constructor equal, callback differs |
```

### Include a method's decorator in its source span and fingerprint

```gherkin
Given a class with "@dec(1)" on line 2 and "work() {}" on line 3
When the method is analyzed
Then its span starts on line 2 and ends on line 3
And changing only the decorator argument from 1 to 2 changes the method fingerprint
And renaming only the method from work to run preserves the fingerprint
```

### Body edits preserve a binding name and binding edits preserve a body fingerprint

```gherkin
Given three successive versions of one row
  A  const f = () => 1
  B  const f = () => 2
  C  const g = () => 2
When all three are analyzed
Then their names are f, f and g respectively
And A and B have different fingerprints, while B and C have equal fingerprints
```

### Name and fingerprint stability under irrelevant edits

```gherkin
Given any generated valid source with nested rows and a chosen row
When a transformation preserves its naming sites, normalized own token texts and their boundaries,
  and the source order of equal stems
Then trivia-only changes including LF to CRLF, a move of the file, or an edit confined to
  a nested body preserve the chosen row's name and fingerprint
And renaming only its declared identifier preserves its fingerprint and changes its name
  when that identifier supplies its own segment
```
