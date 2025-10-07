use crate::{
    authorship::{
        transcript::{AiTranscript, Message},
        working_log::AgentId,
    },
    error::GitAiError,
};
use chrono::{TimeZone, Utc};
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
        checkpoints: &mut [crate::authorship::working_log::Checkpoint],
    ) -> Result<(), GitAiError> {
        use std::collections::HashMap;

        // Group checkpoints by Cursor conversation ID
        let mut cursor_conversations: HashMap<
            String,
            Vec<&mut crate::authorship::working_log::Checkpoint>,
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

pub struct GithubCopilotPreset;

impl AgentCheckpointPreset for GithubCopilotPreset {
    fn run(&self, flags: AgentCheckpointFlags) -> Result<AgentRunResult, GitAiError> {
        // Parse hook_input JSON to extract chat session information
        let hook_input_json = flags.hook_input.ok_or_else(|| {
            GitAiError::PresetError("hook_input is required for GitHub Copilot preset".to_string())
        })?;

        let hook_data: serde_json::Value = serde_json::from_str(&hook_input_json)
            .map_err(|e| GitAiError::PresetError(format!("Invalid JSON in hook_input: {}", e)))?;

        let chat_session_path = hook_data
            .get("chatSessionPath")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                GitAiError::PresetError("chatSessionPath not found in hook_input".to_string())
            })?;

        // Accept either chatSessionId (old) or sessionId (from VS Code extension)
        let chat_session_id = hook_data
            .get("chatSessionId")
            .and_then(|v| v.as_str())
            .or_else(|| hook_data.get("sessionId").and_then(|v| v.as_str()))
            .unwrap_or("unknown")
            .to_string();

        // Read the Copilot chat session JSON
        let session_content =
            std::fs::read_to_string(chat_session_path).map_err(|e| GitAiError::IoError(e))?;
        // Required working directory provided by the extension
        let repo_working_dir: String = hook_data
            .get("workspaceFolder")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                GitAiError::PresetError(
                    "workspaceFolder not found in hook_input for GitHub Copilot preset".to_string(),
                )
            })?
            .to_string();

        // Build transcript and model via helper
        let (transcript, detected_model) =
            GithubCopilotPreset::transcript_and_model_from_copilot_session_json(&session_content)?;

        let agent_id = AgentId {
            tool: "github-copilot".to_string(),
            id: chat_session_id,
            model: detected_model.unwrap_or_else(|| "unknown".to_string()),
        };

        Ok(AgentRunResult {
            agent_id,
            is_human: false,
            transcript: Some(transcript),
            repo_working_dir: Some(repo_working_dir),
        })
    }
}

