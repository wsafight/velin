# Syntax cheat sheet

[简体中文](CHEATSHEET.zh-CN.md)

A one-page lookup for the statement language. Read this when you already know the model and need a form. For semantics, see the [language reference](LANGUAGE.md). For copy-paste scripts, see [Examples](EXAMPLES.md).

## Statements

Indent with exactly four spaces. Tabs are rejected. `#` starts a line comment.

| Form | Notes |
| --- | --- |
| `default name = constant` | Top-level only; compile-time constant; each name once |
| `set name = expression` | Runtime assignment |
| `name = expression` | Same as `set`; the keyword is optional |
| `perform command(args...)` | Unbound host effect; resume with `None` |
| `name = perform command(args...)` | Bound host effect; resume with `Some(value)` |
| `if condition:` / `elif condition:` / `else:` | Condition must be boolean |
| `while condition:` | Re-checked before each iteration |
| `label name:` | Jump target; optional indented body |
| `jump name` | Transfer control to a label |

## Expressions

Highest to lowest precedence: unary `-` / `not`, then `*` `/`, then `+` `-`, then comparisons, then `==` `!=`, then `and`, then `or`.

- Integers are `i64`. Overflow and division by zero are errors.
- `+` concatenates two strings.
- `<` `<=` `>` `>=` compare two integers or two strings.
- `==` / `!=` compare any values deterministically.
- `"text [expression]"` interpolates. `[[` is a literal `[`.

## Values and built-ins

| Type | Construct |
| --- | --- |
| Integer | `0`, `-12` |
| Boolean | `true`, `false` |
| String | `"hello"` |
| List | `list(1, 2, 3)` |
| Record | `record("name", "Ada", "hp", 30)` |

| Call | Result |
| --- | --- |
| `get(c, k)` / `get(c, k, fallback)` | List item or record field |
| `put(c, k, v)` | Updated list or record |
| `push(list, v)` | List with `v` appended |
| `remove(c, k)` | Item or field removed |
| `len(v)` | Characters, items, or fields |
| `contains(c, v)` | Membership, key, or substring |
| `random(lo, hi)` | Seeded inclusive integer |
| `chance(percent)` | Seeded boolean, `0`–`100` |

Lists are zero-based. Collections are persistent: assign the result of `push` / `put` / `remove`.

## Host names

`say`, `ask`, `log`, and any other identifier after `perform` are host conventions, not keywords. The CLI and Playground implement `say` and `ask`.

## Hard limits (short)

- Source: 1 MiB, 10,000 lines, 64 statement-block levels.
- One expression: 64 KiB, 512 tokens, 32 parenthesis levels.
- VM: 10,000 immediate fuel units per execution call by default; configure `ExecutionPolicy` for other limits.

Full tables live in [Resource limits](LIMITS.md).
