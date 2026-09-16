# Examples and recipes

[简体中文](EXAMPLES.zh-CN.md)

These scripts stay inside Velin's statement language. I/O still belongs to the host: the CLI prints `say` and reads `ask` from stdin; the Playground consumes a JSON array of replies. Other embedders can bind the same command names, or ignore these names and define their own.

Run the two files that ship in the repository:

```sh
cargo run -p velin-cli -- check examples/counting.velin
cargo run -p velin-cli -- run examples/counting.velin
cargo run -p velin-cli -- check examples/adventure.velin
echo 1 | cargo run -p velin-cli -- run examples/adventure.velin
```

Paste any example below into the [Playground](PLAYGROUND.md) and set `ask` replies to a JSON array such as `[1]`. Each recipe states which replies it expects.

> Four spaces per indent. Collections are persistent: assign the result of `push` / `put`. `say` and `ask` are host names; your embedder can use others.

## Run the included samples

`examples/counting.velin` never waits for the host. It folds `1..5` into a running total and finishes:

```velin
default total = 0
default n = 1

while n <= 5:
    set total = total + n
    perform say("added", n, "running total is", total)
    n = n + 1

perform say("final total:", total)
```

`examples/adventure.velin` is the canonical branching sample. `default` sets compile-time constants, `perform ask` suspends for a host integer, and `jump` selects an ending:

```velin
default hp = 30
default potions = 2

label start:
    perform say("You wake in a cold cell.")
    choice = perform ask("Drink a potion? (1 = yes, 0 = no)")
    if choice == 1:
        set hp = hp + 10
        set potions = potions - 1
        perform say("You feel better.")
    else:
        perform say("You steel yourself.")

    while potions > 0:
        set potions = potions - 1
        perform say("You stash a potion for later.")

    if hp > 35:
        jump good_end
    perform say("You limp onward into the dark.")

label good_end:
    perform say("You escape, unbroken.")
```

A reply of `1` drinks the potion (`hp` becomes `40` and the good ending runs). A reply of `0` skips the drink.

## Build messages with interpolation

Use `[expression]` inside a string to format values without converting them first. String `+` concatenates strings, while `[[` emits a literal opening bracket.

```velin
default name = "Ada"
default role = "ranger"
default level = 4

title = name + " the " + role
perform say("[title] reached level [level + 1].")
perform say("Quest state: [[ready]")
```

This prints `Ada the ranger reached level 5.` followed by `Quest state: [ready]`. Interpolated expressions can produce any Velin value; `+` only concatenates when both operands are strings.

## Inventory with a list

Lists are persistent. `push` returns a new list; the previous value is unchanged unless you assign the result.

```velin
default items = list("torch")
default gold = 5

choice = perform ask("Buy a key for 3 gold? (1 = yes)")
if choice == 1:
    if gold >= 3:
        set gold = gold - 3
        set items = push(items, "key")
        perform say("You bought a key. Gold left: [gold]")
    else:
        perform say("Not enough gold.")
else:
    perform say("You keep walking.")

perform say("Pack:", items)
```

`len(items)` is the item count. `contains(items, "key")` is a membership test. `get(items, 0)` reads the first element; missing indexes are runtime errors unless you pass a fallback: `get(items, 3, "nothing")`.

## Character sheet with a record

Records take alternating string keys and values. `put` updates one field and returns a new record.

```velin
default hero = record("name", "Ada", "hp", 30, "atk", 4)

name = get(hero, "name")
perform say("[name] stands ready.")
set hero = put(hero, "hp", get(hero, "hp") - 7)
hp = get(hero, "hp")
perform say("HP is now [hp]")

if get(hero, "hp") <= 0:
    perform say("Ada falls.")
else:
    perform say("Ada holds.")
```

`contains(hero, "hp")` tests for a key. `remove(hero, "atk")` returns a record without that field; removing a missing key is an error.

## Read optional settings safely

