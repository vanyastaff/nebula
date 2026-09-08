fn main() {
    fn name_turn_handoff<T: nebula_sdk::integration::action::ExecutionTurnHandoff>() {}
    let _ = name_turn_handoff::<()>;
}
