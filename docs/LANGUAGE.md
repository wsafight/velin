# Language reference

[简体中文](LANGUAGE.zh-CN.md)

Velin source files use the `.velin` extension. The language is indentation-sensitive, deterministic, and deliberately small: scripts own values and control flow, while the embedding host owns every external effect.

This page is the full reference. For a one-page lookup use the [syntax cheat sheet](CHEATSHEET.md). For copy-paste scripts use [Examples](EXAMPLES.md). Lists and records are covered in depth in [Values](VALUES.md).

## File structure

- Indent blocks with exactly four spaces per level. Tabs are rejected.
- Use `#` for line comments.
- Blank lines are ignored.
- Identifiers name variables, labels, and host commands.
- Source is limited to 1 MiB, 10,000 physical lines, and 64 nested statement blocks.

```velin
# Defaults are compile-time constants.
default hp = 30
default ready = true

if ready:
    perform say("HP: [hp]")
```

## Values

Velin has five value types:

| Type | Examples and construction |
| --- | --- |
| Integer | `0`, `-12`, `9223372036854775807` (`i64`) |
| Boolean | `true`, `false` |
| String | `"hello"`, `"HP: [hp]"` |
| List | `list(1, 2, 3)` |
| Record | `record("name", "Ada", "score", 10)` |

Lists and records are persistent values. `push`, `put`, and `remove` return updated collections without mutating the input value. There is no `null` and no floating-point type.

Forgetting to assign the result of `push` or `put` leaves the previous collection unchanged. Missing keys and indexes are runtime errors unless you pass a fallback to `get`.

## Variables and defaults

`default` declares a top-level initial value. It must be a compile-time constant, cannot reference runtime variables, and each default name may be declared only once.

```velin
default score = 0
default tags = list("new")
```

`set` assigns a runtime expression. The `set` keyword is optional for ordinary assignment.

```velin
set score = score + 10
score = score + 10
```

Reading a variable that may not be assigned on every incoming control-flow path is reported by the static checker.

## Expressions and operators

From highest to lowest precedence:

| Operators | Meaning |
| --- | --- |
| `-value`, `not value` | Integer negation and boolean negation |
| `*`, `/` | Checked integer multiplication and division |
| `+`, `-` | Checked integer arithmetic; `+` also concatenates two strings |
| `<`, `<=`, `>`, `>=` | Ordering for two integers or two strings |
| `==`, `!=` | Deterministic equality for any values |
| `and` | Short-circuit boolean conjunction |
| `or` | Short-circuit boolean disjunction |

Arithmetic reports overflow, and division by zero is an error. Parentheses can group expressions. A single expression is limited to 64 KiB, 512 tokens, and 32 parenthesis levels.

`and` / `or` only accept booleans. Mixing an integer with `and` is a check error when the checker can prove the types.

## String interpolation

Place an expression inside square brackets in a string. Every value has a deterministic display representation.

```velin
default name = "Ada"
default score = 7
perform say("[name] has [score + 1] points")
```

Write `[[` for a literal opening bracket. Interpolation may nest up to 32 levels and shares a cumulative parsing budget.

## Conditional control flow

`if`, `elif`, and `else` each own an indented block. Conditions must evaluate to booleans.

```velin
if score >= 100:
    perform unlock("gold")
elif score >= 50:
    perform unlock("silver")
else:
    perform unlock("bronze")
```

## Loops

`while` reevaluates its boolean condition before each iteration.

```velin
default n = 3
while n > 0:
    perform say(n)
    set n = n - 1
```

The default VM policy allows at most 10,000 immediate fuel units per execution call. A loop with no `perform` cannot occupy the caller indefinitely.

## Labels and jumps

A label marks a bytecode target and may optionally own an indented body. `jump` transfers control to a label. Duplicate or unknown labels are compile errors.

```velin
label retry:
    choice = perform ask("Try again?")
    if choice == true:
        jump retry

label done:
    perform say("Finished")
```

## Host effects

`perform` is the only way a script reaches the outside world. Command names are opaque to Velin; the host decides what they mean.

```velin
perform log("started")
answer = perform ask("Continue?")
```

The first form expects no returned value. The bound form suspends the VM and stores the value supplied by the host when it calls `resume(Some(value))`. `say`, `ask`, `log`, and any other names are host conventions, not language keywords.

A host that does not implement a name can still receive it: the CLI prints the name and arguments and resumes. Product hosts should allow-list commands instead. See [Host protocol](HOST.md).

## Built-in functions

| Function | Result |
| --- | --- |
| `list(values...)` | A list of up to 128 arguments |
| `record(key, value, ...)` | A record; keys must be unique strings |
| `get(collection, key)` | A list item or record value; errors when missing |
| `get(collection, key, fallback)` | The item/value, or `fallback` when missing |
| `put(collection, key, value)` | An updated list or record |
| `push(list, value)` | A list with `value` appended |
| `remove(collection, key)` | A list item or record field removed |
| `len(value)` | Character, list-item, or record-field count |
| `contains(collection, value)` | Membership, record-key, or substring test |
| `random(low, high)` | A seeded integer in the inclusive range |
| `chance(percent)` | A seeded boolean for an integer percentage from 0 to 100 |

List indexes are zero-based. `put` and `remove` require an existing list index; removing a missing record key is an error. Random state belongs to the machine frame, so the same seed and inputs replay exactly.

## Static diagnostics

`velin check` combines parsing, lowering, conservative type inference, and definite-assignment analysis. The checker reports only mismatches it can prove. Values returned by hosts and conflicting branch types become `Unknown`, allowing valid dynamic host behavior to proceed.

```sh
cargo run -p velin-cli -- check examples/adventure.velin
```

Diagnostics use 1-based line and column positions and point to the relevant expression node. The full policy is in [Static checking](CHECKING.md).

## Common mistakes

- **Tabs.** The parser rejects them. Use four spaces.
- **`say` as a keyword.** Write `perform say(...)`. Bare `say(...)` is not a statement.
- **Mutating a list in place.** `push(items, "key")` without `set items = ...` does nothing visible.
- **`if 1:`.** Conditions must be booleans. Write `if choice == 1:`.
- **A tight loop with no `perform`.** The VM stops after the immediate fuel budget (10,000 by default).
- **Reading a variable set in only one branch.** Definite assignment will flag it.

Budgets for source, values, and the VM are listed in [Resource limits](LIMITS.md).
