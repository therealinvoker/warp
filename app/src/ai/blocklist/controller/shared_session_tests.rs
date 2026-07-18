use super::{AIConversationId, BlocklistAIController};

/// A viewer that addresses a specific conversation via its token continues that
/// conversation, even when a different conversation is currently active.
#[test]
fn prefers_token_resolved_conversation() {
    let from_token = AIConversationId::new();
    let active = AIConversationId::new();

    let resolved = BlocklistAIController::resolve_remote_prompt_conversation_id(
        Some(from_token),
        Some(active),
    );

    assert_eq!(resolved, Some(from_token));
}

/// A tokenless remote prompt (e.g. from the mobile web viewer) continues the
/// sharer's currently active conversation instead of starting a new one.
#[test]
fn falls_back_to_active_conversation_when_token_missing() {
    let active = AIConversationId::new();

    let resolved = BlocklistAIController::resolve_remote_prompt_conversation_id(None, Some(active));

    assert_eq!(resolved, Some(active));
}

/// With neither a resolvable token nor an active conversation, no conversation
/// is returned so the caller can create a fresh one.
#[test]
fn returns_none_when_no_conversation_available() {
    let resolved = BlocklistAIController::resolve_remote_prompt_conversation_id(None, None);

    assert_eq!(resolved, None);
}
