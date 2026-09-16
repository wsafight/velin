use super::*;
use crate::{HostSignature, Type, compile};
use std::future::{Future, ready};
use std::task::{Context, Poll, Waker};

#[test]
fn synchronous_driver_checks_dispatches_and_validates_replies() {
    let script = compile(
        "host.velin",
        "answer = perform ask(\"Ready?\")\nset result = answer + 1\n",
    )
    .unwrap();
    let mut driver = SyncHostDriver::<String>::new().command(
        HostCommand::new(
            "ask",
            HostSignature::exact(vec![Type::String], Some(Type::Integer)),
        )
        .description("Ask for a numeric answer."),
        |values| {
            assert_eq!(values, &[Value::String("Ready?".into())]);
            Ok(Some(Value::Integer(6)))
        },
    );

    let machine = driver.run("host.velin", &script).unwrap();
    assert_eq!(machine.variable("result"), Some(&Value::Integer(7)));
    assert_eq!(
        driver.schema().command_info("ask").unwrap().documentation(),
        Some("Ask for a numeric answer.")
    );
}

#[test]
fn driver_rejects_static_contract_errors_before_dispatch() {
    let script = compile("host.velin", "perform ask(1)\n").unwrap();
    let mut driver = SyncHostDriver::<String>::new().command(
        HostCommand::new(
            "ask",
            HostSignature::exact(vec![Type::String], Some(Type::Integer)),
        ),
        |_| Ok(Some(Value::Integer(1))),
    );
    assert!(matches!(
        driver.run("host.velin", &script),
        Err(HostDriveError::Static(_))
    ));
}

#[test]
fn asynchronous_driver_uses_the_same_declaration() {
    let script = compile("host.velin", "answer = perform ask()\n").unwrap();
    let mut driver = AsyncHostDriver::<String>::new().command(
        HostCommand::new("ask", HostSignature::exact(Vec::new(), Some(Type::Boolean))),
        |_| ready(Ok(Some(Value::Boolean(true)))),
    );
    let machine = block_on(driver.run("host.velin", &script)).unwrap();
    assert_eq!(machine.variable("answer"), Some(&Value::Boolean(true)));
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}
