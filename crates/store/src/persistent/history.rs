//! [`ChatHistoryStore`] implementation for [`SqliteTokenStore`].

use async_trait::async_trait;
use byokey_types::{
    ByokError, ChatHistoryStore, ConversationSummary, MessageRecord, traits::Result,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};

use super::{SqliteTokenStore, db_exec_raw, now_unix};
use crate::entity::{conversation, message};

#[async_trait]
impl ChatHistoryStore for SqliteTokenStore {
    async fn create_conversation(
        &self,
        id: &str,
        model: &str,
        provider: &str,
        title: Option<&str>,
    ) -> Result<()> {
        let now = now_unix();
        let m = conversation::ActiveModel {
            id: Set(id.to_string()),
            title: Set(title.map(String::from)),
            model: Set(model.to_string()),
            provider: Set(provider.to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        };
        m.insert(&self.db).await?;
        Ok(())
    }

    async fn append_message(&self, msg: &MessageRecord) -> Result<()> {
        let extra = msg
            .extra
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| ByokError::Storage(e.to_string()))?;

        #[allow(clippy::cast_possible_wrap)]
        let m = message::ActiveModel {
            id: Set(msg.id.clone()),
            conversation_id: Set(msg.conversation_id.clone()),
            role: Set(msg.role.clone()),
            content: Set(msg.content.clone()),
            input_tokens: Set(msg.input_tokens.map(|v| v as i64)),
            output_tokens: Set(msg.output_tokens.map(|v| v as i64)),
            model: Set(msg.model.clone()),
            finish_reason: Set(msg.finish_reason.clone()),
            duration_ms: Set(msg.duration_ms.map(|v| v as i64)),
            extra_json: Set(extra),
            created_at: Set(msg.created_at),
        };
        m.insert(&self.db).await?;

        // Update the conversation's updated_at timestamp.
        db_exec_raw(
            &self.db,
            "UPDATE conversations SET updated_at = ? WHERE id = ?",
            vec![now_unix().into(), msg.conversation_id.clone().into()],
        )
        .await?;

        Ok(())
    }

    async fn list_conversations(
        &self,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<ConversationSummary>> {
        let rows = conversation::Entity::find()
            .order_by_desc(conversation::Column::UpdatedAt)
            .limit(limit)
            .offset(offset)
            .all(&self.db)
            .await?;

        Ok(rows
            .into_iter()
            .map(|m| ConversationSummary {
                id: m.id,
                title: m.title,
                model: m.model,
                provider: m.provider,
                created_at: m.created_at,
                updated_at: m.updated_at,
            })
            .collect())
    }

    async fn get_messages(&self, conversation_id: &str) -> Result<Vec<MessageRecord>> {
        let rows = message::Entity::find()
            .filter(message::Column::ConversationId.eq(conversation_id))
            .order_by_asc(message::Column::CreatedAt)
            .all(&self.db)
            .await?;

        let mut result = Vec::with_capacity(rows.len());
        for m in rows {
            let extra = m
                .extra_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(|e| ByokError::Storage(e.to_string()))?;

            #[allow(clippy::cast_sign_loss)]
            result.push(MessageRecord {
                id: m.id,
                conversation_id: m.conversation_id,
                role: m.role,
                content: m.content,
                input_tokens: m.input_tokens.map(|v| v as u64),
                output_tokens: m.output_tokens.map(|v| v as u64),
                model: m.model,
                finish_reason: m.finish_reason,
                duration_ms: m.duration_ms.map(|v| v as u64),
                extra,
                created_at: m.created_at,
            });
        }
        Ok(result)
    }

    async fn delete_conversation(&self, conversation_id: &str) -> Result<()> {
        // Delete messages first (foreign key), then conversation.
        message::Entity::delete_many()
            .filter(message::Column::ConversationId.eq(conversation_id))
            .exec(&self.db)
            .await?;
        conversation::Entity::delete_by_id(conversation_id.to_string())
            .exec(&self.db)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem() -> SqliteTokenStore {
        SqliteTokenStore::new("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn test_create_conversation() {
        let s = mem().await;
        s.create_conversation("conv-1", "claude-opus-4-5", "claude", Some("Hello"))
            .await
            .unwrap();
        let convos = s.list_conversations(10, 0).await.unwrap();
        assert_eq!(convos.len(), 1);
        assert_eq!(convos[0].id, "conv-1");
        assert_eq!(convos[0].title.as_deref(), Some("Hello"));
        assert_eq!(convos[0].model, "claude-opus-4-5");
    }

    #[tokio::test]
    async fn test_append_and_get_messages() {
        let s = mem().await;
        s.create_conversation("conv-2", "gpt-4o", "codex", None)
            .await
            .unwrap();

        let msg1 = MessageRecord {
            id: "msg-1".into(),
            conversation_id: "conv-2".into(),
            role: "user".into(),
            content: "Hello".into(),
            input_tokens: None,
            output_tokens: None,
            model: None,
            finish_reason: None,
            duration_ms: None,
            extra: None,
            created_at: now_unix(),
        };
        s.append_message(&msg1).await.unwrap();

        let msg2 = MessageRecord {
            id: "msg-2".into(),
            conversation_id: "conv-2".into(),
            role: "assistant".into(),
            content: "Hi there!".into(),
            input_tokens: Some(10),
            output_tokens: Some(5),
            model: Some("gpt-4o".into()),
            finish_reason: Some("stop".into()),
            duration_ms: Some(150),
            extra: None,
            created_at: now_unix(),
        };
        s.append_message(&msg2).await.unwrap();

        let messages = s.get_messages("conv-2").await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "Hello");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].input_tokens, Some(10));
        assert_eq!(messages[1].finish_reason.as_deref(), Some("stop"));
    }

