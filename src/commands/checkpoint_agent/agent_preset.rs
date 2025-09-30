use crate::{
    error::GitAiError,
    log_fmt::{
        transcript::{AiTranscript, Message},
        working_log::AgentId,
    },
};
use rusqlite::{Connection, OpenFlags};
use std::env;
use std::path::{Path, PathBuf};

pub struct AgentCheckpointFlags {
    pub prompt_id: Option<String>,
    pub hook_input: Option<String>,
}

pub struct AgentRunResult {
    pub agent_id: AgentId,
    pub is_human: bool,
    pub transcript: Option<AiTranscript>,
    pub repo_working_dir: Option<String>,
}

pub trait AgentCheckpointPreset {
    fn run(&self, flags: AgentCheckpointFlags) -> Result<AgentRunResult, GitAiError>;
}

// Claude Code to checkpoint preset
pub struct ClaudePreset;

impl AgentCheckpointPreset for ClaudePreset {
    fn run(&self, flags: AgentCheckpointFlags) -> Result<AgentRunResult, GitAiError> {
        // Parse claude_hook_stdin as JSON
        let stdin_json = flags.hook_input.ok_or_else(|| {
            GitAiError::PresetError("hook_input is required for Claude preset".to_string())
        })?;

        let hook_data: serde_json::Value = serde_json::from_str(&stdin_json)
            .map_err(|e| GitAiError::PresetError(format!("Invalid JSON in hook_input: {}", e)))?;

        // Extract transcript_path and cwd from the JSON
        let transcript_path = hook_data
            .get("transcript_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                GitAiError::PresetError("transcript_path not found in hook_input".to_string())
            })?;

        let _cwd = hook_data
            .get("cwd")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GitAiError::PresetError("cwd not found in hook_input".to_string()))?;

        // Extract the ID from the filename
        // Example: /Users/aidancunniffe/.claude/projects/-Users-aidancunniffe-Desktop-ghq/cb947e5b-246e-4253-a953-631f7e464c6b.jsonl
        let path = Path::new(transcript_path);
        let filename = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| {
                GitAiError::PresetError(
                    "Could not extract filename from transcript_path".to_string(),
                )
            })?;

        // Read the file content
        let jsonl_content =
            std::fs::read_to_string(transcript_path).map_err(|e| GitAiError::IoError(e))?;

        // Parse into transcript and extract model
        let (transcript, model) = AiTranscript::from_claude_code_jsonl_with_model(&jsonl_content)
            .map_err(|e| GitAiError::JsonError(e))?;

        // The filename should be a UUID
        let agent_id = AgentId {
            tool: "claude".to_string(),
            id: filename.to_string(),
            model: model.unwrap_or_else(|| "unknown".to_string()),
        };

        Ok(AgentRunResult {
            agent_id,
            is_human: false,
            transcript: Some(transcript),
            // use default.
            repo_working_dir: None,
        })
    }
}

// Cursor to checkpoint preset
pub struct CursorPreset;

impl AgentCheckpointPreset for CursorPreset {
    fn run(&self, flags: AgentCheckpointFlags) -> Result<AgentRunResult, GitAiError> {
        // Parse hook_input JSON to extract workspace_roots and conversation_id
        let hook_input_json = flags.hook_input.ok_or_else(|| {
            GitAiError::PresetError("hook_input is required for Cursor preset".to_string())
        })?;

        let hook_data: serde_json::Value = serde_json::from_str(&hook_input_json)
            .map_err(|e| GitAiError::PresetError(format!("Invalid JSON in hook_input: {}", e)))?;

        // Extract conversation_id and workspace_roots from the JSON
        let conversation_id = hook_data
            .get("conversation_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                GitAiError::PresetError("conversation_id not found in hook_input".to_string())
            })?
            .to_string();

