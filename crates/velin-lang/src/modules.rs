//! Compile-time modules and hygienically expanded pure functions.

use crate::{CompiledScript, Stmt, lower, parse_program};
use std::collections::BTreeMap;
use velin_syntax::{Builtin, Diagnostic, Expr, StrPart};

mod expand;
use expand::{Expander, validate_functions};

const MAX_MODULES: usize = 128;
const MAX_MODULE_BYTES: usize = 4 * 1024 * 1024;
const MAX_EXPANDED_CALLS: usize = 4096;

/// Source returned by a host-controlled [`ModuleResolver`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModule {
    pub name: String,
    pub source: String,
}

impl ResolvedModule {
    #[must_use]
    pub fn new(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
        }
    }
}

/// Resolves one compile-time import. No resolver is retained by the artifact.
pub trait ModuleResolver {
    /// Resolves `specifier` as imported by `importer`.
    ///
    /// # Errors
    /// Returns a host-owned message when the module cannot be loaded.
    fn resolve(&self, importer: &str, specifier: &str) -> Result<ResolvedModule, String>;
}

impl<F> ModuleResolver for F
where
    F: Fn(&str, &str) -> Result<ResolvedModule, String>,
{
    fn resolve(&self, importer: &str, specifier: &str) -> Result<ResolvedModule, String> {
        self(importer, specifier)
    }
}

#[derive(Debug, Clone)]
struct Function {
    parameters: Vec<String>,
    body: Vec<Stmt>,
    exported: bool,
    line: usize,
}

#[derive(Debug, Clone, Default)]
struct Module {
    imports: BTreeMap<String, String>,
    functions: BTreeMap<String, Function>,
    top_level: Vec<Stmt>,
}

struct Loader<'a, R: ?Sized> {
    resolver: &'a R,
    modules: BTreeMap<String, Module>,
    loading: Vec<String>,
    source_bytes: usize,
}

/// Compiles an entry script plus all of its compile-time imports.
///
/// Function calls are expanded hygienically before ordinary lowering. This
/// intentionally keeps the initial call graph acyclic and adds no runtime call
/// stack or snapshot state.
///
/// # Errors
/// Returns a diagnostic for resolution failures, import cycles, invalid module
/// structure, impure functions, recursive calls, or ordinary parsing/lowering.
pub fn compile_modules<R: ModuleResolver + ?Sized>(
    entry_name: &str,
    entry_source: &str,
    resolver: &R,
) -> Result<CompiledScript, Diagnostic> {
    let mut loader = Loader {
        resolver,
        modules: BTreeMap::new(),
        loading: Vec::new(),
        source_bytes: 0,
    };
    loader.load(entry_name.to_owned(), entry_source, true)?;
    validate_functions(&loader.modules)?;
    let entry = loader
        .modules
        .get(entry_name)
        .ok_or_else(|| diagnostic(entry_name, 1, "entry module was not loaded"))?;
    let mut expander = Expander {
        modules: &loader.modules,
        calls: 0,
        next_scope: 0,
        active: Vec::new(),
    };
    let statements = expander.expand_block(entry_name, &entry.top_level, None)?;
    lower::lower(statements).map_err(|error| error.into_diagnostic(entry_name))
}

impl<R: ModuleResolver + ?Sized> Loader<'_, R> {
    fn load(&mut self, name: String, source: &str, is_entry: bool) -> Result<(), Diagnostic> {
        if self.modules.contains_key(&name) {
            return Ok(());
        }
        if let Some(start) = self.loading.iter().position(|item| item == &name) {
            let mut cycle = self.loading[start..].to_vec();
            cycle.push(name.clone());
            return Err(diagnostic(
                &name,
                1,
                format!("cyclic module import: {}", cycle.join(" -> ")),
            ));
        }
        if self.modules.len() + self.loading.len() >= MAX_MODULES {
            return Err(diagnostic(&name, 1, "module graph exceeds 128 modules"));
        }
        self.source_bytes = self
            .source_bytes
            .checked_add(source.len())
            .filter(|bytes| *bytes <= MAX_MODULE_BYTES)
            .ok_or_else(|| diagnostic(&name, 1, "module graph source exceeds 4 MiB"))?;
        self.loading.push(name.clone());
        let statements = parse_program(source).map_err(|error| error.into_diagnostic(&name))?;
        let mut module = split_module(&name, statements, is_entry)?;
        let imports: Vec<_> = module.imports.keys().cloned().collect();
        for specifier in imports {
            let resolved = self
                .resolver
                .resolve(&name, &specifier)
                .map_err(|message| {
                    diagnostic(
                        &name,
                        1,
                        format!("cannot resolve module `{specifier}`: {message}"),
                    )
                })?;
            if resolved.name.is_empty() {
                return Err(diagnostic(
                    &name,
                    1,
                    "resolver returned an empty module name",
                ));
            }
            module.imports.insert(specifier, resolved.name.clone());
            self.load(resolved.name, &resolved.source, false)?;
        }
        self.loading.pop();
        self.modules.insert(name, module);
        Ok(())
    }
}

