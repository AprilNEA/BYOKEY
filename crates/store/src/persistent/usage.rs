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
            "INSERT INTO usage_records (model, provider, input_tokens, output_tokens, success, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
            vec![
                rec.model.clone().into(),
                rec.provider.clone().into(),
                (rec.input_tokens as i64).into(),
                (rec.output_tokens as i64).into(),
                i32::from(rec.success).into(),
                now_unix().into(),
            ],
        );
        self.connection().execute_raw(stmt).await?;
        Ok(())
    }

    async fn query(
        &self,
        from: i64,
        to: i64,
        model: Option<&str>,
        bucket_secs: i64,
    ) -> Result<Vec<UsageBucket>> {
        let (sql, values) = if let Some(m) = model {
            (
                format!(
                    "SELECT (created_at / {bucket_secs}) * {bucket_secs} AS period_start,
                            model,
                            COUNT(*)      AS request_count,
                            SUM(input_tokens)  AS input_tokens,
                            SUM(output_tokens) AS output_tokens
                     FROM usage_records
                     WHERE created_at >= ? AND created_at < ? AND model = ?
                     GROUP BY period_start, model
                     ORDER BY period_start"
                ),
                vec![from.into(), to.into(), m.to_string().into()],
            )
        } else {
            (
                format!(
                    "SELECT (created_at / {bucket_secs}) * {bucket_secs} AS period_start,
                            model,
                            COUNT(*)      AS request_count,
                            SUM(input_tokens)  AS input_tokens,
                            SUM(output_tokens) AS output_tokens
                     FROM usage_records
                     WHERE created_at >= ? AND created_at < ?
                     GROUP BY period_start, model
                     ORDER BY period_start"
                ),
                vec![from.into(), to.into()],
            )
        };

        let stmt =
            Statement::from_sql_and_values(self.connection().get_database_backend(), &sql, values);
        let rows = self.connection().query_all_raw(stmt).await?;

        let mut buckets = Vec::with_capacity(rows.len());
        for row in &rows {
            #[allow(clippy::cast_sign_loss)]
            buckets.push(UsageBucket {
                period_start: row.try_get_by_index::<i64>(0).unwrap_or(0),
                model: row.try_get_by_index::<String>(1).unwrap_or_default(),
                request_count: row.try_get_by_index::<i64>(2).unwrap_or(0) as u64,
                input_tokens: row.try_get_by_index::<i64>(3).unwrap_or(0) as u64,
                output_tokens: row.try_get_by_index::<i64>(4).unwrap_or(0) as u64,
            });
        }
        Ok(buckets)
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
            "SELECT 0 AS period_start,
                    model,
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
                period_start: 0,
                model: row.try_get_by_index::<String>(1).unwrap_or_default(),
                request_count: row.try_get_by_index::<i64>(2).unwrap_or(0) as u64,
                input_tokens: row.try_get_by_index::<i64>(3).unwrap_or(0) as u64,
                output_tokens: row.try_get_by_index::<i64>(4).unwrap_or(0) as u64,
            });
        }
        Ok(buckets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem() -> SqliteTokenStore {
        SqliteTokenStore::new("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn test_record_and_totals() {
        let s = mem().await;
        s.record(&UsageRecord {
            model: "gpt-4o".into(),
            provider: "codex".into(),
            input_tokens: 100,
            output_tokens: 50,
            success: true,
        })
        .await
        .unwrap();
        s.record(&UsageRecord {
            model: "gpt-4o".into(),
            provider: "codex".into(),
            input_tokens: 200,
            output_tokens: 100,
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

    #[tokio::test]
    async fn test_query_buckets() {
        let s = mem().await;
        let now = now_unix();
        s.record(&UsageRecord {
            model: "claude-opus-4-5".into(),
            provider: "claude".into(),
            input_tokens: 10,
            output_tokens: 5,
            success: true,
        })
        .await
        .unwrap();

        let buckets = s.query(now - 3600, now + 3600, None, 3600).await.unwrap();
        assert!(!buckets.is_empty());
        assert_eq!(buckets[0].model, "claude-opus-4-5");
        assert_eq!(buckets[0].input_tokens, 10);
    }
}
