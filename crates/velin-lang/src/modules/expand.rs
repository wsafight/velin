use super::{Function, MAX_EXPANDED_CALLS, Module, diagnostic, statement_line};
use crate::ast::Branch;
use crate::{Condition, Stmt};
use std::collections::{BTreeMap, BTreeSet};
use velin_syntax::{Diagnostic, Expr, StrPart};

type FunctionKey = (String, String);

pub(super) fn validate_functions(modules: &BTreeMap<String, Module>) -> Result<(), Diagnostic> {
    let mut visiting = Vec::new();
    let mut visited = BTreeSet::new();
    for (module, item) in modules {
        for function in item.functions.keys() {
            visit_function(
                modules,
                &(module.clone(), function.clone()),
                &mut visiting,
                &mut visited,
            )?;
        }
    }
    Ok(())
}

fn visit_function(
    modules: &BTreeMap<String, Module>,
    key: &FunctionKey,
    visiting: &mut Vec<FunctionKey>,
    visited: &mut BTreeSet<FunctionKey>,
) -> Result<(), Diagnostic> {
    if visited.contains(key) {
        return Ok(());
    }
    if let Some(start) = visiting.iter().position(|item| item == key) {
        let mut cycle: Vec<_> = visiting[start..]
            .iter()
            .map(|(module, function)| format!("{module}.{function}"))
            .collect();
        cycle.push(format!("{}.{}", key.0, key.1));
        let function = function(modules, key)?;
        return Err(diagnostic(
            &key.0,
            function.line,
            format!("recursive function call graph: {}", cycle.join(" -> ")),
        ));
    }
    visiting.push(key.clone());
    let definition = function(modules, key)?;
    let mut calls = Vec::new();
    collect_calls(&definition.body, &mut calls);
    for (module, name, line) in calls {
        let target = resolve_key(modules, &key.0, module.as_deref(), &name, line)?;
        visit_function(modules, &target, visiting, visited)?;
    }
    visiting.pop();
    visited.insert(key.clone());
    Ok(())
}

fn collect_calls(statements: &[Stmt], calls: &mut Vec<(Option<String>, String, usize)>) {
    for statement in statements {
        match statement {
            Stmt::Call {
                module,
                function,
                line,
                ..
            } => calls.push((module.clone(), function.clone(), *line)),
            Stmt::Label { body, .. }
            | Stmt::While { body, .. }
            | Stmt::For { body, .. }
            | Stmt::Function { body, .. } => collect_calls(body, calls),
            Stmt::If {
                branches,
                otherwise,
            } => {
                for branch in branches {
                    collect_calls(&branch.body, calls);
                }
                if let Some(body) = otherwise {
                    collect_calls(body, calls);
                }
            }
            Stmt::Import { .. }
            | Stmt::Return { .. }
            | Stmt::Default { .. }
            | Stmt::Set { .. }
            | Stmt::Perform { .. }
            | Stmt::Break { .. }
            | Stmt::Continue { .. }
            | Stmt::Jump { .. } => {}
        }
    }
}

fn resolve_key(
    modules: &BTreeMap<String, Module>,
    current: &str,
    qualifier: Option<&str>,
    name: &str,
    line: usize,
) -> Result<FunctionKey, Diagnostic> {
    let target_module = match qualifier {
        None => current,
        Some(alias) => modules
            .get(current)
            .and_then(|module| module.imports.get(alias))
            .map(String::as_str)
            .ok_or_else(|| {
                diagnostic(current, line, format!("module `{alias}` is not imported"))
            })?,
    };
    let target = modules
        .get(target_module)
        .and_then(|module| module.functions.get(name))
        .ok_or_else(|| {
            diagnostic(
                current,
                line,
                format!("unknown function `{target_module}.{name}`"),
            )
        })?;
    if qualifier.is_some() && !target.exported {
        return Err(diagnostic(
            current,
            line,
            format!("function `{target_module}.{name}` is not exported"),
        ));
    }
    Ok((target_module.to_owned(), name.to_owned()))
}

fn function<'a>(
    modules: &'a BTreeMap<String, Module>,
    key: &FunctionKey,
) -> Result<&'a Function, Diagnostic> {
    modules
        .get(&key.0)
        .and_then(|module| module.functions.get(&key.1))
        .ok_or_else(|| diagnostic(&key.0, 1, format!("unknown function `{}`", key.1)))
}

