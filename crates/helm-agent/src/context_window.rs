//! Rolling context trimmer to prevent token-limit parse failures on Groq.

use helm_core::{ContentBlock, Message, Role};

const TOKEN_BUDGET: usize = 12_000;
const BYTES_PER_TOKEN: usize = 4;
const KEEP_RECENT_EXCHANGES: usize = 4;

/// Trims `messages` to stay within the token budget.
///
/// Returns the trimmed message list and the number of assistant turns that
/// were collapsed into a summary.  When no trimming is needed the original
/// messages are returned unchanged with a count of 0.
pub fn trim(messages: &[Message]) -> (Vec<Message>, u32) {
    if estimate_tokens(messages) <= TOKEN_BUDGET {
        return (messages.to_vec(), 0);
    }

    // messages[0] is always the initial user goal — keep it.
    if messages.len() <= 1 {
        return (messages.to_vec(), 0);
    }

    let tail = &messages[1..];
    // Keep the last KEEP_RECENT_EXCHANGES exchange pairs (assistant + user = 2).
    let keep_count = (KEEP_RECENT_EXCHANGES * 2).min(tail.len());
    let summarize_count = tail.len().saturating_sub(keep_count);

    if summarize_count == 0 {
        return (messages.to_vec(), 0);
    }

    let summarized = &tail[..summarize_count];
    let kept = &tail[summarize_count..];

    let turns_collapsed = summarized
        .iter()
        .filter(|msg| msg.role == Role::Assistant)
        .count() as u32;

    let summary =
        format!("[{turns_collapsed} earlier turn(s) summarized to reduce context length]");

    let mut trimmed = Vec::with_capacity(2 + kept.len());
    trimmed.push(messages[0].clone());
    trimmed.push(Message::user(summary));
    trimmed.extend_from_slice(kept);

    (trimmed, turns_collapsed)
}

fn estimate_tokens(messages: &[Message]) -> usize {
    messages
        .iter()
        .flat_map(|msg| &msg.content)
        .map(content_bytes)
        .sum::<usize>()
        / BYTES_PER_TOKEN
}

fn content_bytes(block: &ContentBlock) -> usize {
    match block {
        ContentBlock::Text(text) => text.len(),
        ContentBlock::ToolUse { name, input, .. } => name.len() + input.to_string().len(),
        ContentBlock::ToolResult { content, .. } => content.len(),
    }
}

#[cfg(test)]
mod tests {
    use helm_core::{ContentBlock, Message};

    use super::{BYTES_PER_TOKEN, TOKEN_BUDGET, trim};

    fn big_text(n: usize) -> String {
        "x".repeat(n)
    }

    #[test]
    fn no_trim_when_under_budget_happy_path() {
        let messages = vec![
            Message::user("goal"),
            Message::assistant(vec![ContentBlock::Text("ok".to_owned())]),
        ];
        let (trimmed, count) = trim(&messages);
        assert_eq!(count, 0);
        assert_eq!(trimmed.len(), messages.len());
    }

    #[test]
    fn trims_old_exchanges_when_over_budget_happy_path() {
        let big = big_text(TOKEN_BUDGET * BYTES_PER_TOKEN + 100);
        let mut messages = vec![Message::user("goal")];
        for _ in 0..10 {
            messages.push(Message::assistant(vec![ContentBlock::Text(big.clone())]));
            messages.push(Message::user("tool result"));
        }
        let (trimmed, count) = trim(&messages);
        assert!(count > 0);
        assert_eq!(trimmed[0], Message::user("goal"));
        match &trimmed[1].content[0] {
            ContentBlock::Text(t) => assert!(t.contains("summarized")),
            other => panic!("expected text summary, got {other:?}"),
        }
    }

    #[test]
    fn single_message_never_trimmed_edge_case() {
        let big = big_text(TOKEN_BUDGET * BYTES_PER_TOKEN * 2);
        let messages = vec![Message::user(big)];
        let (trimmed, count) = trim(&messages);
        assert_eq!(count, 0);
        assert_eq!(trimmed.len(), 1);
    }

    #[test]
    fn goal_message_always_preserved_edge_case() {
        let big = big_text(TOKEN_BUDGET * BYTES_PER_TOKEN);
        let mut messages = vec![Message::user("original goal")];
        for _ in 0..20 {
            messages.push(Message::assistant(vec![ContentBlock::Text(big.clone())]));
            messages.push(Message::user("result"));
        }
        let (trimmed, _) = trim(&messages);
        assert_eq!(trimmed[0], Message::user("original goal"));
    }
}
