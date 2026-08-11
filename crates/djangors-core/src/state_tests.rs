use super::*;

#[test]
fn test_app_state_roundtrip() {
    let state = AppState::new().insert(42i32).insert("hello".to_string());

    assert_eq!(state.get::<i32>(), Some(&42i32));
    assert_eq!(state.get::<String>(), Some(&"hello".to_string()));
    assert_eq!(state.get::<bool>(), None);
}
