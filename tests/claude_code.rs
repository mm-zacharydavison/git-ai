mod test_utils;

use git_ai::authorship::transcript::{AiTranscript, Message};
use test_utils::load_fixture;

#[test]
fn test_parse_example_claude_code_jsonl_with_model() {
    let jsonl_content = load_fixture("example-claude-code.jsonl");

    let (transcript, model) = AiTranscript::from_claude_code_jsonl_with_model(&jsonl_content)
        .expect("Failed to parse JSONL");

    // Verify we parsed some messages
    assert!(!transcript.messages().is_empty());

    // Verify we extracted the model
    assert!(model.is_some());
    let model_name = model.unwrap();
    println!("Extracted model: {}", model_name);

    // Based on the example file, we should get claude-sonnet-4-20250514
    assert_eq!(model_name, "claude-sonnet-4-20250514");

    // Print the parsed transcript for inspection
    println!("Parsed {} messages:", transcript.messages().len());
    for (i, message) in transcript.messages().iter().enumerate() {
        match message {
            Message::User { text, .. } => println!("{}: User: {}", i, text),
            Message::Assistant { text, .. } => println!("{}: Assistant: {}", i, text),
            Message::ToolUse { name, input, .. } => {
                println!("{}: ToolUse: {} with input: {:?}", i, name, input)
            }
        }
    }
}
