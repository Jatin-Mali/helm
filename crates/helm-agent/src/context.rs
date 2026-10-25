use helm_core::{ContentBlock, Message, Role};

/// Trim whole exchanges only; retain the latest user request and newest tool evidence.
pub(crate) fn trim(messages: &mut Vec<Message>, limit: usize) -> Result<(), String> {
    while serde_json::to_vec(messages)
        .map_err(|e| e.to_string())?
        .len()
        > limit
    {
        let goal = messages
            .iter()
            .rposition(|message| {
                message.role == Role::User
                    && message
                        .content
                        .iter()
                        .any(|block| matches!(block, ContentBlock::Text(_)))
            })
            .unwrap_or(0);
        if goal > 0 {
            messages.drain(..goal);
            continue;
        }
        let end = if messages.get(1).is_some_and(|message| {
            message
                .content
                .iter()
                .any(|block| matches!(block, ContentBlock::ToolUse { .. }))
        }) {
            3
        } else {
            2
        };
        if messages.len() <= end {
            return Err(
                "context limit cannot fit the current goal and newest tool exchange".into(),
            );
        }
        messages.drain(1..end);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trimming_preserves_tool_pair_and_current_goal() {
        let call = |id: &str| {
            Message::assistant(vec![ContentBlock::ToolUse {
                id: id.into(),
                name: "read".into(),
                input: serde_json::json!({}),
            }])
        };
        let result = |id: &str| {
            Message::tool_results(vec![ContentBlock::ToolResult {
                tool_use_id: id.into(),
                content: "evidence".repeat(10),
                is_error: false,
            }])
        };
        let mut messages = vec![
            Message::user("goal"),
            call("old"),
            result("old"),
            call("new"),
            result("new"),
        ];
        let expected = vec![
            messages[0].clone(),
            messages[3].clone(),
            messages[4].clone(),
        ];
        let limit = serde_json::to_vec(&expected).unwrap().len();
        trim(&mut messages, limit).unwrap();
        assert_eq!(messages, expected);
        assert!(trim(&mut messages, 16).is_err());
    }
}