impl GithubCopilotPreset {
    /// Translate a GitHub Copilot chat session JSON string into an AiTranscript and optional model.
    fn transcript_and_model_from_copilot_session_json(
        session_json_str: &str,
    ) -> Result<(AiTranscript, Option<String>), GitAiError> {
        let session_json: serde_json::Value =
            serde_json::from_str(session_json_str).map_err(|e| GitAiError::JsonError(e))?;

        // Extract the requests array which represents the conversation from start to finish
        let requests = session_json
            .get("requests")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                GitAiError::PresetError(
                    "requests array not found in Copilot chat session".to_string(),
                )
            })?;

        let mut transcript = AiTranscript::new();
        let mut detected_model: Option<String> = None;

        for request in requests {
            // Parse the human timestamp once per request (unix ms and RFC3339)
            let user_ts_ms = request.get("timestamp").and_then(|v| v.as_i64());
            let user_ts_rfc3339 = user_ts_ms
                .and_then(|ms| Utc.timestamp_millis_opt(ms).single().map(|dt| dt.to_rfc3339()));

            // Add the human's message
            if let Some(user_text) = request
                .get("message")
                .and_then(|m| m.get("text"))
                .and_then(|v| v.as_str())
            {
                let trimmed = user_text.trim();
                if !trimmed.is_empty() {
                    transcript.add_message(Message::User {
                        text: trimmed.to_string(),
                        timestamp: user_ts_rfc3339.clone(),
                    });
                }
            }

            // Process the agent's response items: tool invocations, edits, and text
            if let Some(response_items) = request.get("response").and_then(|v| v.as_array()) {
                let mut assistant_text_accumulator = String::new();

                for item in response_items {
                    // Capture tool invocations and other structured actions as tool_use
                    if let Some(kind) = item.get("kind").and_then(|v| v.as_str()) {
                        match kind {
                            // Primary tool invocation entries
                            "toolInvocationSerialized" => {
                                let tool_name = item
                                    .get("toolId")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("tool");

                                // Normalize invocationMessage to a string
                                let inv_msg = item.get("invocationMessage").and_then(|im| {
                                    if let Some(s) = im.as_str() {
                                        Some(s.to_string())
                                    } else if im.is_object() {
                                        im.get("value")
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string())
                                    } else {
                                        None
                                    }
                                });

                                if let Some(msg) = inv_msg {
                                    transcript.add_message(Message::tool_use(
                                        tool_name.to_string(),
                                        serde_json::Value::String(msg),
                                    ));
                                }
                            }
                            // Other structured response elements worth capturing
                            "textEditGroup" | "prepareToolInvocation" => {
                                transcript
                                    .add_message(Message::tool_use(kind.to_string(), item.clone()));
                            }
                            // codeblockUri should contribute a visible mention like @path, not a tool_use
                            "codeblockUri" => {
                                let path_opt = item
                                    .get("uri")
                                    .and_then(|u| {
                                        u.get("fsPath")
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string())
                                            .or_else(|| {
                                                u.get("path")
                                                    .and_then(|v| v.as_str())
                                                    .map(|s| s.to_string())
                                            })
                                    })
                                    .or_else(|| {
                                        item.get("fsPath")
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string())
                                    })
                                    .or_else(|| {
                                        item.get("path")
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string())
                                    });
                                if let Some(p) = path_opt {
                                    let mention = format!("@{}", p);
                                    if !assistant_text_accumulator.is_empty() {
                                        assistant_text_accumulator.push(' ');
                                    }
                                    assistant_text_accumulator.push_str(&mention);
                                }
                            }
                            // inlineReference should contribute a visible mention like @path, not a tool_use
                            "inlineReference" => {
                                let path_opt = item.get("inlineReference").and_then(|ir| {
                                    // Try nested uri.fsPath or uri.path
                                    ir.get("uri")
                                        .and_then(|u| u.get("fsPath"))
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string())
                                        .or_else(|| {
                                            ir.get("uri")
                                                .and_then(|u| u.get("path"))
                                                .and_then(|v| v.as_str())
                                                .map(|s| s.to_string())
                                        })
                                        // Or top-level fsPath / path on inlineReference
                                        .or_else(|| {
                                            ir.get("fsPath")
                                                .and_then(|v| v.as_str())
                                                .map(|s| s.to_string())
                                        })
                                        .or_else(|| {
                                            ir.get("path")
                                                .and_then(|v| v.as_str())
                                                .map(|s| s.to_string())
                                        })
                                });
                                if let Some(p) = path_opt {
                                    let mention = format!("@{}", p);
                                    if !assistant_text_accumulator.is_empty() {
                                        assistant_text_accumulator.push(' ');
                                    }
                                    assistant_text_accumulator.push_str(&mention);
                                }
                            }
                            _ => {}
                        }
                    }

                    // Accumulate visible assistant text snippets
                    if let Some(val) = item.get("value").and_then(|v| v.as_str()) {
                        let t = val.trim();
                        if !t.is_empty() {
                            if !assistant_text_accumulator.is_empty() {
                                assistant_text_accumulator.push(' ');
                            }
                            assistant_text_accumulator.push_str(t);
                        }
                    }
                }

                if !assistant_text_accumulator.trim().is_empty() {
                    // Set assistant timestamp to user_ts + totalElapsed if available
                    let assistant_ts = request
                        .get("result")
                        .and_then(|r| r.get("timings"))
                        .and_then(|t| t.get("totalElapsed"))
                        .and_then(|v| v.as_i64())
                        .and_then(|elapsed| user_ts_ms.map(|ums| ums + elapsed))
                        .and_then(|ms| {
                            Utc.timestamp_millis_opt(ms)
                                .single()
                                .map(|dt| dt.to_rfc3339())
                        });

                    transcript.add_message(Message::Assistant {
                        text: assistant_text_accumulator.trim().to_string(),
                        timestamp: assistant_ts,
                    });
                }
            }

            // Detect model from request metadata if not yet set (uses first modelId seen)
            if detected_model.is_none() {
                if let Some(model_id) = request.get("modelId").and_then(|v| v.as_str()) {
                    detected_model = Some(model_id.to_string());
                }
            }
        }

        Ok((transcript, detected_model))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copilot_session_parsing_stub() {
        // Minimal valid shape with empty requests
        let sample = r#"{"requests": []}"#;
        let result = GithubCopilotPreset::transcript_and_model_from_copilot_session_json(sample);
        assert!(result.is_ok());
        let (tx, model) = result.unwrap();
        assert!(tx.messages.is_empty());
        assert!(model.is_none());
    }

    #[test]
    fn copilot_session_parsing_simple() {
        let sample = r#"{
  "version": 3,
  "requesterUsername": "svarlamov",
  "requesterAvatarIconUri": {
    "$mid": 1,
    "path": "/u/10859025",
    "scheme": "https",
    "authority": "avatars.githubusercontent.com",
    "query": "v=4"
  },
  "responderUsername": "GitHub Copilot",
  "responderAvatarIconUri": {
    "id": "copilot"
  },
  "initialLocation": "panel",
  "requests": [
    {
      "requestId": "request_5db7fb17-1726-48b4-b737-c0cb4c97caa8",
      "message": {
        "parts": [
          {
            "range": {
              "start": 0,
              "endExclusive": 26
            },
            "editorRange": {
              "startLineNumber": 1,
              "startColumn": 1,
              "endLineNumber": 1,
              "endColumn": 27
            },
            "text": "What can you help me with?",
            "kind": "text"
          }
        ],
        "text": "What can you help me with?"
      },
      "variableData": {
        "variables": [
          {
            "kind": "file",
            "value": {
              "$mid": 1,
              "fsPath": "/Users/svarlamov/projects/testing-git/index.ts",
              "external": "file:///Users/svarlamov/projects/testing-git/index.ts",
              "path": "/Users/svarlamov/projects/testing-git/index.ts",
              "scheme": "file"
            },
            "id": "file:///Users/svarlamov/projects/testing-git/index.ts",
            "name": "index.ts"
          }
        ]
      },
      "response": [
        {
          "value": "I'm GitHub Copilot, and I can help you with a wide range of programming and development tasks! Based on your current workspace, I can see you have a TypeScript project with an ",
          "supportThemeIcons": false,
          "supportHtml": false,
          "baseUri": {
            "$mid": 1,
            "path": "/Users/svarlamov/projects/testing-git/",
            "scheme": "file"
          }
        },
        {
          "kind": "inlineReference",
          "inlineReference": {
            "$mid": 1,
            "fsPath": "/Users/svarlamov/projects/testing-git/index.ts",
            "external": "file:///Users/svarlamov/projects/testing-git/index.ts",
            "path": "/Users/svarlamov/projects/testing-git/index.ts",
            "scheme": "file"
          }
        },
        {
          "value": " file that prints \"Bonjour\" and numbers 1-10.\n\nHere are some ways I can assist you:\n\n## Code Development\n- **Write, edit, and refactor code** in various languages (TypeScript, JavaScript, Python, Java, C#, Go, Rust, and many more)\n- **Debug and fix issues** in your existing code\n- **Add new features** or modify existing functionality\n- **Optimize performance** and improve code quality\n\n## Project Management\n- **Set up new projects** and workspaces from scratch\n- **Configure build systems** and development environments\n- **Manage dependencies** and package installations\n- **Create and modify configuration files**\n\n## Git and Version Control\n- **Commit changes** and manage your Git workflow\n- **Create branches** and handle merges\n- **View git history** and resolve conflicts\n- **Work with pull requests** and issues\n\n## Development Tools\n- **Run terminal commands** and scripts\n- **Install and configure VS Code extensions**\n- **Set up debugging configurations**\n- **Work with databases** (PostgreSQL support available)\n\n## Documentation and Testing\n- **Write documentation** and README files\n- **Create and run tests**\n- **Generate code comments** and API documentation\n\nFor your current TypeScript project, I could help you:\n- Add error handling and input validation\n- Convert it to a proper CLI tool with argument parsing\n- Add TypeScript configurations and build scripts\n- Set up testing frameworks\n- Add more sophisticated functionality\n\nWhat specific task would you like help with today?",
          "supportThemeIcons": false,
          "supportHtml": false,
          "baseUri": {
            "$mid": 1,
            "path": "/Users/svarlamov/projects/testing-git/",
            "scheme": "file"
          }
        }
      ],
      "responseId": "response_f31b8287-fb6d-4c9b-87c9-9a40412097b8",
      "result": {
        "timings": {
          "firstProgress": 4581,
          "totalElapsed": 12361
        },
        "metadata": {
          "codeBlocks": [],
          "renderedUserMessage": [
            {
              "type": 1,
              "text": "<attachments>\n<attachment id=\"index.ts\" filePath=\"/Users/svarlamov/projects/testing-git/index.ts\">\nconsole.log('Bonjour');\n\nconsole.log('CLI initialized - starting number print sequence');\n\n// Print numbers 1 through 10\nfor (let i = 1; i <= 10; i++) {\n\tconsole.log(i);\n}\n\nconsole.log('Bonjour World');\n</attachment>\n\n</attachments>\n<context>\nThe current date is October 7, 2025.\nNo terminals found.\n</context>\n<editorContext>\nThe user's current file is /Users/svarlamov/projects/testing-git/index.ts. \n</editorContext>\n<reminderInstructions>\nWhen using the replace_string_in_file tool, include 3-5 lines of unchanged code before and after the string you want to replace, to make it unambiguous which part of the file should be edited.\n\n</reminderInstructions>\n<userRequest>\nWhat can you help me with?\n</userRequest>\n"
            },
            {
              "type": 3,
              "cacheType": "ephemeral"
            }
          ],
          "renderedGlobalContext": [
            {
              "type": 1,
              "text": "<environment_info>\nThe user's current OS is: macOS\nThe user's default shell is: \"zsh\". When you generate terminal commands, please generate them correctly for this shell.\n</environment_info>\n<workspace_info>\nI am working in a workspace with the following folders:\n- /Users/svarlamov/projects/testing-git \nI am working in a workspace that has the following structure:\n```\nindex.ts\nREADME.md\n```\nThis is the state of the context at this point in the conversation. The view of the workspace structure may be truncated. You can use tools to collect more context if needed.\n</workspace_info>\n"
            },
            {
              "type": 3,
              "cacheType": "ephemeral"
            }
          ],
          "toolCallRounds": [
            {
              "response": "I'm GitHub Copilot, and I can help you with a wide range of programming and development tasks! Based on your current workspace, I can see you have a TypeScript project with an `index.ts` file that prints \"Bonjour\" and numbers 1-10.\n\nHere are some ways I can assist you:\n\n## Code Development\n- **Write, edit, and refactor code** in various languages (TypeScript, JavaScript, Python, Java, C#, Go, Rust, and many more)\n- **Debug and fix issues** in your existing code\n- **Add new features** or modify existing functionality\n- **Optimize performance** and improve code quality\n\n## Project Management\n- **Set up new projects** and workspaces from scratch\n- **Configure build systems** and development environments\n- **Manage dependencies** and package installations\n- **Create and modify configuration files**\n\n## Git and Version Control\n- **Commit changes** and manage your Git workflow\n- **Create branches** and handle merges\n- **View git history** and resolve conflicts\n- **Work with pull requests** and issues\n\n## Development Tools\n- **Run terminal commands** and scripts\n- **Install and configure VS Code extensions**\n- **Set up debugging configurations**\n- **Work with databases** (PostgreSQL support available)\n\n## Documentation and Testing\n- **Write documentation** and README files\n- **Create and run tests**\n- **Generate code comments** and API documentation\n\nFor your current TypeScript project, I could help you:\n- Add error handling and input validation\n- Convert it to a proper CLI tool with argument parsing\n- Add TypeScript configurations and build scripts\n- Set up testing frameworks\n- Add more sophisticated functionality\n\nWhat specific task would you like help with today?",
              "toolCalls": [],
              "toolInputRetry": 0,
              "id": "0b7274b9-b8c5-409f-9648-f7a23404c4c4"
            }
          ],
          "modelMessageId": "93bc6c88-e5f4-4a76-a115-7d88e44dbaf8",
          "responseId": "1f6f164a-cfcd-4aa6-b131-d60d0f300b7b",
          "sessionId": "29299837-0260-4fa8-8073-f02755e9dbcc",
          "agentId": "github.copilot.editsAgent"
        },
        "details": "Claude Sonnet 4 • 1x"
      },
      "responseMarkdownInfo": [],
      "followups": [],
      "isCanceled": false,
      "agent": {
        "extensionId": {
          "value": "GitHub.copilot-chat",
          "_lower": "github.copilot-chat"
        },
        "extensionVersion": "0.31.5",
        "publisherDisplayName": "GitHub",
        "extensionPublisherId": "GitHub",
        "extensionDisplayName": "GitHub Copilot Chat",
        "id": "github.copilot.editsAgent",
        "description": "Edit files in your workspace in agent mode",
        "when": "config.chat.agent.enabled",
        "metadata": {
          "themeIcon": {
            "id": "tools"
          },
          "hasFollowups": false,
          "supportIssueReporting": false
        },
        "name": "agent",
        "fullName": "GitHub Copilot",
        "isDefault": true,
        "locations": [
          "panel"
        ],
        "modes": [
          "agent"
        ],
        "slashCommands": [
          {
            "name": "list"
          }
        ],
        "disambiguation": []
      },
      "contentReferences": [],
      "codeCitations": [],
      "timestamp": 1759845073835,
      "modelId": "copilot/claude-sonnet-4"
    },
    {
      "requestId": "request_e838f1ef-f292-49ab-92fc-3317408b67f1",
      "message": {
        "parts": [
          {
            "range": {
              "start": 0,
              "endExclusive": 35
            },
            "editorRange": {
              "startLineNumber": 1,
              "startColumn": 1,
              "endLineNumber": 1,
              "endColumn": 36
            },
            "text": "Change Bonjour World to hello world",
            "kind": "text"
          }
        ],
        "text": "Change Bonjour World to hello world"
      },
      "variableData": {
        "variables": [
          {
            "kind": "file",
            "id": "vscode.implicit.selection",
            "name": "file:index.ts",
            "value": {
              "uri": {
                "$mid": 1,
                "fsPath": "/Users/svarlamov/projects/testing-git/index.ts",
                "external": "file:///Users/svarlamov/projects/testing-git/index.ts",
                "path": "/Users/svarlamov/projects/testing-git/index.ts",
                "scheme": "file"
              },
              "range": {
                "startLineNumber": 10,
                "startColumn": 14,
                "endLineNumber": 10,
                "endColumn": 27,
                "selectionStartLineNumber": 10,
                "selectionStartColumn": 14,
                "positionLineNumber": 10,
                "positionColumn": 27
              }
            },
            "modelDescription": "User's active selection"
          }
        ]
      },
      "response": [
        {
          "value": "I'll change \"Bonjour World\" to \"hello world\" in your selected text. ",
          "supportThemeIcons": false,
          "supportHtml": false,
          "baseUri": {
            "$mid": 1,
            "path": "/Users/svarlamov/projects/testing-git/",
            "scheme": "file"
          }
        },
        {
          "kind": "prepareToolInvocation",
          "toolName": "copilot_replaceString"
        },
        {
          "kind": "toolInvocationSerialized",
          "presentation": "hidden",
          "invocationMessage": "Using \"Replace String in File\"",
          "isConfirmed": {
            "type": 1
          },
          "isComplete": true,
          "source": {
            "type": "internal",
            "label": "Built-In"
          },
          "toolCallId": "1894ea97-5e8b-4b1b-9a18-c20f05839043",
          "toolId": "copilot_replaceString"
        },
        {
          "value": "\n```\n",
          "supportThemeIcons": false,
          "supportHtml": false,
          "uris": {}
        },
        {
          "id": "041b621f-13ae-440a-bcdc-739c9bfe00b1",
          "kind": "undoStop"
        },
        {
          "kind": "codeblockUri",
          "uri": {
            "$mid": 1,
            "fsPath": "/Users/svarlamov/projects/testing-git/index.ts",
            "external": "file:///Users/svarlamov/projects/testing-git/index.ts",
            "path": "/Users/svarlamov/projects/testing-git/index.ts",
            "scheme": "file"
          },
          "isEdit": true
        },
        {
          "kind": "textEditGroup",
          "uri": {
            "$mid": 1,
            "fsPath": "/Users/svarlamov/projects/testing-git/index.ts",
            "external": "file:///Users/svarlamov/projects/testing-git/index.ts",
            "path": "/Users/svarlamov/projects/testing-git/index.ts",
            "scheme": "file"
          },
          "edits": [
            [
              {
                "text": "// Print numbers 1 through 10\nfor (let i = 1; i <= 10; i++) {\n\tconsole.log(i);\n}\n\nconsole.log('hello world');",
                "range": {
                  "startLineNumber": 5,
                  "startColumn": 1,
                  "endLineNumber": 10,
                  "endColumn": 30
                }
              }
            ],
            []
          ],
          "done": true
        },
        {
          "value": "\n```\n",
          "supportThemeIcons": false,
          "supportHtml": false,
          "uris": {}
        },
        {
          "value": "Done! I've changed \"Bonjour World\" to \"hello world\" in line 10 of your ",
          "supportThemeIcons": false,
          "supportHtml": false,
          "baseUri": {
            "$mid": 1,
            "path": "/Users/svarlamov/projects/testing-git/",
            "scheme": "file"
          }
        },
        {
          "kind": "inlineReference",
          "inlineReference": {
            "$mid": 1,
            "fsPath": "/Users/svarlamov/projects/testing-git/index.ts",
            "external": "file:///Users/svarlamov/projects/testing-git/index.ts",
            "path": "/Users/svarlamov/projects/testing-git/index.ts",
            "scheme": "file"
          }
        },
        {
          "value": " file.",
          "supportThemeIcons": false,
          "supportHtml": false,
          "baseUri": {
            "$mid": 1,
            "path": "/Users/svarlamov/projects/testing-git/",
            "scheme": "file"
          }
        }
      ],
      "responseId": "response_409401ac-4621-4115-b392-593e2145b3c5",
      "result": {
        "timings": {
          "firstProgress": 3009,
          "totalElapsed": 9509
        },
        "metadata": {
          "codeBlocks": [],
          "renderedUserMessage": [
            {
              "type": 1,
              "text": "<attachments>\n<attachment id=\"file:index.ts\">\nUser's active selection, this should be the main focus:\nExcerpt from index.ts, lines 10 to 10:\n```typescript\nBonjour World\n```\n</attachment>\n<attachment filePath=\"/Users/svarlamov/projects/testing-git/index.ts\">\nUser's active file for additional context:\nconsole.log('Bonjour');\n\nconsole.log('CLI initialized - starting number print sequence');\n\n// Print numbers 1 through 10\nfor (let i = 1; i <= 10; i++) {\n\tconsole.log(i);\n}\n\nconsole.log('Bonjour World');\n</attachment>\n\n</attachments>\n<context>\nThe current date is October 7, 2025.\nNo terminals found.\n</context>\n<editorContext>\nThe user's current file is /Users/svarlamov/projects/testing-git/index.ts. The current selection is from line 10 to line 10.\n</editorContext>\n<reminderInstructions>\nWhen using the replace_string_in_file tool, include 3-5 lines of unchanged code before and after the string you want to replace, to make it unambiguous which part of the file should be edited.\n\n</reminderInstructions>\n<userRequest>\nChange Bonjour World to hello world\n</userRequest>\n"
            },
            {
              "type": 3,
              "cacheType": "ephemeral"
            }
          ],
          "toolCallRounds": [
            {
              "response": "I'll change \"Bonjour World\" to \"hello world\" in your selected text. ",
              "toolCalls": [
                {
                  "name": "replace_string_in_file",
                  "arguments": "{\"filePath\": \"/Users/svarlamov/projects/testing-git/index.ts\", \"oldString\": \"// Print numbers 1 through 10\\nfor (let i = 1; i <= 10; i++) {\\n\\tconsole.log(i);\\n}\\n\\nconsole.log('Bonjour World');\", \"newString\": \"// Print numbers 1 through 10\\nfor (let i = 1; i <= 10; i++) {\\n\\tconsole.log(i);\\n}\\n\\nconsole.log('hello world');\"}",
                  "id": "toolu_vrtx_01UAL9wRmnDUwNjStDn4qhne__vscode-1759845028970"
                }
              ],
              "toolInputRetry": 0,
              "id": "e4e2c5f6-0c40-46ae-8c38-a17a61177986"
            },
            {
              "response": "Done! I've changed \"Bonjour World\" to \"hello world\" in line 10 of your `index.ts` file.",
              "toolCalls": [],
              "toolInputRetry": 0,
              "id": "ccf0e77f-a3a7-4da1-a2f5-c30f724198b0"
            }
          ],
          "toolCallResults": {
            "toolu_vrtx_01UAL9wRmnDUwNjStDn4qhne__vscode-1759845028970": {
              "$mid": 20,
              "content": [
                {
                  "$mid": 23,
                  "value": {
                    "node": {
                      "type": 1,
                      "ctor": 2,
                      "ctorName": "qy",
                      "children": [
                        {
                          "type": 2,
                          "priority": 9007199254740991,
                          "text": "The following files were successfully edited:",
                          "lineBreakBefore": false
                        },
                        {
                          "type": 2,
                          "priority": 9007199254740991,
                          "text": "\n",
                          "lineBreakBefore": false
                        },
                        {
                          "type": 2,
                          "priority": 9007199254740991,
                          "text": "/Users/svarlamov/projects/testing-git/index.ts",
                          "lineBreakBefore": false
                        },
                        {
                          "type": 2,
                          "priority": 9007199254740991,
                          "text": "\n",
                          "lineBreakBefore": false
                        }
                      ],
                      "props": {},
                      "references": []
                    }
                  }
                }
              ]
            }
          },
          "modelMessageId": "876b165b-2504-454b-b96c-58bf77d34443",
          "responseId": "032efa19-4a6b-4de5-80d6-3edcbb56dc0f",
          "sessionId": "29299837-0260-4fa8-8073-f02755e9dbcc",
          "agentId": "github.copilot.editsAgent"
        },
        "details": "Claude Sonnet 4 • 1x"
      },
      "responseMarkdownInfo": [],
      "followups": [],
      "isCanceled": false,
      "agent": {
        "extensionId": {
          "value": "GitHub.copilot-chat",
          "_lower": "github.copilot-chat"
        },
        "extensionVersion": "0.31.5",
        "publisherDisplayName": "GitHub",
        "extensionPublisherId": "GitHub",
        "extensionDisplayName": "GitHub Copilot Chat",
        "id": "github.copilot.editsAgent",
        "description": "Edit files in your workspace in agent mode",
        "when": "config.chat.agent.enabled",
        "metadata": {
          "themeIcon": {
            "id": "tools"
          },
          "hasFollowups": false,
          "supportIssueReporting": false
        },
        "name": "agent",
        "fullName": "GitHub Copilot",
        "isDefault": true,
        "locations": [
          "panel"
        ],
        "modes": [
          "agent"
        ],
        "slashCommands": [
          {
            "name": "list"
          }
        ],
        "disambiguation": []
      },
      "contentReferences": [],
      "codeCitations": [],
      "timestamp": 1759845101282,
      "modelId": "copilot/claude-sonnet-4"
    },
    {
      "requestId": "request_6a383028-ec48-40b3-b20c-bb91b6612fa1",
      "message": {
        "parts": [
          {
            "range": {
              "start": 0,
              "endExclusive": 40
            },
            "editorRange": {
              "startLineNumber": 1,
              "startColumn": 1,
              "endLineNumber": 1,
              "endColumn": 41
            },
            "text": "Search for any other mentions of Bonjour",
            "kind": "text"
          }
        ],
        "text": "Search for any other mentions of Bonjour"
      },
      "variableData": {
        "variables": [
          {
            "kind": "file",
            "id": "vscode.implicit.selection",
            "name": "file:index.ts",
            "value": {
              "uri": {
                "$mid": 1,
                "fsPath": "/Users/svarlamov/projects/testing-git/index.ts",
                "external": "file:///Users/svarlamov/projects/testing-git/index.ts",
                "path": "/Users/svarlamov/projects/testing-git/index.ts",
                "scheme": "file"
              },
              "range": {
                "startLineNumber": 10,
                "startColumn": 14,
                "endLineNumber": 10,
                "endColumn": 25,
                "selectionStartLineNumber": 10,
                "selectionStartColumn": 14,
                "positionLineNumber": 10,
                "positionColumn": 25
              }
            },
            "modelDescription": "User's active selection"
          }
        ]
      },
      "response": [
        {
          "value": "I'll search for any other mentions of \"Bonjour\" in your workspace. ",
          "supportThemeIcons": false,
          "supportHtml": false,
          "baseUri": {
            "$mid": 1,
            "path": "/Users/svarlamov/projects/testing-git/",
            "scheme": "file"
          }
        },
        {
          "kind": "prepareToolInvocation",
          "toolName": "copilot_findTextInFiles"
        },
        {
          "kind": "toolInvocationSerialized",
          "invocationMessage": {
            "value": "Searching text for `Bonjour`",
            "supportThemeIcons": false,
            "supportHtml": false,
            "uris": {}
          },
          "pastTenseMessage": {
            "value": "Searched text for `Bonjour`, 1 result",
            "supportThemeIcons": false,
            "supportHtml": false,
            "uris": {}
          },
          "isConfirmed": {
            "type": 1
          },
          "isComplete": true,
          "source": {
            "type": "internal",
            "label": "Built-In"
          },
          "resultDetails": [
            {
              "uri": {
                "$mid": 1,
                "fsPath": "/Users/svarlamov/projects/testing-git/index.ts",
                "external": "file:///Users/svarlamov/projects/testing-git/index.ts",
                "path": "/Users/svarlamov/projects/testing-git/index.ts",
                "scheme": "file"
              },
              "range": {
                "startLineNumber": 1,
                "startColumn": 14,
                "endLineNumber": 1,
                "endColumn": 21
              }
            }
          ],
          "toolCallId": "e695b61d-b1ce-406a-a339-5d3402378544",
          "toolId": "copilot_findTextInFiles"
        },
        {
          "value": "I found one more mention of \"Bonjour\" in your code! It's on line 1 of ",
          "supportThemeIcons": false,
          "supportHtml": false,
          "baseUri": {
            "$mid": 1,
            "path": "/Users/svarlamov/projects/testing-git/",
            "scheme": "file"
          }
        },
        {
          "kind": "inlineReference",
          "inlineReference": {
            "$mid": 1,
            "fsPath": "/Users/svarlamov/projects/testing-git/index.ts",
            "external": "file:///Users/svarlamov/projects/testing-git/index.ts",
            "path": "/Users/svarlamov/projects/testing-git/index.ts",
            "scheme": "file"
          }
        },
        {
          "value": ":\n\n```typescript\nconsole.log('Bonjour');\n```\n\nWould you like me to change this one to \"hello\" as well to make it consistent?",
          "supportThemeIcons": false,
          "supportHtml": false,
          "baseUri": {
            "$mid": 1,
            "path": "/Users/svarlamov/projects/testing-git/",
            "scheme": "file"
          }
        }
      ],
      "responseId": "response_69f6b834-8692-498d-a500-b2cda8dd802e",
      "result": {
        "timings": {
          "firstProgress": 4636,
          "totalElapsed": 8787
        },
        "metadata": {
          "codeBlocks": [
            {
              "code": "console.log('Bonjour');\n",
              "language": "typescript",
              "markdownBeforeBlock": "I found one more mention of \"Bonjour\" in your code! It's on line 1 of :\n\n"
            }
          ],
          "renderedUserMessage": [
            {
              "type": 1,
              "text": "<attachments>\n<attachment id=\"file:index.ts\">\nUser's active selection, this should be the main focus:\nExcerpt from index.ts, lines 10 to 10:\n```typescript\nhello world\n```\n</attachment>\n<attachment filePath=\"/Users/svarlamov/projects/testing-git/index.ts\">\nUser's active file for additional context:\nconsole.log('Bonjour');\n\nconsole.log('CLI initialized - starting number print sequence');\n\n// Print numbers 1 through 10\nfor (let i = 1; i <= 10; i++) {\n\tconsole.log(i);\n}\n\nconsole.log('hello world');\n</attachment>\n\n</attachments>\n<context>\nThe current date is October 7, 2025.\nNo terminals found.\n</context>\n<editorContext>\nThe user's current file is /Users/svarlamov/projects/testing-git/index.ts. The current selection is from line 10 to line 10.\n</editorContext>\n<reminderInstructions>\nWhen using the replace_string_in_file tool, include 3-5 lines of unchanged code before and after the string you want to replace, to make it unambiguous which part of the file should be edited.\n\n</reminderInstructions>\n<userRequest>\nSearch for any other mentions of Bonjour\n</userRequest>\n"
            },
            {
              "type": 3,
              "cacheType": "ephemeral"
            }
          ],
          "toolCallRounds": [
            {
              "response": "I'll search for any other mentions of \"Bonjour\" in your workspace. ",
              "toolCalls": [
                {
                  "name": "grep_search",
                  "arguments": "{\"query\": \"Bonjour\", \"isRegexp\": false}",
                  "id": "toolu_vrtx_01QZ8eYAhaNiLjhip28kHbk7__vscode-1759845028971"
                }
              ],
              "toolInputRetry": 0,
              "id": "6bdce891-804b-449b-941c-e6ae1ca48602"
            },
            {
              "response": "I found one more mention of \"Bonjour\" in your code! It's on line 1 of `index.ts`:\n\n```typescript\nconsole.log('Bonjour');\n```\n\nWould you like me to change this one to \"hello\" as well to make it consistent?",
              "toolCalls": [],
              "toolInputRetry": 0,
              "id": "294f9cab-0c59-4190-ae17-96daf68e46d1"
            }
          ],
          "toolCallResults": {
            "toolu_vrtx_01QZ8eYAhaNiLjhip28kHbk7__vscode-1759845028971": {
              "$mid": 20,
              "content": [
                {
                  "$mid": 23,
                  "value": {
                    "node": {
                      "type": 1,
                      "ctor": 2,
                      "ctorName": "_rt",
                      "children": [
                        {
                          "type": 1,
                          "ctor": 2,
                          "ctorName": "Oue",
                          "children": [
                            {
                              "type": 2,
                              "priority": 20,
                              "text": "1 match",
                              "references": [],
                              "lineBreakBefore": true
                            }
                          ],
                          "props": {
                            "priority": 20
                          },
                          "references": []
                        },
                        {
                          "type": 1,
                          "ctor": 2,
                          "ctorName": "Coe",
                          "children": [
                            {
                              "type": 1,
                              "ctor": 2,
                              "ctorName": "t",
                              "children": [
                                {
                                  "type": 1,
                                  "ctor": 2,
                                  "ctorName": "",
                                  "children": [
                                    {
                                      "type": 2,
                                      "priority": 9007199254740991,
                                      "text": "<match path=\"/Users/svarlamov/projects/testing-git/index.ts\" line=1>\n",
                                      "lineBreakBefore": false
                                    }
                                  ],
                                  "props": {},
                                  "references": [],
                                  "keepWithId": 61
                                },
                                {
                                  "type": 1,
                                  "ctor": 2,
                                  "ctorName": "UVe",
                                  "children": [
                                    {
                                      "type": 1,
                                      "ctor": 2,
                                      "ctorName": "Oue",
                                      "children": [
                                        {
                                          "type": 2,
                                          "priority": 1000,
                                          "text": "console.log('Bonjour');",
                                          "references": [],
                                          "lineBreakBefore": true
                                        }
                                      ],
                                      "props": {
                                        "priority": 1000
                                      },
                                      "references": []
                                    },
                                    {
                                      "type": 2,
                                      "priority": 1,
                                      "text": "\n",
                                      "lineBreakBefore": false
                                    }
                                  ],
                                  "props": {
                                    "flexGrow": 1,
                                    "priority": 1
                                  },
                                  "references": [
                                    {
                                      "anchor": {
                                        "uri": {
                                          "$mid": 1,
                                          "fsPath": "/Users/svarlamov/projects/testing-git/index.ts",
                                          "external": "file:///Users/svarlamov/projects/testing-git/index.ts",
                                          "path": "/Users/svarlamov/projects/testing-git/index.ts",
                                          "scheme": "file"
                                        },
                                        "range": [
                                          {
                                            "line": 0,
                                            "character": 13
                                          },
                                          {
                                            "line": 0,
                                            "character": 20
                                          }
                                        ]
                                      },
                                      "options": {
                                        "isFromTool": true
                                      }
                                    }
                                  ]
                                },
                                {
                                  "type": 1,
                                  "ctor": 2,
                                  "ctorName": "",
                                  "children": [
                                    {
                                      "type": 2,
                                      "priority": 9007199254740991,
                                      "text": "</match>",
                                      "lineBreakBefore": false
                                    }
                                  ],
                                  "props": {},
                                  "references": [],
                                  "keepWithId": 61
                                },
                                {
                                  "type": 2,
                                  "priority": 9007199254740991,
                                  "text": "\n",
                                  "lineBreakBefore": false
                                }
                              ],
                              "props": {},
                              "references": []
                            }
                          ],
                          "props": {
                            "passPriority": true
                          },
                          "references": [],
                          "flags": 4
                        }
                      ],
                      "props": {},
                      "references": []
                    }
                  }
                }
              ]
            }
          },
          "modelMessageId": "f5d0a09a-8e81-4f8d-b484-e2d9eb49f248",
          "responseId": "a2235d81-ac38-495b-8a8b-0f75253e4203",
          "sessionId": "29299837-0260-4fa8-8073-f02755e9dbcc",
          "agentId": "github.copilot.editsAgent"
        },
        "details": "Claude Sonnet 4 • 1x"
      },
      "responseMarkdownInfo": [
        {
          "suggestionId": "sgt-44e782d7-4353-4313-a52b-da81217be951"
        }
      ],
      "followups": [],
      "isCanceled": false,
      "agent": {
        "extensionId": {
          "value": "GitHub.copilot-chat",
          "_lower": "github.copilot-chat"
        },
        "extensionVersion": "0.31.5",
        "publisherDisplayName": "GitHub",
        "extensionPublisherId": "GitHub",
        "extensionDisplayName": "GitHub Copilot Chat",
        "id": "github.copilot.editsAgent",
        "description": "Edit files in your workspace in agent mode",
        "when": "config.chat.agent.enabled",
        "metadata": {
          "themeIcon": {
            "id": "tools"
          },
          "hasFollowups": false,
          "supportIssueReporting": false
        },
        "name": "agent",
        "fullName": "GitHub Copilot",
        "isDefault": true,
        "locations": [
          "panel"
        ],
        "modes": [
          "agent"
        ],
        "slashCommands": [
          {
            "name": "list"
          }
        ],
        "disambiguation": []
      },
      "contentReferences": [],
      "codeCitations": [],
      "timestamp": 1759850150757,
      "modelId": "copilot/claude-sonnet-4"
    }
  ],
  "sessionId": "331fed7f-cfd6-409c-8502-06a56322b636",
  "creationDate": 1759845046680,
  "isImported": false,
  "lastMessageDate": 1759850150757,
  "customTitle": "Assistance with TypeScript project tasks"
}"#;
        // I'll change "Bonjour World" to "hello world" in your selected text.
        // index.ts+1-1
        // Done! I've changed "Bonjour World" to "hello world" in line 10 of your index.ts file.
        let result = GithubCopilotPreset::transcript_and_model_from_copilot_session_json(sample);
        assert!(result.is_ok());
        let (tx, model) = result.unwrap();
        use serde_json::json;

        // Build expected transcript messages exactly
        let expected_messages = vec![
            Message::User {
                text: "What can you help me with?".to_string(),
                timestamp: Some("2025-10-07T13:51:13.835+00:00".to_string()),
            },
            Message::Assistant {
                text: "I'm GitHub Copilot, and I can help you with a wide range of programming and development tasks! Based on your current workspace, I can see you have a TypeScript project with an @/Users/svarlamov/projects/testing-git/index.ts file that prints \"Bonjour\" and numbers 1-10.\n\nHere are some ways I can assist you:\n\n## Code Development\n- **Write, edit, and refactor code** in various languages (TypeScript, JavaScript, Python, Java, C#, Go, Rust, and many more)\n- **Debug and fix issues** in your existing code\n- **Add new features** or modify existing functionality\n- **Optimize performance** and improve code quality\n\n## Project Management\n- **Set up new projects** and workspaces from scratch\n- **Configure build systems** and development environments\n- **Manage dependencies** and package installations\n- **Create and modify configuration files**\n\n## Git and Version Control\n- **Commit changes** and manage your Git workflow\n- **Create branches** and handle merges\n- **View git history** and resolve conflicts\n- **Work with pull requests** and issues\n\n## Development Tools\n- **Run terminal commands** and scripts\n- **Install and configure VS Code extensions**\n- **Set up debugging configurations**\n- **Work with databases** (PostgreSQL support available)\n\n## Documentation and Testing\n- **Write documentation** and README files\n- **Create and run tests**\n- **Generate code comments** and API documentation\n\nFor your current TypeScript project, I could help you:\n- Add error handling and input validation\n- Convert it to a proper CLI tool with argument parsing\n- Add TypeScript configurations and build scripts\n- Set up testing frameworks\n- Add more sophisticated functionality\n\nWhat specific task would you like help with today?".to_string(),
                timestamp: Some("2025-10-07T13:51:26.196+00:00".to_string()),
            },
            Message::User {
                text: "Change Bonjour World to hello world".to_string(),
                timestamp: Some("2025-10-07T13:51:41.282+00:00".to_string()),
            },
            Message::ToolUse {
                name: "prepareToolInvocation".to_string(),
                input: json!({
                    "kind": "prepareToolInvocation",
                    "toolName": "copilot_replaceString"
                }),
                timestamp: None,
            },
            Message::ToolUse {
                name: "copilot_replaceString".to_string(),
                input: serde_json::Value::String("Using \"Replace String in File\"".to_string()),
                timestamp: None,
            },
            Message::ToolUse {
                name: "textEditGroup".to_string(),
                input: json!({
                    "done": true,
                    "edits": [[{
                        "range": {
                            "endColumn": 30,
                            "endLineNumber": 10,
                            "startColumn": 1,
                            "startLineNumber": 5
                        },
                        "text": "// Print numbers 1 through 10\nfor (let i = 1; i <= 10; i++) {\n\tconsole.log(i);\n}\n\nconsole.log('hello world');"
                    }], []],
                    "kind": "textEditGroup",
                    "uri": {
                        "$mid": 1,
                        "external": "file:///Users/svarlamov/projects/testing-git/index.ts",
                        "fsPath": "/Users/svarlamov/projects/testing-git/index.ts",
                        "path": "/Users/svarlamov/projects/testing-git/index.ts",
                        "scheme": "file"
                    }
                }),
                timestamp: None,
            },
            Message::Assistant {
                text: "I'll change \"Bonjour World\" to \"hello world\" in your selected text. ``` @/Users/svarlamov/projects/testing-git/index.ts ``` Done! I've changed \"Bonjour World\" to \"hello world\" in line 10 of your @/Users/svarlamov/projects/testing-git/index.ts file.".to_string(),
                timestamp: Some("2025-10-07T13:51:50.791+00:00".to_string()),
            },
            Message::User {
                text: "Search for any other mentions of Bonjour".to_string(),
                timestamp: Some("2025-10-07T15:15:50.757+00:00".to_string()),
            },
            Message::ToolUse {
                name: "prepareToolInvocation".to_string(),
                input: json!({
                    "kind": "prepareToolInvocation",
                    "toolName": "copilot_findTextInFiles"
                }),
                timestamp: None,
            },
            Message::ToolUse {
                name: "copilot_findTextInFiles".to_string(),
                input: serde_json::Value::String("Searching text for `Bonjour`".to_string()),
                timestamp: None,
            },
            Message::Assistant {
                text: "I'll search for any other mentions of \"Bonjour\" in your workspace. I found one more mention of \"Bonjour\" in your code! It's on line 1 of @/Users/svarlamov/projects/testing-git/index.ts :\n\n```typescript\nconsole.log('Bonjour');\n```\n\nWould you like me to change this one to \"hello\" as well to make it consistent?".to_string(),
                timestamp: Some("2025-10-07T15:15:59.544+00:00".to_string()),
            },
        ];

        // Validate transcript messages exactly
        assert_eq!(tx.messages, expected_messages);

        // Validate model exactly
        assert_eq!(model, Some("copilot/claude-sonnet-4".to_string()));
    }
}

