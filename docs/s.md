# s: the language's high-level form

The user's own scripting language, given on 20 Oct as the preferred readable form of the
project's language (checklist: "The language"). The 5-bit format stays the stored form. How
s maps onto it, including how the 5-bit format's unused escape space is allocated, is left
to the implementation, done efficiently. Two requirements come with it:

- **Tail call optimisation is essential** (§6: a call in tail position replaces the current
  unit).
- **The notes view gets a right-aligned mode** for editing s as code. In s the word that acts
  stands at the right, so right alignment keeps the acting words in a column.

The spec below is the user's, verbatim. The reference implementation it mentions
(`src/rt.rs`, `src/compile.rs`, `src/natives.rs`, `std/*.s`, `tests/cases`, `web/srt.js`)
lives in the user's own s repository, not in this one.

---

# s

A postfix scripting language. Words accumulate on a stack; the word that acts stands at the right and consumes what stands to its left. Operators on names are suffixes: the last character of a word decides what the word does.

Reference implementation: `src/rt.rs` (semantics, interpreter, runtime), `src/compile.rs` (s → Rust), `src/natives.rs` (the library's words, in Rust), `std/*.s` (the library files that load them). Tests: `tests/cases/*.s` with expected `*.out`, run through both Rust backends by `cargo test --release` and through the browser runtime by `node web/test.js`.

## 1. Usage

```
s [file.s] [args...]                  interpret (default ./main.s)
s --src file.s [args...]              interpret
s run file.s [args...]                interpret
s rs file.s [-o out.rs] [-e file]... [-n native.rs]    emit one self-contained Rust file
s build file.s [-o bin] [-e file]... [-n native.rs]    emit and compile with rustc -O (--keep keeps the .rs)
s version
```

The crate has no dependencies and needs Rust 1.70 or later: `cargo build --release` gives `target/release/s`. The emitted `.rs` depends only on `std` and builds with `rustc -O --edition 2021 out.rs`.

## 2. Units and tokens

A unit is a source string run as one sequence of operations: a file, a script body, a matched table value, a thread message.

Whitespace is space, newline, tab, carriage return. A unit is scanned left to right into tokens.

- **Brace literal** `{...}`. Braces nest; the literal ends at the matching `}`. At depth 1, `~name~` interpolates the value of `name` (§4.3) when the literal is evaluated; `name` is non-empty and contains no whitespace, `{`, `}` or `~`. `~~` yields `~`. A `~` that does not open a well-formed interpolation is literal. Text at depth ≥ 2 is raw, including `~`.
- **Quote literal** `'...'`. Raw text up to the next `'`. No interpolation, no brace meaning.
- **Word**. A maximal run of non-whitespace characters containing no `{` or `'`.

Characters immediately before a `{` or `'` (no whitespace between) are a prefix of that literal: `pre{fix}` is the literal `prefix`. Characters immediately after a closing `}` or `'` begin a new token.

An unclosed brace or quote is a parse error for the unit.

## 3. Values and state

A value is text. The runtime may hold a value as a number; a number's text is its canonical form: shortest round-trip decimal, no exponent, no trailing `.0`, `-0` written `0`, `NaN`, `inf`, `-inf`. Reading text as a number trims whitespace and parses a decimal float; unparsable text reads as `0`. Representation is unobservable: every operation behaves as if all values were text in canonical form.

State of one interpreter:

| name | content |
|---|---|
| stack | sequence of values |
| ow | boolean, initially false |
| mem | name → value |
| numbers | name → number |
| scripts | name → unit |
| lists | name → sequence of values |
| replacements | word text → stack of definitions, each a value, its code and a generation number |
| frames | stack of saved lines |
| files | name → open file handle with eof flag |
| threads | index → worker |

`mem`, `numbers`, `scripts` and `lists` are separate namespaces over the same names.

Reserved names used by commands: `=` arithmetic default destination, `n` repeat counter, `r` / `rc` shell output and exit code, `rn` prng default destination, `$` list pop result, `err` error text, `eof` end-of-file flag, `out` thread result, `threads` last spawned index, `_` stdin default destination, `args` / `argc` command line.

At start, list `args` holds the command-line arguments after the program file and `argc` their count.

## 4. Words

A word is classified in this order:

1. If the whole word is a command name (§5), it is that command.
2. Else, if the word has at least two characters and starts with `\`, it is an escape (§4.4).
3. Else, if the word has at least two characters and its last character is one of `` ? ! ~ $ ` # ; : . @ ``, it is a suffix form over the stem (the word minus its last character).
4. Else it is a literal word.

A command name is therefore never a literal word: `w`, `r`, `n`, `l`, `f`, `t`, `mv` and the symbol commands must be written as `{w}` or `'w'` when meant as data.

### 4.1 Literals

A literal word pushes its text, unless a definition of it is visible (§11), in which case that definition runs. A command word runs the command, unless a definition of the command's name is visible, in which case that definition runs. Brace and quote literals push their text and are never replaced.

### 4.2 Suffix forms

| form | effect |
|---|---|
| `x?` | if ow is true, act as word `x` (classified again); else nothing |
| `x!` | if ow is false, act as word `x`; else nothing |
| `x~` | lookup (§4.3), then dispatch the value (§4.3) |
| `x@` | move the top value into mem[x]; the rest of the stack stays; an empty stack is an error |
| `` x` `` | mem[x] = the stack joined with single spaces (a single value is stored as is); clear stack |
| `x#` | numbers[x] = first stack value as number (0 if empty); clear stack |
| `x;` | numbers[x] += first stack value (1 if empty; unset counts as 0); clear stack |
| `x:` | numbers[x] −= first stack value (1 if empty); clear stack |
| `x$` | scripts[x] = the stack joined with single spaces, with every `\|` replaced by `~`; clear stack |
| `x.` | call x (§6) |
| `x..` | call x, never as a repeat (§6) |

`?` and `!` compose with every other form: `>?`, `x~!`, `loop.?`, `<-?`.

### 4.3 Lookup

The value of `x` is mem[x] if set, else numbers[x] if set, else a fresh random unsigned 64-bit integer if `x` is `?`, else `404`.

Dispatch: a value whose text is one word (non-empty, no whitespace) runs as that word, classified as in §4: a command runs, a suffix form or escape acts, a word with a visible definition runs it. Any other value, and a literal word without a definition, is pushed.

`x~` dispatches; `{~x~}` pushes the value as data. When a value may be a command name or end in a suffix character, push it with braces.

### 4.4 Escapes

`\w`, where w is a command name, is that command as built in: no definition ever replaces it. `\w` for any other w without a suffix is the literal text w, pushed and never replaced. `\` before a suffix form changes nothing. A lone `\` is a literal word.

### 4.5 Angle words

`⟨word⟩` is word taken whole: it is never read as a suffix form or a command, so a word may end in a suffix character. A suffix written after the closing `⟩` applies to the whole word: `⟨x.⟩~`, `⟨+!⟩$`, `⟨a?⟩.`, `⟨w⟩` pushes the text `w`.

### 4.6 Locals

A name beginning with `:` and longer than `:` is local. `:x~`, `` :x` ``, `:x#`, `:x;`, `:x:`, `:x@`, `:x$`, `:x.` and the list commands on `:x` act on the running activation's own mem, numbers, lists and scripts. Every unit run is an activation: the program, a script call, a definition expanding, a file, text run by `~`. A tail call replaces the activation and its locals; any other call starts a new one. Names without `:` are global.

## 5. Commands

Every command reads its arguments from the stack, A0 being the bottom. Unless stated, a command clears the stack when done. Fewer arguments than required is an error. Square brackets mark optional arguments. `dest` defaults are shown after `=`.

### Output, input, shell

| word | args | effect |
|---|---|---|
| `>` | any | write newline, then each value preceded by one space |
| `>_` | any | write each value, no separators, no newline |
| `<_` | [dest=`_`] | read one line of stdin without its line ending into mem[dest]; mem[eof] = 1 at end of input, else 0 |
| `<` | cmd | run `sh -c cmd` (`cmd /C` on Windows); mem[r] = stdout, numbers[rc] = exit code; stderr passes through |

### Files by path

| word | args | effect |
|---|---|---|
| `w` | path text | write file |
| `+w` | path text | append to file, creating it |
| `mv` | from to | move, overwriting |
| `r` | path [dest=path] | mem[dest] = file contents |
| `r@` | path off len dest | mem[dest] = up to len bytes from byte off |

### File handles

Modes: `r w a r+ w+ a+`, `b` ignored. Handles are named.

| word | args | effect |
|---|---|---|
| `f` | path name mode [errcode] | open |
| `+f` | name off key [errcode] | write mem[key] at byte off |
| `@f` | name off len dest | mem[dest] = up to len bytes at off; eof flag = fewer than len read |
| `f_` | name text [errcode] | write text and newline at the current position |
| `_f` | name dest | mem[dest] = next line without its line ending; eof flag = no line was read |
| `?eof` | name | mem[eof] = 1 if the handle's eof flag is set or no such handle, else 0 |
| `f-` | name | close |

With `errcode` given, a failure sets mem[err] to the message, clears the stack and runs `errcode` as a unit; the stack is then left as that unit leaves it. Without it, failure is an error.

### Network

| word | args | effect |
|---|---|---|
| `tcp` | addr name mode [errcode] | mode `c`: connect to addr (`host:port`) as handle name; mode `s`: listen on addr as handle name, numbers[port] = the bound port; mode `a`: wait for a connection on the listener named addr, as handle name, mem[peer] = the remote address |

A connection is a handle: `f_` writes a line, `_f` reads one (eof flag set when the other side has closed), `+f` writes mem[key] and `@f` reads what has arrived, up to len bytes; their offsets are ignored. `f-` closes. Failures follow the errcode rule above. `127.0.0.1:0 srv s tcp` listens on a free port.

### Numbers

| word | args | effect |
|---|---|---|
| `+ - * / ^` | a b [dest=`=`] | numbers[dest] = a op b; `^` is power |
| `%` | a b [dest=`=`] | numbers[dest] = Euclidean remainder, sign of the result ≥ 0 for b > 0 |
| `n` | value name | numbers[name] = value |
| `#prng` | [seed [seed2 [dest=`rn`]]] | with a seed, reseed the generator from (seed, seed2=0); numbers[dest] = next float in [0, 1) |
| `` `prng `` | [seed [seed2 [dest=`rn`]]] | as `#prng`; mem[dest] = 32 letters from `a-zA-Z` |

The generator is xoshiro256** seeded through splitmix64; the same seeds give the same sequence on every platform. Without a seed, the generator continues from its state, which starts from operating-system randomness.

### Conditions

| word | args | ow becomes |
|---|---|---|
| `=` | a b | a and b have the same text |
| `!=` | a b | not the same text |
| `&` | a b | ow and same text |
| `;` | a b | ow and not the same text |
| `==` | a b | a = b as numbers |
| `<<` `>>` `<<=` `>>=` | a b | a <, >, ≤, ≥ b as numbers |
| `!!` | | not ow |

### Lists

Indices count from 0; a negative index counts from the end.

| word | args | effect |
|---|---|---|
| `l` | value list | append |
| `-l` | list | mem[`$`] = removed last value, `404` if empty |
| `[l]` | list i dest | mem[dest] = item i; nothing if out of range |
| `l=` | value list i | item i = value; nothing if out of range |
| `l-` | list | delete list |
| `#l` | list [dest=`=`] | numbers[dest] = length (0 if no list) |
| `/l` | text list [sep] | list = parts of text: split on whitespace runs without sep, into characters when sep is empty, else on sep |
| `l/` | list dest [sep=` `] | mem[dest] = items joined with sep |
| `#s` | text [dest=`=`] | numbers[dest] = number of characters |

### Code

| word | args | effect |
|---|---|---|
| `?)` | table key [default] | table is read as whitespace-separated pairs `k v`; the first k equal to key gives v, else default; the stack is cleared and that text runs as a unit, leaving the stack as the unit leaves it |
| `&&&` | count name | for n = 0 … count−1: clear stack, numbers[n] = n, call name |
| `><` | value word | add a definition of word (§11) |
| `//` | any | clear stack |
| `<-` | | end the current unit; the stack is kept |
| `<--` | [code=0] | flush output and end the process with code |

`//` clears what stands to its left, so a comment is written before it: `this is a note //`. A brace literal makes a comment that may contain command words: `{any > text} //`.

### Frames and evaluation

These do not clear the stack.

| word | args | effect |
|---|---|---|
| `(` | k | remove k; keep the top k values (all if fewer); move the rest, in order, onto the frame stack |
| `)` | | pop the frame stack; the saved values go back underneath the current ones; no frame is an error |
| `~` | code | remove the top value and run its text as a unit (§11) on the rest of the stack |
| `##` | | push the number of values on the stack |

`a b c d 2 ( + =~ )` leaves `a b 7` for `c=3 d=4`: the frame narrows the line to the two values `+` should see.

### Threads

| word | args | effect |
|---|---|---|
| `t` | path... | per path, start a worker; numbers[threads] = its index |
| `<t` | code i | send code to worker i |
| `>t` | i | wait for worker i's next result, clear stack, run it as a unit, leaving the stack as the unit leaves it |
| `-t` | i | stop worker i; its index is not reused |
| `***` | path... | run each file in its own fresh interpreter in parallel; when all finish, run each one's mem[out] in order, as units |

A worker is a fresh interpreter that runs its file, then repeatedly takes a message, runs it as a unit, and if mem[out] is set, sends it back and unsets it. The message `__stop__` stops it. A bad index sets mem[err] to `Invalid thread index`; a closed worker sets `thread closed`. Workers share nothing with the parent except messages.

## 6. Scripts and calls

`body name$` defines a script. `|` in the body becomes `~` at definition, so `x|` inside a body is the deferred lookup `x~`.

`name.` calls:

- With exactly one value on the stack, that value is a repeat count, a whole number ≥ 0: the stack is cleared, and the script runs count times, each run starting from an empty stack; the stack is cleared afterwards.
- Otherwise the script runs once on the current stack. Values left by the caller are visible to it, and values it leaves remain for the caller.

`name..` always runs once on the current stack.

If no script `name` exists, the file at path `name` is run as the unit, then a library file of that name (§12); if none exists, it is an error. A file path given to `name.`, `t` or `***` is looked up from the working directory and then from the directory of the main program file. Paths given to file commands (`r`, `w`, `f` and the rest) are taken from the working directory only.

A call that is the last operation of a unit is a tail call: it replaces the current unit instead of nesting in it, so recursion in tail position runs in constant space. `loop.?` as the last word of `loop` is a loop.

`<-` ends only the innermost running unit.

## 7. Errors

An error writes `s: message` to stderr, flushes stdout and ends the process with code 1. Errors: missing arguments, a bad repeat count, an unparsable unit, a missing script or file, file and shell failures without an error code unit.

## 8. Compilation

`s rs` translates a program into Rust with the same behaviour as the interpreter.

- The runtime (`rt.rs`) and the library's natives (`natives.rs`) are copied verbatim into the output; the runtime contains the interpreter, which runs any unit not known at compile time.
- The machine keeps its own stack of frames: calls, definitions expanding and units run never recurse in Rust, so recursion depth is bounded by memory, not by the thread's stack. A library word that applies a quotation (`map`, `times`, …) runs the machine nested for that quotation.
- Every statically known unit becomes a Rust function `fn cN(rt: &mut Rt, st: u32) -> Flow` (a chunk): the main file, every `-e` file, and every script body whose text is fully known at its `$` site. Script bodies nested in script bodies are found by the same rule. A chunk is a state machine over its ops: it runs straight through and returns to the machine only when an op pushed a frame, resuming at the next state when that frame is done. States are laid out as nested labelled blocks that fall through one into the next, so every transition is a forward jump and only the entry dispatches.
- A chunk whose last op calls the script it is itself restarts in place, without returning to the machine, while nothing else holds its frame.
- Names are interned to integers at compile time; the runtime interns the same table first, so names in chunks are array indices.
- A `$` site carries the chunk compiled for the body it expects. At run time the body text is compared with the chunk's source; on a match the script is the chunk, otherwise it is looked up among all chunks by text, otherwise parsed and interpreted. The same lookup applies to any unit run from text (`?)`, `>t`, files). Compiled and interpreted units call each other freely.
- Every brace or quote literal whose text parses is also compiled as a chunk, so quotations handed to `~`, `><` and the library run compiled.
- Tail calls compile to `return Flow::Tail(name)`; a trampoline runs them.
- A run of up to three literal or lookup words followed by an arithmetic, comparison, or number-setting operation compiles to a guarded fast path computing directly on numbers. The guard holds when the stack is empty, none of the words or the command has a visible definition, and no looked-up value would dispatch to anything but itself; otherwise the plain operation sequence runs. A name with no definitions is remembered per generation, so the guard costs one comparison until the next `><`.
- `-e path` compiles a file into the program; `t`, `***` and `name.` use the compiled file for that exact path instead of reading it.
- Every `name.` whose name is a file on disk or a library file is compiled in as if given with `-e`, transitively, so a program that loads `std/text.s.` carries the library inside the binary.
- The program runs on a thread with a 64 MiB stack; workers get 16 MiB.

## 9. Changes from s.v

- Exact command names are matched before suffixes; `!!` works, lone `;` is the command, lone `:` is a literal.
- `x;` increments and `x:` decrements, as the `Chunk` enum states; both default to 1 and clear the stack.
- A script call no longer pushes an empty value afterwards.
- `?` and `!` re-classify their stem, so suffixes compose.
- Braces nest; interpolation names exclude whitespace, so bodies like `{x~ y~ >}` keep their lookups for run time without `|`.
- Quotes are raw; `{` inside quotes is text.
- Units are parsed independently; an open brace in one unit no longer captures text of the caller.
- `?)` handles a last pair without trailing whitespace, runs the default only on a miss, clears the stack before running the code and leaves what that code leaves.
- `&&&` sets `n` before each run.
- Repeat count 0 runs zero times; a non-numeric single argument to `name.` is an error instead of a panic.
- `-t` keeps indices stable and no longer blocks.
- Numbers print canonically (`3`, not `3.0`).
- Tab and carriage return are whitespace.
- Added: `>_ <_ % & == << >> <<= >>= l= #l /l l/ #s _f <- <-- ( ) ~ ##`, `x..`, `x@`, escapes, tail calls, `args`/`argc`, numbers[rc], an optional destination for `r`.
- `x~` runs a one-word value as a word, as `~` did through ingest in s.v.
- `><` definitions run as code (§11).

## 10. Web editor

`web/srt.js` is the browser runtime, a port of the s 1.0 interpreter; `node web/test.js` runs it against `tests/cases`, leaving out the cases for the native library modules, angle words and locals. `node web/build.js` writes `web/s-editor.html` with the runtime, the library and the examples inlined. The browser carries the library of s 1.0 written in s (`web/std/`): `core`, `line`, `text`, `gem` and `sh` with the words they had then. In the browser, files live in the editor tabs, `<` runs a built-in shell (echo, cat, ls, rm, cp, mv, pwd, true, false, exit) over them, stdin comes from the stdin box, and workers run in step with the program, so `>t` with no pending result is an error instead of a wait.

## 11. Definitions

`value word ><` adds a definition of word. Each definition gets the next generation number from a counter that starts at 1. A word may have many definitions; the newest visible one is used.

Code runs under a bound. A definition is visible when its generation is below the bound.

- The program, files run by `name.`, and text run by `~`, `?)`, `>t` and error codes run with no bound: every definition made so far is visible.
- A definition's body runs with its own generation as the bound: it sees only definitions older than itself. A word used inside its own body has its earlier meaning, or none; expansion always terminates.
- A script runs with the bound it was given when defined: the counter at its `$`. It sees the definitions that existed then.

Code written down sees what was defined before it was written, as text processed by a preprocessor does; code made from data at run time sees everything. A body that wants the built-in command regardless writes the escape `\w`.

When word already has a definition, naming it as the key runs it; quote the key: `{…} 'word' ><`. A command name is always quoted: `{…} '+' ><`.

The value's text is parsed when `><` runs; text that does not parse is pushed as data when the word is used. A definition never changes words ending in a suffix character, which are suffix forms (§4.2).

## 12. Library

A library file is loaded with `std/name.s.`, on an empty stack. Each file loads what it needs, runs once per interpreter, keeps the caller's stack in a frame while it loads, and registers its module's words with `std:name.`. The words are natives (§13) in `src/natives.rs`, defined at the generation where the file loads them, so code written before the load does not see them and a later definition can wrap or replace any of them. The files are built into the `s` binary; a file at the same path on disk takes precedence, and a disk file that leaves out `std:name.` defines only what it says. In compiled programs the files and the natives are compiled in (§8).

Conventions:

- Words act on the line: an operation takes its arguments from the top of the stack and leaves the rest. `k ( … )` narrows the line.
- Quotations are brace literals; a quotation handed to `map`, `filter`, `any`, `all`, `find` sees one value at a time, apart from the line; `fold`, `times`, `while`, `until`, `dip` and `keep` run theirs on the line itself.
- Conditions leave their answer in ow, as the built-in commands do.
- `std:name.` registers module name once per interpreter; `t` and `***` start fresh interpreters, which load what their files load.

### std/core.s

stack words, line arithmetic, maths, bits, conditions, quotations, loops.

| word | effect |
|---|---|
| `drop` `dup` `swap` `over` `rot` `nip` `tuck` | a → ; a → a a ; a b → b a ; a b → a b a ; a b c → b c a ; a b → b ; a b → b a b |
| `2dup` `2drop` | a b → a b a b ; a b → |
| `pick` | … n → … x, a copy of the value n below the top (0 copies the top) |
| `clear` `depth` | … → ; … → … n |
| `add` `sub` `mul` `div` `mod` `pow` | a b → a+b, a−b, a×b, a÷b, a mod b (sign of b), a^b |
| `idiv` | a b → floor(a÷b) |
| `neg` `inc` `dec` `abs` | a → −a, a+1, a−1, \|a\| (ow = a<0) |
| `min` `max` | a b → the smaller, the larger (ow = a<b, a>b) |
| `clamp` | x lo hi → x held within lo … hi |
| `floor` `ceil` `round` `trunc` `sign` | a → rounded down, up, to nearest (halves away from 0), toward 0; −1, 0 or 1 |
| `sqrt` `hypot` `exp` `ln` `log` `log2` | a → √a ; a b → √(a²+b²) ; e^a ; natural, base-10, base-2 logarithm |
| `sin` `cos` `tan` `asin` `acos` `atan` `atan2` | radians; a b → atan2(a, b) |
| `pi` `tau` | → π, 2π |
| `rand` `randint` | → a number in [0, 1) ; lo hi → a whole number in lo … hi; seeded with `#prng` |
| `band` `bor` `bxor` `bnot` `shl` `shr` | bits of whole numbers: a b → a&b, a\|b, a^b ; a → !a ; a n → a<<n, a>>n |
| `lt` `gt` `le` `ge` `eq` `ne` | a b → : ow = a<b, a>b, a≤b, a≥b, a=b, a≠b as numbers |
| `same` `differ` | a b → : ow = same text, different text (numbers compare as numbers) |
| `between` | x lo hi → : ow = lo ≤ x ≤ hi |
| `even` `odd` `isnum` | a → : ow = a is even, odd, a number |
| `not` `yes` `no` | : ow = not ow, true, false |
| `call` | … q → … : run q on the line |
| `if` `when` `unless` | … t f → run t when ow, else f ; … q → run q when ow ; … q → run q unless ow |
| `test` | … q → … : run q on a copy of the top value, apart from the line; only its ow remains |
| `and` `or` | p q → : ow = p and q, p or q; q runs only when needed |
| `dip` `keep` `bi` | … x q → … q x ; x q → q(x) x ; x p q → p(x) q(x) |
| `times` | … q n → … : run q n times on the line |
| `while` `until` | … c b → while c tests true run b ; … b c → run b, again until c tests true |

### std/line.s

the line as an array. Loads `std/core.s`.

| word | effect |
|---|---|
| `gather` `spill` | … name → : the line becomes list name ; name → … : list name onto the line, list kept |
| `show` `count` | … → … : print the line, keep it ; … → n |
| `map` `filter` | x… q → q(x)… ; x… q → the x where q leaves ow true |
| `fold` `foldr` `scan` | x… q → q joins values left to right, right to left ; running results |
| `rev` `sort` `sortdesc` `dedup` | reversed ; ascending, descending by number, stable ; first occurrences only |
| `sum` `product` `mean` `most` `least` | x… → total, product, average (0 when empty), largest, smallest |
| `iota` `til` `range` | n → 1 … n ; n → 0 … n−1 ; a b → a … b by steps of 1 up or down |
| `take` `skip` | x… n → first n ; all but the first n |
| `first` `last` `at` | x… → the first, the last ; x… i → value i from 0, negative from the end, 404 outside |
| `index` | x… v → position of the first value equal to v, or −1 |
| `find` | x… q → the first x where q leaves ow true, or 404 (ow = found) |
| `any` `all` | x… q → x… : ow = q true for some x, for every x |

### std/text.s

text: build, split, join, case, search, numbers as text, format, records. Loads `std/line.s`.

| word | effect |
|---|---|
| `cat` `concat` `glue` `joinwith` `unlines` | a b → ab ; x… → one text, no separators, single spaces, sep given on top, newlines |
| `quote` | x → `{x}`, code that pushes x |
| `words` `chars` `lines` `split` | text → word…, char…, line… (a final newline ends the last line) ; text sep → part… |
| `len` `reversed` `repeat` `squish` | characters ; txet ; text n → texttext… ; single-spaced |
| `trim` `ltrim` `rtrim` | whitespace off both ends, the start, the end |
| `has` `starts` `ends` | text part → : ow = contains, begins with, ends with part |
| `subst` `subst1` `occurs` `pos` | text old new → every old as new, the first ; text part → how many times, character position or −1 |
| `slice` | text from n → n characters from position from (negative counts from the end) |
| `padl` `padr` | text width → padded with spaces on the left, on the right |
| `upper` `lower` `cap` `title` | a–z case ; the first letter up ; every word's first letter up |
| `codeof` `charof` | text → code of its first character ; code → character |
| `fixed` `hex` `bin` `unhex` `unbin` | x n → x with n decimals ; n → hexadecimal, binary text ; text → number (`0x`, `0b` allowed) |
| `format` `printf` `say` | x… text → text with each % filled in order ; the same, printed ; x… → print the line and a newline |
| `get` `set` `keys` `del` `haskey` | records: name key → value or 404 ; name key value → store ; name → keys in the order first set ; name key → unset ; name key → : ow = set |

A record field `name/key` is an ordinary variable; `name/keys` lists the keys.

### std/gem.s

gematria over letters a–z. Loads `std/text.s`.

| word | effect |
|---|---|
| `ord` `red` `rord` | text → ordinal sum (a=1 … z=26), reduced (a=1 … i=9, j=1 …), reverse ordinal (a=26 … z=1) |
| `gem` | text → text ord red rord |
| `ords` `letters` | text → ord of each word ; the ordinal of each letter |
| `droot` | n → digital root |

Case is ignored; other characters count 0.

### std/sh.s

pipelines over lines of text: the line holds one value per text line, `|>` is a separator you can read along. Because a script body turns every `|` into `~`, `~>` is the same separator. Loads `std/text.s`; its `cat` is the file one.

| word | effect |
|---|---|
| `cat` `sh` | path → its lines ; command → its output lines (sets `r` and `rc`) |
| `grep` `grepv` `sed` | line… text → lines that contain text, that do not ; line… old new → each with old as new |
| `head` `tail` `wc` | line… n → first n, last n ; line… → how many |
| `uniq` `nl` `shout` | without repeats next to each other ; numbered `1  line` ; in capitals |
| `put` `>file` `>>file` | line… → print one per line ; line… path → write, append them |
| `ls` `exists` `mkdir` | dir → names, sorted ; path → : ow = it exists ; path → : make it and its parents |
| `env` `now` `sleep` | name → value or 404 ; → milliseconds since 1970 ; ms → : wait |

### std/logic.s

gates on bits: each takes its inputs from the line and leaves its outputs. Loads `std/core.s`; its `not`, `and`, `or` are the gates.

| word | effect |
|---|---|
| `bit` `buf` | a → 1 when a≠0 ; a → a |
| `not` `and` `or` `xor` `nand` `nor` `xnor` | gates on 0 and 1 |
| `hadd` `fadd` | a b → sum carry ; a b c → sum carry |
| `mux` `majority` | a b s → a when s=0, else b ; a b c → 1 when two or three are 1 |
| `bits` `unbits` `popcount` | n w → w bits, highest first ; bit… → n ; n → how many 1 bits |

### std/elec.s

circuits in SI units: volts, amperes, ohms, farads, henries, hertz, seconds. Loads `std/line.s`.

| word | effect |
|---|---|
| `ohm` `amps` `watts` | i r → v ; v r → i ; v i → p |
| `series` `parallel` | r… → total resistance |
| `divider` | vin r1 r2 → vout across r2 |
| `timeconst` `charge` `discharge` | r c → τ ; v r c t → capacitor voltage charging toward v, discharging from v |
| `ledr` | vsupply vforward i → series resistor |
| `xc` `xl` `resonance` | f c → capacitive reactance ; f l → inductive reactance ; l c → resonant frequency |
| `db` `dbp` | ratio → decibels of a voltage ratio, of a power ratio |
| `bands` | color… → resistance: digit bands then a multiplier band (gold ÷10, silver ÷100) |
| `e12` | r → the nearest E12 standard value |

## 13. Natives

A native is a primitive written in Rust. A Rust file given with `-n` is placed in the generated program as module `ext` (the runtime is in scope) and must define

```rust
pub static NATIVES: &[(&str, NativeFn)] = &[("name", function), …];   // NativeFn = fn(&mut Rt)
```

Each entry becomes a definition of name with generation 0, visible to all code, so a native is a word like a library word and later definitions can wrap or replace it. The library modules of §12 are natives of the same kind, registered when their file loads. A native that applies a quotation runs it to the end with `rt.run_quote(text)`; one that hands control on pushes a frame with `rt.run_src(text)`. A native works on the line through `rt.args`, or with `rt.pop(who)`, `rt.pop_num(who)`, `rt.pop_text(who)`, `rt.push_num(x)`, `rt.push_text(s)`, `rt.push(v)`, and may read and set variables with `rt.get_val(sym)`, `rt.mset`, `rt.nset` and `rt.it.sym(name)`.

`examples/native/ext.rs` defines `sqrt now env hex primes`. `s build examples/native/runner.s -n examples/native/ext.rs -o s-ext` makes an interpreter with those natives: `s-ext file.s` runs file.s.

## 14. Limits

- A value looked up with `x~` that is one word runs. Data that might be a command name or end in a suffix character is read with `{~x~}`.
- Words ending in `` ? ! ~ $ ` # ; : . @ `` are suffix forms; no definition renames them, and such a word is written `⟨+!⟩` (§4.5).
- Text sorting is numeric; `sort` orders numbers.
- Every definition of a name is kept, so redefining a name in a loop grows memory.
- In the browser, `<` is a small built-in shell, workers run in step with the program, `tcp` always fails and natives are absent.
- Natives given with `-n` exist only in compiled programs; the `s` interpreter carries the library's natives.
