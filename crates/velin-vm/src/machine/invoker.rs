use super::{ExecutionPolicy, LengthGuard, Machine};
use velin_bytecode::{ExprOp, InitialFrame, Op, Program, ValidatedProgram};

/// Reusable owner of a validated machine and its initial frame.
///
/// Calling [`restart`](Self::restart) restores the same logical initial state
/// while reusing the machine's frame and register allocations. The contained
/// machine is intentionally mutable and must be owned per concurrent caller.
#[derive(Debug, Clone)]
pub struct MachineInvoker {
    machine: Machine,
    initial: InitialFrame,
    seed: i64,
}

impl MachineInvoker {
    /// Creates a low-latency invoker with execution profiling disabled.
    #[must_use]
    pub fn new(program: &ValidatedProgram, seed: i64, initial: &InitialFrame) -> Option<Self> {
        Self::new_with_policy(program, seed, initial, ExecutionPolicy::default())
    }

    /// Creates a low-latency invoker with an explicit execution policy.
    #[must_use]
    pub fn new_with_policy(
        program: &ValidatedProgram,
        seed: i64,
        initial: &InitialFrame,
        policy: ExecutionPolicy,
    ) -> Option<Self> {
        let machine = Machine::from_validated_with_seed_and_frame_without_profile_and_policy(
            program, seed, initial, policy,
        )?;
        Some(Self {
            machine,
            initial: initial.clone(),
            seed,
        })
    }

    /// Creates an invoker that retains the normal execution profile.
    #[must_use]
    pub fn with_profile(
        program: &ValidatedProgram,
        seed: i64,
        initial: &InitialFrame,
    ) -> Option<Self> {
        Self::with_profile_and_policy(program, seed, initial, ExecutionPolicy::default())
    }

    /// Creates a profiled invoker with an explicit execution policy.
    #[must_use]
    pub fn with_profile_and_policy(
        program: &ValidatedProgram,
        seed: i64,
        initial: &InitialFrame,
        policy: ExecutionPolicy,
    ) -> Option<Self> {
        let machine =
            Machine::from_validated_with_seed_and_frame_and_policy(program, seed, initial, policy)?;
        Some(Self {
            machine,
            initial: initial.clone(),
            seed,
        })
    }

    /// Restarts the contained machine from its prevalidated initial frame.
    ///
    /// # Errors
    /// This retained result type is compatible with [`Machine::restart`]; the
    /// invoker's private initial frame was validated when it was constructed.
    pub fn restart(&mut self) -> Result<(), &'static str> {
        self.machine
            .restart_from_known_initial(&self.initial, self.seed);
        Ok(())
    }

    /// Returns the reusable machine for binding and execution.
    #[must_use]
    pub const fn machine(&self) -> &Machine {
        &self.machine
    }

    /// Returns mutable access to the reusable machine.
    pub fn machine_mut(&mut self) -> &mut Machine {
        &mut self.machine
    }

    /// Clears profile counters before the next measurement run.
    pub fn reset_profile(&mut self) {
        self.machine.reset_profile();
    }
}

pub(super) fn length_guards(program: &Program) -> Box<[Option<LengthGuard>]> {
    program
        .ops
        .iter()
        .map(|op| {
            let Op::JumpIfFalse { condition, .. } = op else {
                return None;
            };
            let chunk = program.chunks.get(*condition as usize)?;
            let ops = program
                .expr_ops
                .get(chunk.ops.start as usize..chunk.ops.end as usize)?;
            let [
                ExprOp::Load {
                    dst: index_dst,
                    slot: index_slot,
                    ..
                },
                ExprOp::Load {
                    dst: collection_dst,
                    slot: collection_slot,
                    ..
                },
                ExprOp::Call {
                    dst: length_dst,
                    function: velin_syntax::Builtin::Len,
                    args,
                },
                ExprOp::Binary {
                    dst: result_dst,
                    left,
                    op: comparison,
                    right,
                },
            ] = ops
            else {
                return None;
            };
            if args.start != *collection_dst
                || args.end != collection_dst.saturating_add(1)
                || *left != *index_dst
                || *right != *length_dst
                || *result_dst != chunk.result
                || !matches!(
                    comparison,
                    velin_syntax::BinaryOp::Less
                        | velin_syntax::BinaryOp::LessEqual
                        | velin_syntax::BinaryOp::Greater
                        | velin_syntax::BinaryOp::GreaterEqual
                )
            {
                return None;
            }
            Some(LengthGuard {
                index_slot: *index_slot,
                collection_slot: *collection_slot,
                comparison: *comparison,
            })
        })
        .collect()
}