        let workspace_roots = hook_data
            .get("workspace_roots")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                GitAiError::PresetError("workspace_roots not found in hook_input".to_string())
            })?
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect::<Vec<String>>();

        let hook_event_name = hook_data
            .get("hook_event_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                GitAiError::PresetError("hook_event_name not found in hook_input".to_string())
            })?
            .to_string();

        // Validate hook_event_name
        if hook_event_name != "beforeSubmitPrompt" && hook_event_name != "afterFileEdit" {
            return Err(GitAiError::PresetError(format!(
                "Invalid hook_event_name: {}. Expected 'beforeSubmitPrompt' or 'afterFileEdit'",
                hook_event_name
            )));
        }

        let repo_working_dir = workspace_roots.first().cloned().ok_or_else(|| {
            GitAiError::PresetError("No workspace root found in hook_input".to_string())
        })?;

        if hook_event_name == "beforeSubmitPrompt" {
            // early return, we're just adding a human checkpoint.
            return Ok(AgentRunResult {
                agent_id: AgentId {
                    tool: "cursor".to_string(),
                    id: conversation_id.clone(),
                    model: "unknown".to_string(),
                },
                is_human: true,
                transcript: None,
                repo_working_dir: Some(repo_working_dir),
            });
        }

        // Use prompt_id if provided, otherwise use conversation_id
        let composer_id = flags.prompt_id.unwrap_or(conversation_id);

        // Locate Cursor storage
        let user_dir = Self::cursor_user_dir()?;
        let global_db = user_dir.join("globalStorage").join("state.vscdb");
        if !global_db.exists() {
            return Err(GitAiError::PresetError(format!(
                "Cursor global state database not found at {:?}. \
                Make sure Cursor is installed and has been used at least once. \
                Expected location: {:?}",
                global_db,
                user_dir.join("globalStorage")
            )));
        }

        // Fetch the composer data and extract transcript
        let payload = Self::fetch_composer_payload(&global_db, &composer_id)?;
        let transcript = Self::transcript_from_composer_payload(
            &payload,
            &global_db,
            &composer_id,
        )?
        .unwrap_or_else(|| {
            // Return empty transcript as default
            // There's a race condition causing new threads to sometimes not show up.
            // We refresh and grab all the messages in post-commit so we're ok with returning an empty (placeholder)transcript here and not throwing
            println!(
                "[Warning] Could not extract transcript from Cursor composer. Retrying at commit."
            );
            AiTranscript::new()
        });

        // Extract model information from the Cursor data
        let model = Self::extract_model_from_cursor_data(&payload, &global_db, &composer_id)?;

        let agent_id = AgentId {
            tool: "cursor".to_string(),
            id: composer_id,
            model,
        };

        Ok(AgentRunResult {
            agent_id,
            is_human: false,
            transcript: Some(transcript),
            repo_working_dir: Some(repo_working_dir),
        })
    }
}

impl CursorPreset {
    /// Update Cursor conversations in working logs to their latest versions
    /// This helps prevent race conditions where we miss the last message in a conversation
    pub fn update_cursor_conversations_to_latest(
        checkpoints: &mut [crate::log_fmt::working_log::Checkpoint],
    ) -> Result<(), GitAiError> {
        use std::collections::HashMap;

        // Group checkpoints by Cursor conversation ID
        let mut cursor_conversations: HashMap<
            String,
            Vec<&mut crate::log_fmt::working_log::Checkpoint>,
        > = HashMap::new();

        for checkpoint in checkpoints.iter_mut() {
            if let Some(agent_id) = &checkpoint.agent_id {
                if agent_id.tool == "cursor" {
                    cursor_conversations
                        .entry(agent_id.id.clone())
                        .or_insert_with(Vec::new)
                        .push(checkpoint);
                }
            }
        }

        // For each unique Cursor conversation, fetch the latest version
        for (conversation_id, conversation_checkpoints) in cursor_conversations {
            // Fetch the latest conversation data
            match Self::fetch_latest_cursor_conversation(&conversation_id) {
                Ok(Some((latest_transcript, latest_model))) => {
                    // Update all checkpoints for this conversation
                    for checkpoint in conversation_checkpoints {
                        if let Some(agent_id) = &mut checkpoint.agent_id {
                            agent_id.model = latest_model.clone();
                        }
                        checkpoint.transcript = Some(latest_transcript.clone());
                    }
                }
                Ok(None) => {
                    // No latest conversation data found, continue with existing data
                }
                Err(_) => {
                    // Failed to fetch latest conversation, continue with existing data
                }
            }
        }

        Ok(())
    }

