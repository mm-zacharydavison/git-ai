mod test_utils;

use rusqlite::{Connection, OpenFlags};
use test_utils::fixture_path;

const TEST_CONVERSATION_ID: &str = "00812842-49fe-4699-afae-bb22cda3f6e1";

/// Helper function to open the test cursor database in read-only mode
fn open_test_db() -> Connection {
    let db_path = fixture_path("cursor_test.vscdb");
    Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("Failed to open test cursor database")
}

#[test]
fn test_can_open_cursor_test_database() {
    let conn = open_test_db();

    // Verify we can query the database
    let mut stmt = conn
        .prepare("SELECT COUNT(*) FROM cursorDiskKV")
        .expect("Failed to prepare statement");

    let count: i64 = stmt
        .query_row([], |row| row.get(0))
        .expect("Failed to query");

    assert!(count > 0, "Database should have some records");
}

#[test]
fn test_cursor_database_has_composer_data() {
    let conn = open_test_db();

    // Check that we have the expected composer data
    let mut stmt = conn
        .prepare("SELECT key FROM cursorDiskKV WHERE key LIKE 'composerData:%'")
        .expect("Failed to prepare statement");

    let keys: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .expect("Failed to query")
        .collect::<Result<Vec<_>, _>>()
        .expect("Failed to collect keys");

    assert!(!keys.is_empty(), "Should have at least one composer");
    assert!(
        keys.contains(&format!("composerData:{}", TEST_CONVERSATION_ID)),
        "Should contain the test conversation"
    );
}

#[test]
fn test_cursor_database_has_bubble_data() {
    let conn = open_test_db();

    // Check that we have bubble data for the test conversation
    let pattern = format!("bubbleId:{}:%", TEST_CONVERSATION_ID);
    let mut stmt = conn
        .prepare("SELECT COUNT(*) FROM cursorDiskKV WHERE key LIKE ?")
        .expect("Failed to prepare statement");

    let count: i64 = stmt
        .query_row([&pattern], |row| row.get(0))
        .expect("Failed to query");

    assert!(
        count > 0,
        "Should have at least one bubble for the conversation"
    );
    println!("Found {} bubbles in test conversation", count);
}

#[test]
fn test_fetch_composer_payload_from_test_db() {
    use serde_json::Value;

    let conn = open_test_db();

    // Manually test the logic that CursorPreset::fetch_composer_payload uses
    let key_pattern = format!("composerData:{}", TEST_CONVERSATION_ID);
    let mut stmt = conn
        .prepare("SELECT value FROM cursorDiskKV WHERE key = ?")
        .expect("Failed to prepare statement");

    let value_text: String = stmt
        .query_row([&key_pattern], |row| row.get(0))
        .expect("Failed to query composer data");

    let data: Value =
        serde_json::from_str(&value_text).expect("Failed to parse composer data as JSON");

    // Verify the structure
    assert!(
        data.get("fullConversationHeadersOnly").is_some(),
        "Should have fullConversationHeadersOnly field"
    );

    let headers = data
        .get("fullConversationHeadersOnly")
        .and_then(|v| v.as_array())
        .expect("fullConversationHeadersOnly should be an array");

    assert!(
        !headers.is_empty(),
        "Should have at least one conversation header"
    );
    println!("Found {} conversation headers", headers.len());

    // Check that headers have bubbleId
    let first_header = &headers[0];
    assert!(
        first_header.get("bubbleId").is_some(),
        "Header should have bubbleId"
    );
}

#[test]
fn test_fetch_bubble_content_from_test_db() {
    use serde_json::Value;

    let conn = open_test_db();

    // First, get a bubble ID from the composer data
    let key_pattern = format!("composerData:{}", TEST_CONVERSATION_ID);
    let mut stmt = conn
        .prepare("SELECT value FROM cursorDiskKV WHERE key = ?")
        .expect("Failed to prepare statement");

    let value_text: String = stmt
        .query_row([&key_pattern], |row| row.get(0))
        .expect("Failed to query composer data");

    let data: Value = serde_json::from_str(&value_text).expect("Failed to parse JSON");

    let headers = data
        .get("fullConversationHeadersOnly")
        .and_then(|v| v.as_array())
        .expect("Should have headers");

    let first_bubble_id = headers[0]
        .get("bubbleId")
        .and_then(|v| v.as_str())
        .expect("Should have bubble ID");

    println!("Testing with bubble ID: {}", first_bubble_id);

    // Now fetch the bubble content
    let bubble_pattern = format!("bubbleId:{}:{}", TEST_CONVERSATION_ID, first_bubble_id);
    let mut stmt = conn
        .prepare("SELECT value FROM cursorDiskKV WHERE key = ?")
        .expect("Failed to prepare statement");

    let bubble_text: String = stmt
        .query_row([&bubble_pattern], |row| row.get(0))
        .expect("Failed to query bubble data");

    let bubble_data: Value =
        serde_json::from_str(&bubble_text).expect("Failed to parse bubble JSON");

    // Verify bubble structure
    assert!(
        bubble_data.get("text").is_some() || bubble_data.get("content").is_some(),
        "Bubble should have text or content field"
    );

    println!("Successfully fetched and parsed bubble content");
}

#[test]
fn test_extract_messages_from_test_conversation() {
    use serde_json::Value;

    let conn = open_test_db();

    // Get composer data
    let key_pattern = format!("composerData:{}", TEST_CONVERSATION_ID);
    let mut stmt = conn
        .prepare("SELECT value FROM cursorDiskKV WHERE key = ?")
        .expect("Failed to prepare statement");

    let value_text: String = stmt
        .query_row([&key_pattern], |row| row.get(0))
        .expect("Failed to query");

    let data: Value = serde_json::from_str(&value_text).expect("Failed to parse JSON");

    let headers = data
        .get("fullConversationHeadersOnly")
        .and_then(|v| v.as_array())
        .expect("Should have headers");

    let mut message_count = 0;

    // Iterate through headers and fetch bubble content
    for header in headers {
        if let Some(bubble_id) = header.get("bubbleId").and_then(|v| v.as_str()) {
            let bubble_pattern = format!("bubbleId:{}:{}", TEST_CONVERSATION_ID, bubble_id);
            let mut stmt = conn
                .prepare("SELECT value FROM cursorDiskKV WHERE key = ?")
                .expect("Failed to prepare statement");

            if let Ok(bubble_text) =
                stmt.query_row([&bubble_pattern], |row| row.get::<_, String>(0))
            {
                if let Ok(bubble_data) = serde_json::from_str::<Value>(&bubble_text) {
                    // Check if this bubble has text content
                    if let Some(text) = bubble_data.get("text").and_then(|v| v.as_str()) {
                        if !text.trim().is_empty() {
                            message_count += 1;
                            let role = header.get("type").and_then(|v| v.as_i64()).unwrap_or(0);
                            let role_str = if role == 1 { "User" } else { "Assistant" };
                            println!(
                                "Message {}: {} - {}",
                                message_count,
                                role_str,
                                &text[..text.len().min(50)]
                            );
                        }
                    }

                    // Also check for content array
                    if let Some(content_array) =
                        bubble_data.get("content").and_then(|v| v.as_array())
                    {
                        for item in content_array {
                            if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                                if !text.trim().is_empty() {
                                    message_count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    assert!(
        message_count > 0,
        "Should extract at least one message from the conversation"
    );
    println!("Successfully extracted {} messages", message_count);
}