// HARD TO TEST IN CI. We need to figure out a way to get cursor and some known good data into the cursor dir.
// #[cfg(test)]
// mod tests {
//     use super::*;

//     #[test]
//     fn cursor_preset_with_conversation_id() {
//         // This test requires a real Cursor installation with at least one conversation.
//         // We'll use a sample conversation_id (you can replace with an actual ID from your database)

//         // First, let's try to get a real conversation ID from the database
//         let user_dir = match CursorPreset::cursor_user_dir() {
//             Ok(dir) => dir,
//             Err(_) => {
//                 println!("Cursor not installed, skipping test");
//                 return;
//             }
//         };

//         let global_db = user_dir.join("globalStorage").join("state.vscdb");
//         if !global_db.exists() {
//             println!("Cursor database not found, skipping test");
//             return;
//         }

//         // Get a real conversation ID from the database
//         let conn = match CursorPreset::open_sqlite_readonly(&global_db) {
//             Ok(c) => c,
//             Err(_) => {
//                 println!("Could not open Cursor database, skipping test");
//                 return;
//             }
//         };

//         // Find a conversation with fullConversationHeadersOnly (bubbles format)
//         let mut stmt = match conn.prepare(
//             "SELECT json_extract(value, '$.composerId') FROM cursorDiskKV
//              WHERE key LIKE 'composerData:%'
//              AND json_extract(value, '$.fullConversationHeadersOnly') IS NOT NULL
//              AND json_array_length(json_extract(value, '$.fullConversationHeadersOnly')) > 0
//              LIMIT 1",
//         ) {
//             Ok(s) => s,
//             Err(_) => {
//                 println!("Could not query database, skipping test");
//                 return;
//             }
//         };