Pass a third argument to `get` when a missing key or list index is expected. The fallback becomes the result instead of raising a runtime error.

```velin
default settings = record("theme", "dark")

theme = get(settings, "theme", "light")
volume = get(settings, "volume", 50)
set settings = put(settings, "volume", volume + 10)
stored_volume = get(settings, "volume")

perform say("Theme: [theme]")
perform say("Volume: [stored_volume]")
```

The script keeps `"dark"` for `theme`, uses `50` for the absent `volume`, and stores `60` in a new record. Without the fallback, `get(settings, "volume")` would stop with an error before the `put`.

## Seeded random encounter

Random calls consume machine-local RNG state. The same seed and replies replay exactly.

```velin
roll = random(1, 6)

perform say("You rolled [roll].")
if roll >= 5:
    perform say("A rare find!")
elif roll >= 3:
    perform say("The path is quiet.")
else:
    perform say("You stumble.")

if chance(25):
    perform say("It starts to rain.")
```

In Rust, create the machine with `Machine::with_seed(program, seed)` when the host must control the sequence. The CLI and Playground use seed `0`.

## Room loop with labels

A label is a jump target. Use it for rooms, menus, and retry loops. Each `run`/`resume` burst still stops after 10,000 immediate fuel units by default, so a loop should `perform` (yield to the host) rather than spin.

```velin
default visits = 0

label hall:
    set visits = visits + 1
    perform say("Hall. Visit [visits].")
    action = perform ask("1 stay, 2 leave")
    if action == 1:
        jump hall
    perform say("You leave the hall.")
```

Unknown or duplicate labels are compile errors. Prefer `while` when the loop condition is a local boolean; prefer `jump` when several named scenes point at each other.

## Host value coming back in

Bound `perform` stores whatever the host returns. The checker treats that value as `Unknown`, so later uses must be valid for the value the host actually supplies.

```velin
default score = 0

delta = perform read_amount("Adjustment")
if delta > 0:
    set score = score + delta
    perform say("Score is [score]")
else:
    perform say("Ignored non-positive amount.")
```

The CLI reference host does not implement `read_amount`; it prints the command and resumes without a value. A real embedder should return `Some(Value::Integer(...))` for this bound call. See [Host protocol](HOST.md).

## Two-option menu

A menu is a bound `ask` plus `if` / `elif`. Replies `[2]` picks the second option.

```velin
default room = "hall"

choice = perform ask("1 look, 2 leave")
if choice == 1:
    perform say("Dust. A door to the east.")
elif choice == 2:
    set room = "street"
    perform say("You step outside.")
else:
    perform say("That is not a choice.")

perform say("You are in the [room].")
```

Keep option numbers in the prompt text. The language does not know what a menu is.

## Limit input retries

A `while` loop can cap retries while `ask` yields to the host on every attempt. This example accepts the number `7` and stops after three wrong answers.

```velin
default attempts = 0
default accepted = false

while attempts < 3 and not accepted:
    answer = perform ask("Enter the access number")
    set attempts = attempts + 1
    if answer == 7:
        set accepted = true
        perform say("Access granted on attempt [attempts].")
    elif attempts < 3:
        perform say("Try again.")

if not accepted:
    perform say("Access denied.")
```

Run a successful third attempt with:

```sh
printf '2\n4\n7\n' | cargo run -p velin-cli -- run retry.velin
```

Here `retry.velin` contains the snippet above, while stdin is reserved for the three `ask` replies. In the Playground, use `[2, 4, 7]` as the reply array.

## Checking before running

`velin check` reports parse errors, type mismatches it can prove, and reads of variables that are not assigned on every incoming path.

```sh
cargo run -p velin-cli -- check examples/adventure.velin
```

A script can still be dynamic where the host is involved: a host return value and conflicting branch types become `Unknown` instead of false-positive errors. Run only after you accept the diagnostics for your host's contract.
