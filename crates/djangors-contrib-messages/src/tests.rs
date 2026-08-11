use super::*;

#[test]
fn test_add_then_take_round_trip() {
    let session = Session::new_empty();

    add(&session, Level::Info, "First message");
    add_success(&session, "Second message");
    add_error(&session, "Third message");

    let messages = take(&session);
    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages[0],
        Message {
            level: Level::Info,
            text: "First message".to_string()
        }
    );
    assert_eq!(
        messages[1],
        Message {
            level: Level::Success,
            text: "Second message".to_string()
        }
    );
    assert_eq!(
        messages[2],
        Message {
            level: Level::Error,
            text: "Third message".to_string()
        }
    );

    // Order preserved
}

#[test]
fn test_take_clears_queue() {
    let session = Session::new_empty();

    add_warning(&session, "Caution");
    let first_take = take(&session);
    assert_eq!(first_take.len(), 1);

    let second_take = take(&session);
    assert!(
        second_take.is_empty(),
        "Second call to take must return an empty vector"
    );
}

#[test]
fn test_level_serialization() {
    let levels = vec![
        Level::Debug,
        Level::Info,
        Level::Success,
        Level::Warning,
        Level::Error,
    ];

    for level in levels {
        let msg = Message::new(level.clone(), "test");
        let json = serde_json::to_string(&msg).expect("Serialize message");
        let deserialized: Message = serde_json::from_str(&json).expect("Deserialize message");
        assert_eq!(deserialized.level, level);
        assert_eq!(deserialized.text, "test");
    }
}
