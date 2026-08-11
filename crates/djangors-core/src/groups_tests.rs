use super::*;

#[tokio::test]
async fn test_groups_broadcast_with_no_receivers() {
    let groups = Groups::new();
    let handle = groups.group("events");
    // Should not panic or error even with 0 receivers
    handle.send("hello");
}

#[tokio::test]
async fn test_groups_subscribe_and_receive() {
    use tokio_stream::StreamExt;

    let groups = Groups::new();
    let handle = groups.group("news");
    let mut stream = handle.subscribe();

    handle.send("breaking news");

    let msg = stream.next().await;
    assert_eq!(msg, Some("breaking news".to_string()));
}
