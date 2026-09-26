//! [`UsageStore`] implementation for [`SqliteTokenStore`].

use async_trait::async_trait;
use byokey_types::{Result, UsageBucket, UsageRecord, UsageStore};
use sea_orm::{ConnectionTrait, Statement};

use super::{SqliteTokenStore, now_unix};

#[async_trait]
impl UsageStore for SqliteTokenStore {
    async fn record(&self, rec: &UsageRecord) -> Result<()> {
        #[allow(clippy::cast_possible_wrap)]
        let stmt = Statement::from_sql_and_values(
            self.connection().get_database_backend(),
            "INSERT INTO usage_records (model, provider, account_id, input_tokens, output_tokens, success, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            vec![
                rec.model.clone().into(),
                rec.provider.to_string().into(),
                rec.account_id.clone().into(),
                (rec.usage.input_tokens as i64).into(),
                (rec.usage.output_tokens as i64).into(),
                i32::from(rec.success).into(),
                now_unix().into(),
            ],
        );
        self.connection().execute_raw(stmt).await.map_err(|e| {
            tracing::warn!(
                model = %rec.model,
                provider = %rec.provider,
                account_id = %rec.account_id,
                error = %e,
                "usage insert failed — billing record lost"
            );
            e
        })?;
        Ok(())
    }

    async fn totals(&self, from: Option<i64>, to: Option<i64>) -> Result<Vec<UsageBucket>> {
        let (where_clause, values) = match (from, to) {
            (Some(f), Some(t)) => (
                "WHERE created_at >= ? AND created_at < ?".to_string(),
                vec![f.into(), t.into()],
            ),
            (Some(f), None) => ("WHERE created_at >= ?".to_string(), vec![f.into()]),
            (None, Some(t)) => ("WHERE created_at < ?".to_string(), vec![t.into()]),
            (None, None) => (String::new(), vec![]),
        };

        let sql = format!(
            "SELECT model,
                    COUNT(*)      AS request_count,
                    SUM(input_tokens)  AS input_tokens,
                    SUM(output_tokens) AS output_tokens
             FROM usage_records
             {where_clause}
             GROUP BY model
             ORDER BY model"
        );

        let stmt =
            Statement::from_sql_and_values(self.connection().get_database_backend(), &sql, values);
        let rows = self.connection().query_all_raw(stmt).await?;

        let mut buckets = Vec::with_capacity(rows.len());
        for row in &rows {
            #[allow(clippy::cast_sign_loss)]
            buckets.push(UsageBucket {
                model: row.try_get_by_index::<String>(0).unwrap_or_default(),
                request_count: row.try_get_by_index::<i64>(1).unwrap_or(0) as u64,
                input_tokens: row.try_get_by_index::<i64>(2).unwrap_or(0) as u64,
                output_tokens: row.try_get_by_index::<i64>(3).unwrap_or(0) as u64,
            });
        }
        Ok(buckets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_types::{ProviderId, Usage};

    async fn mem() -> SqliteTokenStore {
        SqliteTokenStore::new("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn test_record_and_totals() {
        let s = mem().await;
        s.record(&UsageRecord {
            model: "gpt-4o".into(),
            provider: ProviderId::Copilot,
            account_id: "default".into(),
            usage: Usage {
                input_tokens: 100,
                output_tokens: 50,
            },
            success: true,
        })
        .await
        .unwrap();
        s.record(&UsageRecord {
            model: "gpt-4o".into(),
            provider: ProviderId::Copilot,
            account_id: "default".into(),
            usage: Usage {
                input_tokens: 200,
                output_tokens: 100,
            },
            success: true,
        })
        .await
        .unwrap();

        let totals = s.totals(None, None).await.unwrap();
        assert_eq!(totals.len(), 1);
        assert_eq!(totals[0].model, "gpt-4o");
        assert_eq!(totals[0].request_count, 2);
        assert_eq!(totals[0].input_tokens, 300);
        assert_eq!(totals[0].output_tokens, 150);
    }
}