pub(super) struct Expander<'a> {
    pub(super) modules: &'a BTreeMap<String, Module>,
    pub(super) calls: usize,
    pub(super) next_scope: usize,
    pub(super) active: Vec<FunctionKey>,
}

impl Expander<'_> {
    pub(super) fn expand_block(
        &mut self,
        module: &str,
        statements: &[Stmt],
        prefix: Option<&str>,
    ) -> Result<Vec<Stmt>, Diagnostic> {
        let mut output = Vec::new();
        for statement in statements {
            self.expand_statement(module, statement, prefix, &mut output)?;
        }
        Ok(output)
    }

    fn expand_statement(
        &mut self,
        module: &str,
        statement: &Stmt,
        prefix: Option<&str>,
        output: &mut Vec<Stmt>,
    ) -> Result<(), Diagnostic> {
        match statement {
            Stmt::Call { .. } => self.expand_call_statement(module, statement, prefix, output),
            Stmt::If { .. } => self.expand_if_statement(module, statement, prefix, output),
            Stmt::While { condition, body } => {
                output.push(Stmt::While {
                    condition: Condition {
                        expr: rename_expr(&condition.expr, prefix),
                        line: condition.line,
                    },
                    body: self.expand_block(module, body, prefix)?,
                });
                Ok(())
            }
            Stmt::For {
                name,
                collection,
                body,
                line,
            } => {
                output.push(Stmt::For {
                    name: rename_name(name, prefix),
                    collection: rename_expr(collection, prefix),
                    body: self.expand_block(module, body, prefix)?,
                    line: *line,
                });
                Ok(())
            }
            Stmt::Label { name, body, line } => {
                output.push(Stmt::Label {
                    name: name.clone(),
                    body: self.expand_block(module, body, prefix)?,
                    line: *line,
                });
                Ok(())
            }
            Stmt::Set { .. }
            | Stmt::Default { .. }
            | Stmt::Perform { .. }
            | Stmt::Break { .. }
            | Stmt::Continue { .. }
            | Stmt::Jump { .. } => {
                output.push(rename_leaf(statement, prefix));
                Ok(())
            }
            Stmt::Import { .. } | Stmt::Function { .. } => Ok(()),
            Stmt::Return { .. } => Err(diagnostic(
                module,
                statement_line(statement),
                "`return` is only allowed as the final statement of a function",
            )),
        }
    }

    fn expand_call_statement(
        &mut self,
        module: &str,
        statement: &Stmt,
        prefix: Option<&str>,
        output: &mut Vec<Stmt>,
    ) -> Result<(), Diagnostic> {
        let Stmt::Call {
            module: qualifier,
            function: name,
            arguments,
            bind,
            line,
        } = statement
        else {
            unreachable!("caller matched a call statement")
        };
        let arguments = arguments
            .iter()
            .map(|argument| rename_expr(argument, prefix))
            .collect();
        self.expand_call(
            module,
            qualifier.as_deref(),
            name,
            arguments,
            rename_name(bind, prefix),
            *line,
            output,
        )
    }

    fn expand_if_statement(
        &mut self,
        module: &str,
        statement: &Stmt,
        prefix: Option<&str>,
        output: &mut Vec<Stmt>,
    ) -> Result<(), Diagnostic> {
        let Stmt::If {
            branches,
            otherwise,
        } = statement
        else {
            unreachable!("caller matched an if statement")
        };
        let mut expanded = Vec::new();
        for branch in branches {
            expanded.push(Branch {
                condition: Condition {
                    expr: rename_expr(&branch.condition.expr, prefix),
                    line: branch.condition.line,
                },
                body: self.expand_block(module, &branch.body, prefix)?,
            });
        }
        let otherwise = otherwise
            .as_ref()
            .map(|body| self.expand_block(module, body, prefix))
            .transpose()?;
        output.push(Stmt::If {
            branches: expanded,
            otherwise,
        });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_call(
        &mut self,
        current: &str,
        qualifier: Option<&str>,
        name: &str,
        arguments: Vec<Expr>,
        bind: String,
        line: usize,
        output: &mut Vec<Stmt>,
    ) -> Result<(), Diagnostic> {
        self.calls += 1;
        if self.calls > MAX_EXPANDED_CALLS {
            return Err(diagnostic(
                current,
                line,
                "expanded call count exceeds 4096",
            ));
        }
        let key = resolve_key(self.modules, current, qualifier, name, line)?;
        if self.active.contains(&key) {
            return Err(diagnostic(
                current,
                line,
                "recursive function expansion is not allowed",
            ));
        }
        let definition = function(self.modules, &key)?.clone();
        if arguments.len() != definition.parameters.len() {
            return Err(diagnostic(
                current,
                line,
                format!(
                    "function `{}.{}` expects {} argument(s), found {}",
                    key.0,
                    key.1,
                    definition.parameters.len(),
                    arguments.len()
                ),
            ));
        }
        let scope = format!("$call{}_", self.next_scope);
        self.next_scope += 1;
        for (parameter, value) in definition.parameters.iter().zip(arguments) {
            output.push(Stmt::Set {
                name: format!("{scope}{parameter}"),
                value,
                line,
            });
        }
        self.active.push(key.clone());
        let (last, body) = definition
            .body
            .split_last()
            .expect("function validation ran");
        output.extend(self.expand_block(&key.0, body, Some(&scope))?);
        let Stmt::Return { value, line } = last else {
            unreachable!("function validation requires a final return")
        };
        output.push(Stmt::Set {
            name: bind,
            value: rename_expr(value, Some(&scope)),
            line: *line,
        });
        self.active.pop();
        Ok(())
    }
}