fn split_module(name: &str, statements: Vec<Stmt>, is_entry: bool) -> Result<Module, Diagnostic> {
    let mut module = Module::default();
    for statement in statements {
        match statement {
            Stmt::Import { name: import, line } => {
                if module
                    .imports
                    .insert(import.clone(), import.clone())
                    .is_some()
                {
                    return Err(diagnostic(
                        name,
                        line,
                        format!("duplicate import `{import}`"),
                    ));
                }
            }
            Stmt::Function {
                name: function,
                parameters,
                body,
                exported,
                line,
            } => {
                validate_function(name, &function, &body, line)?;
                let definition = Function {
                    parameters,
                    body,
                    exported,
                    line,
                };
                if module
                    .functions
                    .insert(function.clone(), definition)
                    .is_some()
                {
                    return Err(diagnostic(
                        name,
                        line,
                        format!("duplicate function `{function}`"),
                    ));
                }
            }
            statement if is_entry => module.top_level.push(statement),
            statement => {
                return Err(diagnostic(
                    name,
                    statement_line(&statement),
                    "imported modules may contain only imports and functions",
                ));
            }
        }
    }
    Ok(module)
}

fn validate_function(
    module: &str,
    name: &str,
    body: &[Stmt],
    line: usize,
) -> Result<(), Diagnostic> {
    let Some((last, prefix)) = body.split_last() else {
        return Err(diagnostic(
            module,
            line,
            format!("function `{name}` has no return"),
        ));
    };
    if !matches!(last, Stmt::Return { .. }) {
        return Err(diagnostic(
            module,
            line,
            format!("function `{name}` must end with `return`"),
        ));
    }
    for statement in prefix {
        validate_pure_statement(module, name, statement)?;
    }
    if let Stmt::Return { value, line } = last
        && expression_uses_random(value)
    {
        return Err(diagnostic(
            module,
            *line,
            format!("function `{name}` cannot use random or chance"),
        ));
    }
    Ok(())
}

fn validate_pure_statement(
    module: &str,
    function: &str,
    statement: &Stmt,
) -> Result<(), Diagnostic> {
    match statement {
        Stmt::Set { value, line, .. } | Stmt::Return { value, line } => {
            if expression_uses_random(value) {
                return Err(diagnostic(
                    module,
                    *line,
                    format!("function `{function}` cannot use random or chance"),
                ));
            }
        }
        Stmt::Call {
            arguments, line, ..
        } => {
            if arguments.iter().any(expression_uses_random) {
                return Err(diagnostic(
                    module,
                    *line,
                    format!("function `{function}` cannot use random or chance"),
                ));
            }
        }
        Stmt::If {
            branches,
            otherwise,
        } => {
            for branch in branches {
                if expression_uses_random(&branch.condition.expr) {
                    return Err(diagnostic(
                        module,
                        branch.condition.line,
                        "random condition in pure function",
                    ));
                }
                for nested in &branch.body {
                    validate_pure_statement(module, function, nested)?;
                }
            }
            for nested in otherwise.iter().flatten() {
                validate_pure_statement(module, function, nested)?;
            }
        }
        Stmt::While { condition, body } => {
            if expression_uses_random(&condition.expr) {
                return Err(diagnostic(
                    module,
                    condition.line,
                    "random condition in pure function",
                ));
            }
            for nested in body {
                validate_pure_statement(module, function, nested)?;
            }
        }
        Stmt::For {
            collection,
            body,
            line,
            ..
        } => {
            if expression_uses_random(collection) {
                return Err(diagnostic(
                    module,
                    *line,
                    "random collection in pure function",
                ));
            }
            for nested in body {
                validate_pure_statement(module, function, nested)?;
            }
        }
        Stmt::Break { .. } | Stmt::Continue { .. } => {}
        Stmt::Perform { line, .. }
        | Stmt::Default { line, .. }
        | Stmt::Label { line, .. }
        | Stmt::Jump { line, .. }
        | Stmt::Import { line, .. }
        | Stmt::Function { line, .. } => {
            return Err(diagnostic(
                module,
                *line,
                format!("statement is not allowed in pure function `{function}`"),
            ));
        }
    }
    Ok(())
}