//         let conversation_id: String = match stmt.query_row([], |row| row.get(0)) {
//             Ok(id) => id,
//             Err(_) => {
//                 println!("No conversations with messages found in database, skipping test");
//                 return;
//             }
//         };

//         println!("Testing with conversation_id: {}", conversation_id);

//         // Create mock hook_input
//         let hook_input = serde_json::json!({
//             "conversation_id": conversation_id,
//             "workspace_roots": ["/tmp/test-workspace"]
//         });

//         let preset = CursorPreset;
//         let result = preset.run(AgentCheckpointFlags {
//             prompt_id: None,
//             hook_input: Some(hook_input.to_string()),
//         });

//         match result {
//             Ok(run) => {
//                 println!("✓ Cursor Agent: {}:{}", run.agent_id.tool, run.agent_id.id);
//                 println!("✓ Model: {}", run.agent_id.model);
//                 let transcript = run.transcript.unwrap();
//                 println!("✓ Message count: {}", transcript.messages.len());

//                 for (i, m) in transcript.messages.iter().enumerate() {
//                     match m {
//                         Message::User { text, .. } => {
//                             let preview = if text.len() > 100 {
//                                 format!("{}...", &text[..100])
//                             } else {
//                                 text.clone()
//                             };
//                             println!("  [{}] User: {}", i, preview);
//                         }
//                         Message::Assistant { text, .. } => {
//                             let preview = if text.len() > 100 {
//                                 format!("{}...", &text[..100])
//                             } else {
//                                 text.clone()
//                             };
//                             println!("  [{}] Assistant: {}", i, preview);
//                         }
//                         Message::ToolUse { name, input, .. } => {
//                             println!(
//                                 "  [{}] ToolUse: {} (input: {} chars)",
//                                 i,
//                                 name,
//                                 input.to_string().len()
//                             );
//                         }
//                     }
//                 }

//                 // Assert that we got at least some messages
//                 assert!(
//                     !transcript.messages.is_empty(),
//                     "Transcript should have at least one message"
//                 );
//                 assert_eq!(run.agent_id.tool, "cursor");
//                 assert_eq!(run.agent_id.id, conversation_id);
//             }
//             Err(e) => {
//                 panic!("CursorPreset error: {}", e);
//             }
//         }
//     }
// }