    /// Fetch the latest version of a Cursor conversation from the database
    fn fetch_latest_cursor_conversation(
        conversation_id: &str,
    ) -> Result<Option<(AiTranscript, String)>, GitAiError> {
        // Get Cursor user directory
        let user_dir = Self::cursor_user_dir()?;
        let global_db = user_dir.join("globalStorage").join("state.vscdb");

        if !global_db.exists() {
            return Ok(None);
        }

        // Fetch composer payload
        let composer_payload = Self::fetch_composer_payload(&global_db, conversation_id)?;

        // Extract transcript and model
        let transcript =
            Self::transcript_from_composer_payload(&composer_payload, &global_db, conversation_id)?;
        let model =
            Self::extract_model_from_cursor_data(&composer_payload, &global_db, conversation_id)?;

        match transcript {
            Some(transcript) => Ok(Some((transcript, model))),
            None => Ok(None),
        }
    }

    fn cursor_user_dir() -> Result<PathBuf, GitAiError> {
        #[cfg(target_os = "windows")]
        {
            // Windows: %APPDATA%\Cursor\User
            let appdata = env::var("APPDATA")
                .map_err(|e| GitAiError::Generic(format!("APPDATA not set: {}", e)))?;
            Ok(Path::new(&appdata).join("Cursor").join("User"))
        }

        #[cfg(target_os = "macos")]
        {
            // macOS: ~/Library/Application Support/Cursor/User
            let home = env::var("HOME")
                .map_err(|e| GitAiError::Generic(format!("HOME not set: {}", e)))?;
            Ok(Path::new(&home)
                .join("Library")
                .join("Application Support")
                .join("Cursor")
                .join("User"))
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            Err(GitAiError::PresetError(
                "Cursor is only supported on Windows and macOS platforms".to_string(),
            ))
        }
    }

