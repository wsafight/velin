# Resource limits

[简体中文](LIMITS.zh-CN.md)

Untrusted scripts fail inside published budgets. The core does not cap wall-clock time, open sockets, or write files: those remain host concerns. This page lists what Velin itself refuses.

## Source and expressions

| Limit | Value |
| --- | --- |
| Source file | 1 MiB |
| Physical lines | 10,000 |
| Nested statement blocks | 64 |
| One expression | 64 KiB, 512 tokens, 32 parenthesis levels |
| Interpolation nesting | 32 levels |
| Shared interpolation work | 256 KiB and 2,048 tokens |
| Compile-time module graph | 128 modules, 4 MiB aggregate source |
| Hygienically expanded calls | 4,096 |

Tabs are rejected. Indentation is exactly four spaces per level.

## Values

| Limit | Value |
| --- | --- |
| Nodes in one value tree | 4,096 |
| Collection nesting | 16 levels |
| Text in one value tree | 1 MiB |
| Values retained by one machine | 100,000 |
| Text retained by one machine | 16 MiB |
| Values in one host payload | 100,000 |
| Text in one host payload | 16 MiB |
| Arguments to `list(...)` | 128 |

Every value passed into `set_variable` or `resume` is checked against the same per-value budget. The VM also accounts for the complete frame and each yielded host payload, counting structurally shared values by logical size so limits do not depend on allocation details.

## Bytecode and the VM

The default `ExecutionPolicy` also applies runtime budgets across execution calls:

| Limit | Default |
| --- | --- |
| Cumulative fuel per machine lifetime | 10,000,000 |
| Immediate fuel per `run` / `resume` / batch call | 10,000 |
| Host effects per machine lifetime | 1,000 |
| VM call depth | 64 |
| Host queue events / values / text | 1,024 / 1,000,000 / 64 MiB |

| Limit | Value |
| --- | --- |
| Control-flow operations | 100,000 |
| Expression chunks | 100,000 |
| Slots | 65,536 |
| Constant-value nodes | 100,000 |
| Constant and slot text | 16 MiB |
| Operations per expression chunk | 4,096 |
| Registers per expression chunk | 1,024 |
| Arguments per host instruction | 128 |
| Immediate VM steps per `run` / `resume` | 10,000 fuel units |

A loop with no `perform` hits the immediate fuel budget and returns an error instead of occupying the caller.

Immediate fuel resets for each execution call. Cumulative fuel and host-effect counts survive `run` / `resume` and are copied by `Machine::clone`; `restart` clears them. Hosts may configure all of these limits through `ExecutionPolicy`.

## Tooling caps

These are not language semantics. They apply to specific tools:

| Tool | Cap |
| --- | --- |
| CLI / Playground output | 1 MiB |
| CLI / Playground host effects | 1,000 |
| Playground reply JSON | 1 MiB |
| Playground Worker request | 5 seconds |
| LSP JSON-RPC body | 4 MiB |
| LSP headers | 64 KiB total, 8 KiB per line |

## What the host must still limit

Velin will not open a file or send a packet. A host should still allow-list command names, check arguments, and apply its own time, output, network, and storage budgets. See [Host protocol](HOST.md).
