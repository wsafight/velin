use super::{BinaryOp, EvalError, LengthGuard, Machine};

impl Machine {
    pub(super) fn step_length_guard(
        &mut self,
        guard: LengthGuard,
        line: usize,
        target: u32,
    ) -> Result<(), EvalError> {
        self.require_assigned(guard.index_slot, line)?;
        self.require_assigned(guard.collection_slot, line)?;
        let collection = self.frame.values[guard.collection_slot as usize]
            .as_ref()
            .expect("collection slot was checked");
        let length = match collection {
            velin_syntax::Value::List(values) => values.len(),
            velin_syntax::Value::Record(values) => values.len(),
            velin_syntax::Value::String(value) => value.chars().count(),
            _ => return Err(EvalError::new(line, "len expects a list, record or string")),
        };
        let length = i64::try_from(length).map_err(|_| EvalError::new(line, "length overflow"))?;
        let index = self.frame.values[guard.index_slot as usize]
            .as_ref()
            .expect("index slot was checked");
        let velin_syntax::Value::Integer(index) = index else {
            return Err(EvalError::new(
                line,
                format!(
                    "comparison cannot combine {} and integer",
                    index.type_name()
                ),
            ));
        };
        let result = match guard.comparison {
            BinaryOp::Less => *index < length,
            BinaryOp::LessEqual => *index <= length,
            BinaryOp::Greater => *index > length,
            BinaryOp::GreaterEqual => *index >= length,
            _ => unreachable!("length guard comparison was validated"),
        };
        if result {
            self.pc += 1;
        } else {
            self.pc = target as usize;
        }
        Ok(())
    }
}
