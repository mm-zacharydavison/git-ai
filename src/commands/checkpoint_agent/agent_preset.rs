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
    pub transcript: AiTranscript,
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

        // The filename should be a UUID
        let agent_id = AgentId {
            tool: "claude".to_string(),
            id: filename.to_string(),
        };

        // Read the file content
        let jsonl_content =
            std::fs::read_to_string(transcript_path).map_err(|e| GitAiError::IoError(e))?;

        // Parse into transcript
        let transcript = AiTranscript::from_claude_code_jsonl(&jsonl_content)
            .map_err(|e| GitAiError::JsonError(e))?;

        Ok(AgentRunResult {
            agent_id,
            transcript,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ConversationMetadata {
    pub composer_id: String,
    #[allow(dead_code)]
    pub key: String,
    #[allow(dead_code)]
    pub title: String,
    #[allow(dead_code)]
    pub last_message_date: Option<i64>,
    #[allow(dead_code)]
    pub message_count: i32,
    #[allow(dead_code)]
    pub status: String,
    #[allow(dead_code)]
    pub created_at: i64,
    #[allow(dead_code)]
    pub preview: String,
}

// Cursor to checkpoint preset
pub struct CursorPreset;

impl CursorPreset {
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

    fn get_latest_conversations(
        global_db_path: &Path,
        limit: Option<i32>,
    ) -> Result<Vec<ConversationMetadata>, GitAiError> {
        let conn = Self::open_sqlite_readonly(global_db_path)?;

        let limit_clause = match limit {
            Some(l) => format!(" LIMIT {}", l),
            None => String::new(),
        };

        let query = format!(
            "SELECT 
                key,
                value,
                json_extract(value, '$.composerId') as composerId,
                json_extract(value, '$.createdAt') as createdAt,
                json_extract(value, '$.lastUpdatedAt') as lastUpdatedAt,
                json_extract(value, '$.status') as status,
                json_extract(value, '$.name') as name,
                json_array_length(json_extract(value, '$.conversation')) as conversationCount,
                json_array_length(json_extract(value, '$.fullConversationHeadersOnly')) as headersCount
            FROM cursorDiskKV 
            WHERE key LIKE 'composerData:%'
              AND (
                json_array_length(json_extract(value, '$.conversation')) > 0 
                OR json_array_length(json_extract(value, '$.fullConversationHeadersOnly')) > 0
              )
            ORDER BY 
              COALESCE(json_extract(value, '$.lastUpdatedAt'), json_extract(value, '$.createdAt'), 0) DESC{}",
            limit_clause
        );

        let mut stmt = conn
            .prepare(&query)
            .map_err(|e| GitAiError::Generic(format!("Failed to prepare query: {}", e)))?;

        let mut rows = stmt
            .query([])
            .map_err(|e| GitAiError::Generic(format!("Query failed: {}", e)))?;

        let mut conversations = Vec::new();
        while let Ok(Some(row)) = rows.next() {
            let key: String = row
                .get(0)
                .map_err(|e| GitAiError::Generic(format!("Failed to read key: {}", e)))?;

            let value_text: String = row
                .get(1)
                .map_err(|e| GitAiError::Generic(format!("Failed to read value: {}", e)))?;

            let composer_id: Option<String> = row.get(2).unwrap_or_default();
            let created_at: Option<i64> = row.get(3).unwrap_or_default();
            let last_updated_at: Option<i64> = row.get(4).unwrap_or_default();
            let status: Option<String> = row.get(5).unwrap_or_default();
            let _name: Option<String> = row.get(6).unwrap_or_default();
            let conversation_count: Option<i32> = row.get(7).unwrap_or_default();
            let headers_count: Option<i32> = row.get(8).unwrap_or_default();

            // Parse the conversation data
            let conversation_data = serde_json::from_str::<serde_json::Value>(&value_text)
                .map_err(|e| GitAiError::Generic(format!("Failed to parse JSON: {}", e)))?;

            // Determine message count
            let message_count =
                std::cmp::max(conversation_count.unwrap_or(0), headers_count.unwrap_or(0));

            let metadata = ConversationMetadata {
                composer_id: composer_id.unwrap_or_else(|| "unknown".to_string()),
                key,
                title: Self::extract_conversation_title(&conversation_data),
                last_message_date: last_updated_at.or(created_at),
                message_count,
                status: status.unwrap_or_else(|| "unknown".to_string()),
                created_at: created_at.unwrap_or(0),
                preview: Self::get_conversation_preview(&conversation_data),
            };

            conversations.push(metadata);
        }

        Ok(conversations)
    }

    fn extract_conversation_title(data: &serde_json::Value) -> String {
        // Try to get title from various possible fields
        if let Some(name) = data.get("name").and_then(|v| v.as_str()) {
            return name.to_string();
        }

        if let Some(title) = data.get("title").and_then(|v| v.as_str()) {
            return title.to_string();
        }

        // Try to extract from conversation messages
        if let Some(conversation) = data.get("conversation").and_then(|v| v.as_array()) {
            for entry in conversation.iter() {
                if let Some(text) = entry.get("text").and_then(|v| v.as_str()) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() && trimmed.len() > 10 {
                        // Take first 50 characters as title
                        return if trimmed.len() > 50 {
                            format!("{}...", &trimmed[..47])
                        } else {
                            trimmed.to_string()
                        };
                    }
                }
            }
        }

        "Untitled Conversation".to_string()
    }

    fn get_conversation_preview(data: &serde_json::Value) -> String {
        // Try to get preview from conversation messages
        if let Some(conversation) = data.get("conversation").and_then(|v| v.as_array()) {
            for entry in conversation.iter() {
                if let Some(text) = entry.get("text").and_then(|v| v.as_str()) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        // Take first 100 characters as preview
                        return if trimmed.len() > 100 {
                            format!("{}...", &trimmed[..97])
                        } else {
                            trimmed.to_string()
                        };
                    }
                }
            }
        }

        "No preview available".to_string()
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
        // Debug: print top-level keys and presence of known fields
        if cfg!(test) {
            if let Some(obj) = data.as_object() {
                let keys: Vec<&String> = obj.keys().take(12).collect();
                println!(
                    "[cursor debug] payload keys (first 12): {:?}; has conversation: {}, conversationMap: {}, fullConversationHeadersOnly: {}",
                    keys,
                    data.get("conversation").is_some(),
                    data.get("conversationMap").is_some(),
                    data.get("fullConversationHeadersOnly").is_some()
                );
            }
        }
        // Try conversation array (main format)
        if let Some(conv) = data.get("conversation").and_then(|v| v.as_array()) {
            let mut transcript = AiTranscript::new();
            for (idx, entry) in conv.iter().enumerate() {
                if cfg!(test) && idx < 3 {
                    println!(
                        "[cursor debug] conversation entry {}: {}",
                        idx,
                        serde_json::to_string_pretty(entry)
                            .unwrap_or_else(|_| "<unprintable>".to_string())
                    );
                }
                // Try different text field names
                let text_opt = entry
                    .get("text")
                    .and_then(|v| v.as_str())
                    .or_else(|| entry.get("content").and_then(|v| v.as_str()))
                    .or_else(|| entry.get("message").and_then(|v| v.as_str()))
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());

                if let Some(text) = text_opt {
                    // Heuristic: type 1 => user, type 2 => assistant (observed from data)
                    let role = entry.get("type").and_then(|v| v.as_i64()).unwrap_or(0);
                    if role == 1 {
                        transcript.add_message(Message::user(text));
                    } else {
                        transcript.add_message(Message::assistant(text));
                    }
                }

                // Additionally handle Claude-like content arrays that may include tool_use
                if let Some(content_array) = entry.get("content").and_then(|v| v.as_array()) {
                    for item in content_array {
                        match item.get("type").and_then(|v| v.as_str()) {
                            Some("text") => {
                                if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                                    let trimmed = text.trim();
                                    if !trimmed.is_empty() {
                                        // Use role heuristic again
                                        let role =
                                            entry.get("type").and_then(|v| v.as_i64()).unwrap_or(0);
                                        if role == 1 {
                                            transcript
                                                .add_message(Message::user(trimmed.to_string()));
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
                                    transcript
                                        .add_message(Message::tool_use(name.to_string(), input));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            if !transcript.messages.is_empty() {
                return Ok(Some(transcript));
            }
        }

        // Try conversationMap (newer format)
        if let Some(conv_map) = data.get("conversationMap").and_then(|v| v.as_object()) {
            let mut transcript = AiTranscript::new();
            for (key, entry) in conv_map.iter() {
                if cfg!(test) {
                    println!(
                        "[cursor debug] conversationMap entry key {}: {}",
                        key,
                        serde_json::to_string_pretty(entry)
                            .unwrap_or_else(|_| "<unprintable>".to_string())
                    );
                }
                let text_opt = entry
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                if let Some(text) = text_opt {
                    // Heuristic: type 1 => user, type 2 => assistant (observed from data)
                    let role = entry.get("type").and_then(|v| v.as_i64()).unwrap_or(0);
                    if role == 1 {
                        transcript.add_message(Message::user(text));
                    } else {
                        transcript.add_message(Message::assistant(text));
                    }
                }

                // Handle content arrays here too
                if let Some(content_array) = entry.get("content").and_then(|v| v.as_array()) {
                    for item in content_array {
                        match item.get("type").and_then(|v| v.as_str()) {
                            Some("text") => {
                                if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                                    let trimmed = text.trim();
                                    if !trimmed.is_empty() {
                                        let role =
                                            entry.get("type").and_then(|v| v.as_i64()).unwrap_or(0);
                                        if role == 1 {
                                            transcript
                                                .add_message(Message::user(trimmed.to_string()));
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
                                    transcript
                                        .add_message(Message::tool_use(name.to_string(), input));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            if !transcript.messages.is_empty() {
                return Ok(Some(transcript));
            }
        }

        // Try fullConversationHeadersOnly - these contain bubble IDs that we need to fetch
        if let Some(conv) = data
            .get("fullConversationHeadersOnly")
            .and_then(|v| v.as_array())
        {
            let mut transcript = AiTranscript::new();
            for header in conv.iter() {
                if let Some(bubble_id) = header.get("bubbleId").and_then(|v| v.as_str()) {
                    if let Ok(Some(bubble_content)) =
                        Self::fetch_bubble_content_from_db(global_db_path, composer_id, bubble_id)
                    {
                        if cfg!(test) {
                            println!(
                                "[cursor debug] bubble header: {}; content: {}",
                                serde_json::to_string_pretty(header)
                                    .unwrap_or_else(|_| "<unprintable>".to_string()),
                                serde_json::to_string_pretty(&bubble_content)
                                    .unwrap_or_else(|_| "<unprintable>".to_string())
                            );
                        }
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

                        // Try bubble content arrays for tool_use
                        if let Some(content_array) =
                            bubble_content.get("content").and_then(|v| v.as_array())
                        {
                            for item in content_array {
                                match item.get("type").and_then(|v| v.as_str()) {
                                    Some("text") => {
                                        if let Some(text) =
                                            item.get("text").and_then(|v| v.as_str())
                                        {
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
                return Ok(Some(transcript));
            }
        }

        // Try root-level text field
        if let Some(text) = data.get("text").and_then(|v| v.as_str()) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                let mut transcript = AiTranscript::new();
                transcript.add_message(Message::user(trimmed.to_string()));
                return Ok(Some(transcript));
            }
        }

        // Try root-level richText field
        if let Some(rich_text) = data.get("richText").and_then(|v| v.as_str()) {
            let trimmed = rich_text.trim();
            if !trimmed.is_empty() {
                let mut transcript = AiTranscript::new();
                transcript.add_message(Message::user(trimmed.to_string()));
                return Ok(Some(transcript));
            }
        }

        Ok(None)
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
}

impl AgentCheckpointPreset for CursorPreset {
    fn run(&self, flags: AgentCheckpointFlags) -> Result<AgentRunResult, GitAiError> {
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

        // If explicit composer ID provided, use it
        if let Some(composer_id) = flags.prompt_id {
            let payload = Self::fetch_composer_payload(&global_db, &composer_id)?;
            let transcript =
                Self::transcript_from_composer_payload(&payload, &global_db, &composer_id)?
                    .ok_or_else(|| {
                        GitAiError::PresetError(
                            "Could not extract transcript from Cursor composer".to_string(),
                        )
                    })?;

            let agent_id = AgentId {
                tool: "cursor".to_string(),
                id: composer_id,
            };

            return Ok(AgentRunResult {
                agent_id,
                transcript,
            });
        }

        // Get the 5 latest conversations
        let conversations = Self::get_latest_conversations(&global_db, Some(5))?;

        if conversations.is_empty() {
            return Err(GitAiError::PresetError(
                "No Cursor conversations found".to_string(),
            ));
        }

        // Use the first (most recent) conversation
        let latest_conversation = &conversations[0];
        let payload = Self::fetch_composer_payload(&global_db, &latest_conversation.composer_id)?;
        let transcript = Self::transcript_from_composer_payload(
            &payload,
            &global_db,
            &latest_conversation.composer_id,
        )?
        .ok_or_else(|| {
            GitAiError::PresetError("Could not extract transcript from Cursor composer".to_string())
        })?;

        let agent_id = AgentId {
            tool: "cursor".to_string(),
            id: latest_conversation.composer_id.clone(),
        };

        Ok(AgentRunResult {
            agent_id,
            transcript,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_preset_prints_transcript() {
        let preset = CursorPreset;
        let result = preset.run(AgentCheckpointFlags {
            prompt_id: None,
            hook_input: None,
        });

        match result {
            Ok(run) => {
                println!("Cursor Agent: {}:{}", run.agent_id.tool, run.agent_id.id);
                for (i, m) in run.transcript.messages.iter().enumerate() {
                    match m {
                        Message::User { text } => println!("User {}: {}", i, text),
                        Message::Assistant { text } => println!("Assistant {}: {}", i, text),
                        Message::ToolUse { name, input } => {
                            println!("ToolUse {}: {} {}", i, name, input)
                        }
                    }
                }
            }
            Err(e) => {
                // It's fine if Cursor isn't installed; print the error for visibility
                println!("CursorPreset error: {}", e);
            }
        }
    }
}