fn rename_leaf(statement: &Stmt, prefix: Option<&str>) -> Stmt {
    match statement {
        Stmt::Set { name, value, line } => Stmt::Set {
            name: rename_name(name, prefix),
            value: rename_expr(value, prefix),
            line: *line,
        },
        Stmt::Default { name, value, line } => Stmt::Default {
            name: rename_name(name, prefix),
            value: rename_expr(value, prefix),
            line: *line,
        },
        Stmt::Perform {
            command,
            arguments,
            bind,
            line,
        } => Stmt::Perform {
            command: command.clone(),
            arguments: arguments
                .iter()
                .map(|argument| rename_expr(argument, prefix))
                .collect(),
            bind: bind.as_ref().map(|name| rename_name(name, prefix)),
            line: *line,
        },
        Stmt::Break { line } => Stmt::Break { line: *line },
        Stmt::Continue { line } => Stmt::Continue { line: *line },
        Stmt::Jump { label, line } => Stmt::Jump {
            label: label.clone(),
            line: *line,
        },
        _ => unreachable!("caller matched a leaf statement"),
    }
}

fn rename_name(name: &str, prefix: Option<&str>) -> String {
    prefix.map_or_else(|| name.to_owned(), |prefix| format!("{prefix}{name}"))
}

fn rename_expr(expression: &Expr, prefix: Option<&str>) -> Expr {
    match expression {
        Expr::Spanned { span, expression } => rename_expr(expression, prefix).spanned(span.clone()),
        Expr::Value(value) => Expr::Value(value.clone()),
        Expr::Variable(name) => Expr::Variable(rename_name(name, prefix)),
        Expr::Unary { op, value } => Expr::Unary {
            op: *op,
            value: Box::new(rename_expr(value, prefix)),
        },
        Expr::Binary { left, op, right } => Expr::Binary {
            left: Box::new(rename_expr(left, prefix)),
            op: *op,
            right: Box::new(rename_expr(right, prefix)),
        },
        Expr::Invoke {
            function,
            arguments,
        } => Expr::Invoke {
            function: *function,
            arguments: arguments
                .iter()
                .map(|argument| rename_expr(argument, prefix))
                .collect(),
        },
        Expr::Interpolate { parts } => Expr::Interpolate {
            parts: parts
                .iter()
                .map(|part| match part {
                    StrPart::Literal(text) => StrPart::Literal(text.clone()),
                    StrPart::Hole(expression) => {
                        StrPart::Hole(Box::new(rename_expr(expression, prefix)))
                    }
                })
                .collect(),
        },
    }
}
