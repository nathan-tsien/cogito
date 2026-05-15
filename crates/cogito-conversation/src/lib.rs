//! cogito-conversation
//!
//! Conversation Service: the persistent event log that is the single source
//! of truth for session state.
//!
//! Provides the `ConversationStore` trait and two implementations:
//! - `SqliteConversationStore` for normal use
//! - `InMemoryConversationStore` for tests