    fn open_sqlite_readonly(path: &Path) -> Result<Connection, GitAiError> {
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| GitAiError::Generic(format!("Failed to open {:?}: {}", path, e)))
    }

    fn fetch_composer_payload(
        global_db_path: &Path,
        composer_id: &str,
    ) -> Result<serde_json::Value, GitAiError> {
        let conn = Self::open_sqlite_readonly(global_db_path)?;

        // Look for the composer data in cursorDiskKV
        let key_pattern = format!("composerData:{}", composer_id);
        let mut stmt = conn
            .prepare("SELECT value FROM cursorDiskKV WHERE key = ?")
            .map_err(|e| GitAiError::Generic(format!("Query failed: {}", e)))?;

        let mut rows = stmt
            .query([&key_pattern])
            .map_err(|e| GitAiError::Generic(format!("Query failed: {}", e)))?;

        if let Ok(Some(row)) = rows.next() {
            let value_text: String = row
                .get(0)
                .map_err(|e| GitAiError::Generic(format!("Failed to read value: {}", e)))?;

            let data = serde_json::from_str::<serde_json::Value>(&value_text)
                .map_err(|e| GitAiError::Generic(format!("Failed to parse JSON: {}", e)))?;

            return Ok(data);
        }

        Err(GitAiError::PresetError(
            "No conversation data found in database".to_string(),
        ))
    }

    fn transcript_from_composer_payload(
        data: &serde_json::Value,
        global_db_path: &Path,
        composer_id: &str,
    ) -> Result<Option<AiTranscript>, GitAiError> {
        // Only support fullConversationHeadersOnly (bubbles format) - the current Cursor format
        // All conversations since April 2025 use this format exclusively
        let conv = data
            .get("fullConversationHeadersOnly")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                GitAiError::PresetError(
                    "Conversation uses unsupported legacy format. Only conversations created after April 2025 are supported.".to_string()
                )
            })?;

        let mut transcript = AiTranscript::new();

        for header in conv.iter() {
            if let Some(bubble_id) = header.get("bubbleId").and_then(|v| v.as_str()) {
                if let Ok(Some(bubble_content)) =
                    Self::fetch_bubble_content_from_db(global_db_path, composer_id, bubble_id)
                {
                    // Extract text from bubble
                    if let Some(text) = bubble_content.get("text").and_then(|v| v.as_str()) {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            let role = header.get("type").and_then(|v| v.as_i64()).unwrap_or(0);
                            if role == 1 {
                                transcript.add_message(Message::user(trimmed.to_string()));
                            } else {
                                transcript.add_message(Message::assistant(trimmed.to_string()));
                            }
                        }
                    }

                    // Handle content arrays for tool_use and structured content
                    if let Some(content_array) =
                        bubble_content.get("content").and_then(|v| v.as_array())
                    {
                        for item in content_array {
                            match item.get("type").and_then(|v| v.as_str()) {
                                Some("text") => {
                                    if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                                        let trimmed = text.trim();
                                        if !trimmed.is_empty() {
                                            let role = header
                                                .get("type")
                                                .and_then(|v| v.as_i64())
                                                .unwrap_or(0);
                                            if role == 1 {
                                                transcript.add_message(Message::user(
                                                    trimmed.to_string(),
                                                ));
                                            } else {
                                                transcript.add_message(Message::assistant(
                                                    trimmed.to_string(),
                                                ));
                                            }
                                        }
                                    }
                                }
                                Some("tool_use") => {
                                    let name_opt = item.get("name").and_then(|v| v.as_str());
                                    let input_val = item.get("input").cloned();
                                    if let (Some(name), Some(input)) = (name_opt, input_val) {
                                        transcript.add_message(Message::tool_use(
                                            name.to_string(),
                                            input,
                                        ));
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        if !transcript.messages.is_empty() {
            Ok(Some(transcript))
        } else {
            Ok(None)
        }
    }

    fn fetch_bubble_content_from_db(
        global_db_path: &Path,
        composer_id: &str,
        bubble_id: &str,
    ) -> Result<Option<serde_json::Value>, GitAiError> {
        let conn = Self::open_sqlite_readonly(global_db_path)?;

        // Look for bubble data in cursorDiskKV with pattern bubbleId:composerId:bubbleId
        let bubble_pattern = format!("bubbleId:{}:{}", composer_id, bubble_id);
        let mut stmt = conn
            .prepare("SELECT value FROM cursorDiskKV WHERE key = ?")
            .map_err(|e| GitAiError::Generic(format!("Query failed: {}", e)))?;

        let mut rows = stmt
            .query([&bubble_pattern])
            .map_err(|e| GitAiError::Generic(format!("Query failed: {}", e)))?;

        if let Ok(Some(row)) = rows.next() {
            let value_text: String = row
                .get(0)
                .map_err(|e| GitAiError::Generic(format!("Failed to read value: {}", e)))?;

            let data = serde_json::from_str::<serde_json::Value>(&value_text)
                .map_err(|e| GitAiError::Generic(format!("Failed to parse JSON: {}", e)))?;

            return Ok(Some(data));
        }

        Ok(None)
    }

    fn extract_model_from_cursor_data(
        composer_payload: &serde_json::Value,
        global_db_path: &Path,
        composer_id: &str,
    ) -> Result<String, GitAiError> {
        // @todo Aidan run some tests once we get cursor support and confirm these mappings are correct
        // First, check if the composer payload has capabilityType
        if let Some(capability_type) = composer_payload.get("capabilityType") {
            if let Some(cap_num) = capability_type.as_i64() {
                // let model = match cap_num {
                //     15 => "claude-3.5-sonnet", // Based on observed capabilityType value
                //     14 => "claude-3-sonnet",
                //     13 => "claude-3-haiku",
                //     12 => "gpt-4",
                //     11 => "gpt-4-turbo",
                //     10 => "gpt-3.5-turbo",
                //     _ => "unknown",
                // };
                return Ok(cap_num.to_string());
            }
        }

        // If not found in composer payload, check bubble content for model info
        if let Some(conv) = composer_payload
            .get("fullConversationHeadersOnly")
            .and_then(|v| v.as_array())
        {
            for header in conv.iter() {
                if let Some(bubble_id) = header.get("bubbleId").and_then(|v| v.as_str()) {
                    if let Ok(Some(bubble_content)) =
                        Self::fetch_bubble_content_from_db(global_db_path, composer_id, bubble_id)
                    {
                        // Check capabilityType in bubble
                        if let Some(capability_type) = bubble_content.get("capabilityType") {
                            if let Some(cap_num) = capability_type.as_i64() {
                                let model = match cap_num {
                                    // @todo Aidan to figure out the rest of these mappings
                                    30 => "gpt-5-codex",
                                    15 => "claude-3.5-sonnet",
                                    _ => return Ok(cap_num.to_string()),
                                };
                                return Ok(model.to_string());
                            }
                        }

                        // Check toolFormerData for model information
                        if let Some(tool_former_data) = bubble_content.get("toolFormerData") {
                            // Look for model information in the toolFormerData
                            if let Some(_model_call_id) = tool_former_data.get("modelCallId") {
                                // The presence of modelCallId suggests this is an AI interaction
                                // We can infer the model from other context or use a default
                                return Ok("claude-3.5-sonnet".to_string()); // Default for Cursor AI interactions
                            }
                        }
                    }
                }
            }
        }

        // Fallback: return a default model for Cursor
        Ok("claude-3.5-sonnet".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_preset_with_conversation_id() {
        // This test requires a real Cursor installation with at least one conversation.
        // We'll use a sample conversation_id (you can replace with an actual ID from your database)

        // First, let's try to get a real conversation ID from the database
        let user_dir = match CursorPreset::cursor_user_dir() {
            Ok(dir) => dir,
            Err(_) => {
                println!("Cursor not installed, skipping test");
                return;
            }
        };

        let global_db = user_dir.join("globalStorage").join("state.vscdb");
        if !global_db.exists() {
            println!("Cursor database not found, skipping test");
            return;
        }

        // Get a real conversation ID from the database
        let conn = match CursorPreset::open_sqlite_readonly(&global_db) {
            Ok(c) => c,
            Err(_) => {
                println!("Could not open Cursor database, skipping test");
                return;
            }
        };

        // Find a conversation with fullConversationHeadersOnly (bubbles format)
        let mut stmt = match conn.prepare(
            "SELECT json_extract(value, '$.composerId') FROM cursorDiskKV 
             WHERE key LIKE 'composerData:%' 
             AND json_extract(value, '$.fullConversationHeadersOnly') IS NOT NULL
             AND json_array_length(json_extract(value, '$.fullConversationHeadersOnly')) > 0
             LIMIT 1",
        ) {
            Ok(s) => s,
            Err(_) => {
                println!("Could not query database, skipping test");
                return;
            }
        };

        let conversation_id: String = match stmt.query_row([], |row| row.get(0)) {
            Ok(id) => id,
            Err(_) => {
                println!("No conversations with messages found in database, skipping test");
                return;
            }
        };

        println!("Testing with conversation_id: {}", conversation_id);

        // Create mock hook_input
        let hook_input = serde_json::json!({
            "conversation_id": conversation_id,
            "workspace_roots": ["/tmp/test-workspace"]
        });

        let preset = CursorPreset;
        let result = preset.run(AgentCheckpointFlags {
            prompt_id: None,
            hook_input: Some(hook_input.to_string()),
        });

        match result {
            Ok(run) => {
                println!("✓ Cursor Agent: {}:{}", run.agent_id.tool, run.agent_id.id);
                println!("✓ Model: {}", run.agent_id.model);
                let transcript = run.transcript.unwrap();
                println!("✓ Message count: {}", transcript.messages.len());

                for (i, m) in transcript.messages.iter().enumerate() {
                    match m {
                        Message::User { text, .. } => {
                            let preview = if text.len() > 100 {
                                format!("{}...", &text[..100])
                            } else {
                                text.clone()
                            };
                            println!("  [{}] User: {}", i, preview);
                        }
                        Message::Assistant { text, .. } => {
                            let preview = if text.len() > 100 {
                                format!("{}...", &text[..100])
                            } else {
                                text.clone()
                            };
                            println!("  [{}] Assistant: {}", i, preview);
                        }
                        Message::ToolUse { name, input, .. } => {
                            println!(
                                "  [{}] ToolUse: {} (input: {} chars)",
                                i,
                                name,
                                input.to_string().len()
                            );
                        }
                    }
                }

                // Assert that we got at least some messages
                assert!(
                    !transcript.messages.is_empty(),
                    "Transcript should have at least one message"
                );
                assert_eq!(run.agent_id.tool, "cursor");
                assert_eq!(run.agent_id.id, conversation_id);
            }
            Err(e) => {
                panic!("CursorPreset error: {}", e);
            }
        }
    }
}
