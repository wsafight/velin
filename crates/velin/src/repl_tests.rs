use super::{ReplError, ReplSession};
use crate::Value;

#[test]
fn fragments_share_values_without_appending_programs() {
    let mut session = ReplSession::new();
    session.execute("set hp = 30\n").unwrap();
    session
        .execute("set hp = hp + 5\nset label = \"ready\"\n")
        .unwrap();

    assert_eq!(session.variable("hp"), Some(&Value::Integer(35)));
    assert_eq!(
        session.variable("label"),
        Some(&Value::String("ready".into()))
    );
    assert_eq!(session.history().len(), 2);
}

#[test]
fn static_or_runtime_failures_do_not_commit_session_state() {
    let mut session = ReplSession::new();
    session.execute("set hp = 1\n").unwrap();

    let error = session.execute("set hp = missing + 1\n").unwrap_err();
    assert!(matches!(error, ReplError::Static(_)));
    assert_eq!(session.variable("hp"), Some(&Value::Integer(1)));
    assert_eq!(session.history().len(), 1);

    let error = session.execute("set hp = 1 / 0\n").unwrap_err();
    assert!(matches!(error, ReplError::Runtime(_)));
    assert_eq!(session.variable("hp"), Some(&Value::Integer(1)));
}

#[test]
fn host_effects_are_driven_by_a_callback_and_commit_on_finish() {
    let mut session = ReplSession::new();
    session.execute("set hp = 3\n").unwrap();
    let mut calls = Vec::new();
    session
        .execute_with_host("perform emit(hp)\nset hp = hp + 1\n", |name, values| {
            calls.push((name.to_owned(), values.to_vec()));
            Ok::<_, &'static str>(None)
        })
        .unwrap();

    assert_eq!(calls, vec![("emit".into(), vec![Value::Integer(3)])]);
    assert_eq!(session.variable("hp"), Some(&Value::Integer(4)));
}

#[test]
fn host_failure_does_not_commit_values_but_cannot_undo_the_callback() {
    let mut session = ReplSession::new();
    session.execute("set hp = 3\n").unwrap();
    let mut called = false;
    let error = session
        .execute_with_host("set hp = 9\nperform emit()\n", |_name, _values| {
            called = true;
            Err::<Option<Value>, _>("host unavailable")
        })
        .unwrap_err();

    assert!(called);
    assert!(matches!(error, ReplError::Host(message) if message == "host unavailable"));
    assert_eq!(session.variable("hp"), Some(&Value::Integer(3)));
}

#[test]
fn bound_host_reply_and_rng_state_survive_following_fragments() {
    let mut first = ReplSession::with_seed(91);
    let mut second = ReplSession::with_seed(91);
    first
        .execute_with_host(
            "answer = perform ask()\nset roll = random(1, 100)\n",
            |_name, _| Ok::<_, &'static str>(Some(Value::Integer(7))),
        )
        .unwrap();
    second
        .execute_with_host(
            "answer = perform ask()\nset roll = random(1, 100)\n",
            |_name, _| Ok::<_, &'static str>(Some(Value::Integer(7))),
        )
        .unwrap();

    assert_eq!(first.variables(), second.variables());
    assert_eq!(first.rng_state(), second.rng_state());
}

#[test]
fn defaults_are_explicitly_rejected_in_fragments() {
    let mut session = ReplSession::new();
    let error = session.execute("default hp = 3\n").unwrap_err();
    assert!(matches!(error, ReplError::DefaultNotAllowed { line: 1 }));
    assert!(session.variables().is_empty());
}
