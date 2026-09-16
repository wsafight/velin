use super::{LoopJumps, LowerError, Lowerer};
use crate::ast::{Condition, Stmt};
use velin_bytecode::{Op, Pc};
use velin_check::{TypeCheckKind, TypeCheckSite};
use velin_syntax::{BinaryOp, Builtin, Expr, Value};

impl Lowerer {
    pub(super) fn lower_while(
        &mut self,
        condition: Condition,
        body: Vec<Stmt>,
        depth: usize,
    ) -> Result<(), LowerError> {
        let start = self.builder.here();
        let guard = self
            .builder
            .jump_if_false_op(&condition.expr, condition.line, Pc::MAX);
        let exit = self.builder.push(guard);
        self.type_sites.push(TypeCheckSite {
            pc: exit as usize,
            expression: condition.expr,
            kind: TypeCheckKind::Condition,
        });
        self.loops.push(LoopJumps::default());
        self.block(body, depth + 1)?;
        let jumps = self.loops.pop().expect("loop context was pushed");
        for jump in jumps.continues {
            self.builder.patch(jump, Op::Jump(start));
        }
        self.builder.push(Op::Jump(start));
        let after = self.builder.here();
        self.builder.patch_condition_target(exit, after);
        for jump in jumps.breaks {
            self.builder.patch(jump, Op::Jump(after));
        }
        Ok(())
    }

    pub(super) fn lower_for(
        &mut self,
        name: &str,
        collection: Expr,
        body: Vec<Stmt>,
        line: usize,
        depth: usize,
    ) -> Result<(), LowerError> {
        let id = self.generated_slots;
        self.generated_slots += 1;
        let collection_name = format!("$for{id}_collection");
        let index_name = format!("$for{id}_index");
        self.lower_set(&collection_name, collection, line);
        self.lower_set(&index_name, Expr::Value(Value::Integer(0)), line);

        let variable = |name: &str| Expr::Variable(name.to_owned());
        let start = self.builder.here();
        let condition = Expr::Binary {
            left: Box::new(variable(&index_name)),
            op: BinaryOp::Less,
            right: Box::new(Expr::Invoke {
                function: Builtin::Len,
                arguments: vec![variable(&collection_name)],
            }),
        };
        let guard = self.builder.jump_if_false_op(&condition, line, Pc::MAX);
        let exit = self.builder.push(guard);
        self.type_sites.push(TypeCheckSite {
            pc: exit as usize,
            expression: condition,
            kind: TypeCheckKind::Condition,
        });
        let item = Expr::Invoke {
            function: Builtin::Get,
            arguments: vec![variable(&collection_name), variable(&index_name)],
        };
        self.lower_set(name, item, line);

        self.loops.push(LoopJumps::default());
        self.block(body, depth + 1)?;
        let jumps = self.loops.pop().expect("loop context was pushed");
        let advance = self.builder.here();
        for jump in jumps.continues {
            self.builder.patch(jump, Op::Jump(advance));
        }
        let next = Expr::Binary {
            left: Box::new(variable(&index_name)),
            op: BinaryOp::Add,
            right: Box::new(Expr::Value(Value::Integer(1))),
        };
        self.lower_set(&index_name, next, line);
        self.builder.push(Op::Jump(start));
        let after = self.builder.here();
        self.builder.patch_condition_target(exit, after);
        for jump in jumps.breaks {
            self.builder.patch(jump, Op::Jump(after));
        }
        Ok(())
    }

    pub(super) fn lower_loop_jump(
        &mut self,
        line: usize,
        is_break: bool,
    ) -> Result<(), LowerError> {
        let Some(context) = self.loops.last_mut() else {
            let keyword = if is_break { "break" } else { "continue" };
            return Err(LowerError::new(
                line,
                format!("`{keyword}` is only allowed inside a loop"),
            ));
        };
        let pc = self.builder.push(Op::Jump(Pc::MAX));
        if is_break {
            context.breaks.push(pc);
        } else {
            context.continues.push(pc);
        }
        Ok(())
    }
}
