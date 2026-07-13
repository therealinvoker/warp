//! A singleton model for storing conversations by ID to enable restoration across terminal views.

use std::collections::HashMap;

use chrono::NaiveDateTime;
use warpui::{Entity, SingletonEntity};

use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::blocklist::history_model::convert_persisted_conversation_to_ai_conversation_with_metadata;
use crate::persistence::model::AgentConversation;

/// Singleton model that holds restored agent conversations on app startup.
///
/// Loading restored conversations into this model is a means of propagating restored data from
/// sqlite (read at startup) to arbitrary consuming locations in the view/model hierarchy without
/// piping it all the way from the root view to the terminal view(s) that require it.
#[derive(Default)]
pub struct RestoredAgentConversations {
    /// All conversations stored by their ID, available for restoration
    conversations: HashMap<AIConversationId, AIConversation>,
    /// Persisted `last_modified_at` per conversation, used to pick the most
    /// recent conversation as a session-restore fallback. Preserved here because
    /// it is dropped when converting to `AIConversation`.
    last_modified_at: HashMap<AIConversationId, NaiveDateTime>,
}

impl RestoredAgentConversations {
    pub fn new(conversations: Vec<AgentConversation>) -> Self {
        let mut conversations_by_id = HashMap::new();
        let mut last_modified_at = HashMap::new();
        for conversation in conversations.into_iter() {
            let conversation_id = conversation.conversation.conversation_id.clone();
            let last_modified = conversation.conversation.last_modified_at;
            let Some(conversation) =
                convert_persisted_conversation_to_ai_conversation_with_metadata(conversation)
            else {
                log::warn!(
                    "Failed to convert persisted conversation {conversation_id} to AIConversation"
                );
                continue;
            };
            last_modified_at.insert(conversation.id(), last_modified);
            conversations_by_id.insert(conversation.id(), conversation);
        }

        Self {
            conversations: conversations_by_id,
            last_modified_at,
        }
    }

    /// Gets a reference to a restored conversation without removing it.
    pub fn get_conversation(&self, id: &AIConversationId) -> Option<&AIConversation> {
        self.conversations.get(id)
    }

    /// Removes the restored conversation and returns it, if any.
    pub fn take_conversation(&mut self, id: &AIConversationId) -> Option<AIConversation> {
        self.last_modified_at.remove(id);
        self.conversations.remove(id)
    }

    /// Removes and returns the most recently modified conversation that is worth
    /// restoring (has at least one task and is not entirely passive).
    ///
    /// Used as a session-restore fallback when a terminal pane referenced
    /// conversations that all turned out to be empty/unrestorable (e.g. it
    /// landed on a fresh empty conversation while the real chat had been evicted
    /// from memory when the snapshot was taken), so the user's last chat comes
    /// back instead of a blank terminal. Removing on take keeps this dedup-safe
    /// across multiple restoring panes.
    pub fn take_most_recent_restorable_conversation(&mut self) -> Option<AIConversation> {
        let id = self
            .conversations
            .iter()
            .filter(|(_, conversation)| {
                conversation.all_tasks().next().is_some() && !conversation.is_entirely_passive()
            })
            .max_by_key(|(id, _)| self.last_modified_at.get(id).copied())
            .map(|(id, _)| *id)?;
        self.take_conversation(&id)
    }

    /// Takes and returns AIConversations for the given IDs, sorted by first exchange start time.
    pub fn take_conversations(
        &mut self,
        conversation_ids: &[AIConversationId],
    ) -> Vec<AIConversation> {
        let mut conversations = Vec::new();
        for &conversation_id in conversation_ids {
            if let Some(conversation) = self.take_conversation(&conversation_id) {
                conversations.push(conversation);
            }
        }

        // Sort by first exchange start time (oldest first)
        conversations.sort_by_key(|conversation| {
            conversation
                .first_exchange()
                .map(|exchange| exchange.start_time)
        });
        conversations
    }
}

impl Entity for RestoredAgentConversations {
    type Event = ();
}

impl SingletonEntity for RestoredAgentConversations {}
