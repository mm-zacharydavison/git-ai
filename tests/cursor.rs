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

    assert_eq!(count, 50, "Database should have exactly 50 records");
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

    assert_eq!(
        count, 42,
        "Should have exactly 42 bubbles for the test conversation"
    );
}

#[test]
fn test_fetch_composer_payload_from_test_db() {
    use git_ai::commands::checkpoint_agent::agent_preset::CursorPreset;

    let db_path = fixture_path("cursor_test.vscdb");

    // Use the actual CursorPreset function
    let composer_payload = CursorPreset::fetch_composer_payload(&db_path, TEST_CONVERSATION_ID)
        .expect("Should fetch composer payload");

    // Verify the structure
    assert!(
        composer_payload
            .get("fullConversationHeadersOnly")
            .is_some(),
        "Should have fullConversationHeadersOnly field"
    );

    let headers = composer_payload
        .get("fullConversationHeadersOnly")
        .and_then(|v| v.as_array())
        .expect("fullConversationHeadersOnly should be an array");

    assert_eq!(
        headers.len(),
        42,
        "Should have exactly 42 conversation headers"
    );

    // Check that first header has bubbleId
    let first_header = &headers[0];
    assert!(
        first_header.get("bubbleId").is_some(),
        "Header should have bubbleId"
    );
}

#[test]
fn test_fetch_bubble_content_from_test_db() {
    use git_ai::commands::checkpoint_agent::agent_preset::CursorPreset;

    let db_path = fixture_path("cursor_test.vscdb");

    // First, get a bubble ID from the composer data using actual function
    let composer_payload = CursorPreset::fetch_composer_payload(&db_path, TEST_CONVERSATION_ID)
        .expect("Should fetch composer payload");

    let headers = composer_payload
        .get("fullConversationHeadersOnly")
        .and_then(|v| v.as_array())
        .expect("Should have headers");

    let first_bubble_id = headers[0]
        .get("bubbleId")
        .and_then(|v| v.as_str())
        .expect("Should have bubble ID");

    // Use the actual CursorPreset function to fetch bubble content
    let bubble_data =
        CursorPreset::fetch_bubble_content_from_db(&db_path, TEST_CONVERSATION_ID, first_bubble_id)
            .expect("Should fetch bubble content")
            .expect("Bubble content should exist");

    // Verify bubble structure
    assert!(
        bubble_data.get("text").is_some() || bubble_data.get("content").is_some(),
        "Bubble should have text or content field"
    );
}

#[test]
fn test_extract_transcript_from_test_conversation() {
    use git_ai::commands::checkpoint_agent::agent_preset::CursorPreset;

    let db_path = fixture_path("cursor_test.vscdb");

    // Use the actual CursorPreset function to extract transcript data
    let composer_payload = CursorPreset::fetch_composer_payload(&db_path, TEST_CONVERSATION_ID)
        .expect("Should fetch composer payload");

    let transcript_data = CursorPreset::transcript_data_from_composer_payload(
        &composer_payload,
        &db_path,
        TEST_CONVERSATION_ID,
    )
    .expect("Should extract transcript data")
    .expect("Should have transcript data");

    let (transcript, model) = transcript_data;

    // Verify exact message count
    assert_eq!(
        transcript.messages().len(),
        13,
        "Should extract exactly 13 messages from the conversation"
    );

    // Verify model extraction
    assert_eq!(model, "gpt-5", "Model should be 'gpt-5'");
}
