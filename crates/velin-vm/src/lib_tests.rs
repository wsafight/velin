use super::SetVariableError;

#[test]
fn formats_every_variant() {
    assert_eq!(
        SetVariableError::UnknownVariable("hp".into()).to_string(),
        "unknown variable `hp`"
    );
    assert_eq!(
        SetVariableError::UnknownSlot(7).to_string(),
        "unknown slot `7`"
    );
    assert_eq!(SetVariableError::InvalidValue("bad").to_string(), "bad");
    assert_eq!(SetVariableError::StateBudget("full").to_string(), "full");
}
