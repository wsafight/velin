//! Shared script generators for pipeline microbenchmarks.

use std::fmt::Write as _;

pub(crate) fn wide_linear_source(variables: usize) -> String {
    let mut source = String::new();
    for index in 0..variables {
        writeln!(source, "default value_{index} = 0").expect("writing to a string cannot fail");
    }
    for index in 0..variables {
        writeln!(source, "set value_{index} = value_{index} + 1")
            .expect("writing to a string cannot fail");
    }
    source
}

pub(crate) fn expression_heavy_source(expressions: usize) -> String {
    let mut source = String::from("default source = 1\n");
    for index in 0..expressions {
        writeln!(source, "set value_{index} = source * 2 + {index}")
            .expect("writing to a string cannot fail");
    }
    source
}

pub(crate) fn builtin_heavy_source(expressions: usize) -> String {
    let mut source = String::from("default source = 1\n");
    for index in 0..expressions {
        writeln!(
            source,
            "set value_{index} = len(list(source, {index}, source + 1))"
        )
        .expect("writing to a string cannot fail");
    }
    source
}

pub(crate) fn short_circuit_heavy_source(expressions: usize) -> String {
    let mut source = String::from("default flag = true\n");
    for index in 0..expressions {
        writeln!(
            source,
            "set value_{index} = flag and flag or flag and flag or flag"
        )
        .expect("writing to a string cannot fail");
    }
    source
}

pub(crate) fn host_call_source(calls: usize) -> String {
    let mut source = String::new();
    for index in 0..calls {
        writeln!(source, "perform emit({index}, {index} + 1)")
            .expect("writing to a string cannot fail");
    }
    source
}