    #[tokio::test]
    async fn test_delete_conversation() {
        let s = mem().await;
        s.create_conversation("conv-3", "claude-opus-4-5", "claude", None)
            .await
            .unwrap();
        let msg = MessageRecord {
            id: "msg-del".into(),
            conversation_id: "conv-3".into(),
            role: "user".into(),
            content: "bye".into(),
            input_tokens: None,
            output_tokens: None,
            model: None,
            finish_reason: None,
            duration_ms: None,
            extra: None,
            created_at: now_unix(),
        };
        s.append_message(&msg).await.unwrap();

        s.delete_conversation("conv-3").await.unwrap();
        let convos = s.list_conversations(10, 0).await.unwrap();
        assert!(convos.is_empty());
        let msgs = s.get_messages("conv-3").await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn test_list_conversations_ordering() {
        let s = mem().await;
        s.create_conversation("conv-a", "m1", "p1", Some("First"))
            .await
            .unwrap();
        s.create_conversation("conv-b", "m2", "p2", Some("Second"))
            .await
            .unwrap();

        // Append a message to conv-b to bump its updated_at above conv-a.
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let msg = MessageRecord {
            id: "msg-order".into(),
            conversation_id: "conv-b".into(),
            role: "user".into(),
            content: "bump".into(),
            input_tokens: None,
            output_tokens: None,
            model: None,
            finish_reason: None,
            duration_ms: None,
            extra: None,
            created_at: now_unix(),
        };
        s.append_message(&msg).await.unwrap();

        let convos = s.list_conversations(10, 0).await.unwrap();
        assert_eq!(convos.len(), 2);
        // Newest first
        assert_eq!(convos[0].id, "conv-b");
        assert_eq!(convos[1].id, "conv-a");
    }

    #[tokio::test]
    async fn test_list_conversations_pagination() {
        let s = mem().await;
        for i in 0..5 {
            s.create_conversation(&format!("c-{i}"), "m", "p", None)
                .await
                .unwrap();
        }

        let page1 = s.list_conversations(2, 0).await.unwrap();
        assert_eq!(page1.len(), 2);
        let page2 = s.list_conversations(2, 2).await.unwrap();
        assert_eq!(page2.len(), 2);
        let page3 = s.list_conversations(2, 4).await.unwrap();
        assert_eq!(page3.len(), 1);
    }

    #[tokio::test]
    async fn test_message_extra_json() {
        let s = mem().await;
        s.create_conversation("conv-extra", "m", "p", None)
            .await
            .unwrap();
        let extra = serde_json::json!({"tool_calls": [{"id": "t1", "type": "function"}]});
        let msg = MessageRecord {
            id: "msg-extra".into(),
            conversation_id: "conv-extra".into(),
            role: "assistant".into(),
            content: String::new(),
            input_tokens: None,
            output_tokens: None,
            model: None,
            finish_reason: Some("tool_calls".into()),
            duration_ms: None,
            extra: Some(extra.clone()),
            created_at: now_unix(),
        };
        s.append_message(&msg).await.unwrap();

        let msgs = s.get_messages("conv-extra").await.unwrap();
        assert_eq!(msgs[0].extra, Some(extra));
    }
}
