use crate::{
    error::GitAiError,
    log_fmt::{transcript::AiTranscript, working_log::AgentId},
};
use std::path::Path;

pub struct AgentCheckpointFlags {
    pub transcript: Option<String>,
    pub model: Option<String>,
    pub prompt_id: Option<String>,
    pub prompt_path: Option<String>,
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
        // Parse the prompt_path - it's required
        let prompt_path = flags.prompt_path.ok_or_else(|| {
            GitAiError::PresetError(
                "prompt_path is required for Claude Code preset but not provided".to_string(),
            )
        })?;

        // Extract the ID from the filename
        // Example: /Users/aidancunniffe/.claude/projects/-Users-aidancunniffe-Desktop-ghq/cb947e5b-246e-4253-a953-631f7e464c6b.jsonl
        let path = Path::new(&prompt_path);
        let filename = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| {
                GitAiError::PresetError("Could not extract filename from prompt_path".to_string())
            })?;

        // The filename should be a UUID
        let agent_id = AgentId {
            tool: "claude".to_string(),
            id: filename.to_string(),
        };

        // Read the file content
        let jsonl_content =
            std::fs::read_to_string(&prompt_path).map_err(|e| GitAiError::IoError(e))?;

        // Parse into transcript
        let transcript = AiTranscript::from_claude_code_jsonl(&jsonl_content)
            .map_err(|e| GitAiError::JsonError(e))?;

        Ok(AgentRunResult {
            agent_id,
            transcript,
        })
    }
}
