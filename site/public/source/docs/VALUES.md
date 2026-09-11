# Values and collections

[简体中文](VALUES.zh-CN.md)

Velin has five value types and no floating-point type. Collections are persistent: updates return a new value. Every value that enters runtime state is checked against a data budget.

## The five value types

| Type | Runtime form | Display |
| --- | --- | --- |
| Integer | `i64` | Decimal, no thousands separators |
| Boolean | `true` / `false` | `true` / `false` |
| String | UTF-8 text | The text itself |
| List | Ordered items | Bracketed, items separated by commas |
| Record | String keys to values | Parenthesized `key: value` pairs in a stable order |

There is no `null`, no float, no byte buffer, and no user-defined type. Missing data is either a runtime error (`get` without a fallback) or an explicit fallback value.

Equality (`==`, `!=`) is deterministic for every type. Records compare by key, not by insertion sequence. Display and serialization order for records is stable.

## Integers

Arithmetic is checked. Overflow and division by zero fail the current `run` / `resume`. Unary `-` negates an integer. There is no bitwise operator and no remainder operator.

Keep physical units in the host, or convert them to integers (cents, millimeters, milliseconds) before the script sees them.

## Strings

`"HP: [hp]"` interpolates an expression. Every value has a display form, so `[list(1, 2)]` is valid. Write `[[` for a literal opening bracket. Interpolation may nest, and nested interpolations share a parsing budget.

`+` concatenates two strings. `<` `<=` `>` `>=` order two strings. `len(s)` is the character count. `contains(s, part)` tests for a substring.

## Lists

```velin
default items = list("torch")
set items = push(items, "key")
first = get(items, 0)
maybe = get(items, 3, "nothing")
```

- Indexes are zero-based integers.
- `push` appends and returns a new list.
- `get(list, i)` errors if `i` is missing; `get(list, i, fallback)` does not.
- `put(list, i, value)` requires an existing index.
- `remove(list, i)` requires an existing index.
- `len` and `contains` use item count and membership.
- A `list(...)` call accepts at most 128 arguments.

Forgetting to assign the result of `push` leaves the original list unchanged.

## Records

```velin
default hero = record("name", "Ada", "hp", 30)
set hero = put(hero, "hp", get(hero, "hp") - 3)
```

- Keys must be unique strings at construction.
- `get(record, "hp")` errors if the key is missing unless a fallback is passed.
- `put` updates or inserts a field and returns a new record.
- `remove(record, "atk")` errors if the key is missing.
- `contains(record, "hp")` tests for a key.
- `len` is the field count.

## Data budget

Each value tree is limited to 4,096 nodes, 16 collection levels, and 1 MiB of text. Hosts cannot bypass the budget by injecting a large string through `resume` or `set_variable`; the value is checked on the way in. See [Resource limits](LIMITS.md).