fn expression_uses_random(expression: &Expr) -> bool {
    match expression {
        Expr::Spanned { expression, .. }
        | Expr::Unary {
            value: expression, ..
        } => expression_uses_random(expression),
        Expr::Binary { left, right, .. } => {
            expression_uses_random(left) || expression_uses_random(right)
        }
        Expr::Invoke {
            function,
            arguments,
        } => {
            matches!(function, Builtin::Random | Builtin::Chance)
                || arguments.iter().any(expression_uses_random)
        }
        Expr::Interpolate { parts } => parts.iter().any(|part| match part {
            StrPart::Literal(_) => false,
            StrPart::Hole(expression) => expression_uses_random(expression),
        }),
        Expr::Value(_) | Expr::Variable(_) => false,
    }
}

fn statement_line(statement: &Stmt) -> usize {
    match statement {
        Stmt::Import { line, .. }
        | Stmt::Function { line, .. }
        | Stmt::Call { line, .. }
        | Stmt::Return { line, .. }
        | Stmt::Label { line, .. }
        | Stmt::Default { line, .. }
        | Stmt::Set { line, .. }
        | Stmt::Perform { line, .. }
        | Stmt::For { line, .. }
        | Stmt::Break { line }
        | Stmt::Continue { line }
        | Stmt::Jump { line, .. } => *line,
        Stmt::If { branches, .. } => branches.first().map_or(1, |branch| branch.condition.line),
        Stmt::While { condition, .. } => condition.line,
    }
}

fn diagnostic(file: &str, line: usize, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(file, line, 1, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct MapResolver(BTreeMap<String, String>);

    impl ModuleResolver for MapResolver {
        fn resolve(&self, _importer: &str, name: &str) -> Result<ResolvedModule, String> {
            self.0
                .get(name)
                .map(|source| ResolvedModule::new(name, source.clone()))
                .ok_or_else(|| format!("missing `{name}`"))
        }
    }

    fn resolver(modules: BTreeMap<&str, &str>) -> MapResolver {
        MapResolver(
            modules
                .into_iter()
                .map(|(name, source)| (name.to_owned(), source.to_owned()))
                .collect(),
        )
    }

    #[test]
    fn expands_exported_pure_function_from_imported_module() {
        let modules = resolver(BTreeMap::from([(
            "math",
            "export fn add(left, right):\n    return left + right\n",
        )]));
        let script = compile_modules(
            "main",
            "import math\nset result = call math.add(2, 3)\n",
            &modules,
        )
        .expect("module should compile");
        assert!(script.program.slots.get("$call0_left").is_some());
        assert!(script.program.slots.get("result").is_some());
    }

    #[test]
    fn rejects_unexported_and_recursive_functions() {
        let private = resolver(BTreeMap::from([(
            "math",
            "fn hidden(value):\n    return value\n",
        )]));
        let error = compile_modules(
            "main",
            "import math\nset result = call math.hidden(1)\n",
            &private,
        )
        .unwrap_err();
        assert!(error.message.contains("not exported"));

        let recursive = resolver(BTreeMap::from([(
            "math",
            "export fn loop(value):\n    next = call loop(value)\n    return next\n",
        )]));
        let error = compile_modules(
            "main",
            "import math\nset result = call math.loop(1)\n",
            &recursive,
        )
        .unwrap_err();
        assert!(error.message.contains("recursive"));
    }

    #[test]
    fn rejects_cycles_and_effects_in_imported_modules() {
        let cycle = |_: &str, name: &str| {
            Ok(ResolvedModule::new(
                name,
                if name == "a" {
                    "import b\n"
                } else {
                    "import a\n"
                },
            ))
        };
        let error = compile_modules("a", "import b\n", &cycle).unwrap_err();
        assert!(error.message.contains("cyclic module import"));

        let impure = resolver(BTreeMap::from([(
            "bad",
            "export fn ask():\n    perform io()\n    return 1\n",
        )]));
        let error = compile_modules("main", "import bad\nset result = call bad.ask()\n", &impure)
            .unwrap_err();
        assert!(error.message.contains("not allowed in pure function"));
    }
}
