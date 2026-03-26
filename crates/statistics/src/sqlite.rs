use crate::event::RoutingEvent;
use crate::query::{AggQuery, AggStats, EventFilter};
use crate::{Result, StatisticsError};
use rusqlite::{Connection, params};
use std::sync::{Arc, Mutex};

/// SQLite 存储实现
#[derive(Clone)]
pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStore {
    /// 创建新的 SQLite 存储
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = Connection::open(db_path).map_err(|e| {
            StatisticsError::DatabaseError(format!("Failed to open database: {}", e))
        })?;

        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };

        store.init_schema()?;
        Ok(store)
    }

    /// 创建内存存储（用于测试）
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(|e| {
            StatisticsError::DatabaseError(format!("Failed to open in-memory database: {}", e))
        })?;

        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };

        store.init_schema()?;
        Ok(store)
    }

    /// 初始化数据库表结构
    fn init_schema(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StatisticsError::DatabaseError(format!("Mutex poisoned: {}", e)))?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp INTEGER NOT NULL,
                remote_addr INTEGER NOT NULL,
                remote_port INTEGER NOT NULL,
                method TEXT NOT NULL,
                path TEXT NOT NULL,
                input_port INTEGER NOT NULL,
                model TEXT NOT NULL,
                routing_path TEXT NOT NULL,
                backend TEXT NOT NULL,
                success INTEGER NOT NULL,
                duration_ms INTEGER NOT NULL,
                error_type TEXT,
                request_size INTEGER,
                response_size INTEGER
            );

            CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);
            CREATE INDEX IF NOT EXISTS idx_events_model ON events(model);
            CREATE INDEX IF NOT EXISTS idx_events_backend ON events(backend);
            CREATE INDEX IF NOT EXISTS idx_events_success ON events(success);

            -- 创建聚合统计表
            CREATE TABLE IF NOT EXISTS aggregated_stats (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                window_start INTEGER NOT NULL,
                window_size INTEGER NOT NULL,
                model TEXT NOT NULL,
                backend TEXT NOT NULL,
                total_requests INTEGER NOT NULL,
                success_count INTEGER NOT NULL,
                fail_count INTEGER NOT NULL,
                avg_duration_ms INTEGER NOT NULL,
                min_duration_ms INTEGER NOT NULL,
                max_duration_ms INTEGER NOT NULL,
                p50_duration_ms INTEGER,
                p90_duration_ms INTEGER,
                p99_duration_ms INTEGER,
                UNIQUE(window_start, window_size, model, backend)
            );

            CREATE INDEX IF NOT EXISTS idx_agg_window ON aggregated_stats(window_start, window_size);
            CREATE INDEX IF NOT EXISTS idx_agg_model ON aggregated_stats(model);
            ",
        )
        .map_err(|e| StatisticsError::DatabaseError(format!("Failed to create schema: {}", e)))?;

        Ok(())
    }

    /// 插入单个事件
    pub fn insert_event(&self, event: &RoutingEvent) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StatisticsError::DatabaseError(format!("Mutex poisoned: {}", e)))?;

        conn.execute(
            "
            INSERT INTO events (
                timestamp, remote_addr, remote_port, method, path, input_port,
                model, routing_path, backend,
                success, duration_ms, error_type, request_size, response_size
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            ",
            params![
                event.timestamp,
                event.remote_addr,
                event.remote_port,
                event.method,
                event.path,
                event.input_port,
                event.model,
                event.routing_path,
                event.backend,
                if event.success { 1 } else { 0 },
                event.duration_ms,
                event.error_type,
                event.request_size,
                event.response_size,
            ],
        )
        .map_err(|e| StatisticsError::DatabaseError(format!("Failed to insert event: {}", e)))?;

        Ok(())
    }

    /// 查询原始事件
    pub fn query_events_internal(&self, filter: &EventFilter) -> Result<Vec<RoutingEvent>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StatisticsError::DatabaseError(format!("Mutex poisoned: {}", e)))?;

        // 构建参数数组
        let mut sql_parts = Vec::new();
        let mut param_values: Vec<rusqlite::types::Value> = Vec::new();

        sql_parts.push(
            "SELECT timestamp, remote_addr, remote_port, method, path, input_port,
                    model, routing_path, backend,
                    success, duration_ms, error_type, request_size, response_size
             FROM events WHERE 1=1"
                .to_string(),
        );

        if let Some(start) = filter.start_time {
            sql_parts.push("AND timestamp >= ?".to_string());
            param_values.push(rusqlite::types::Value::Integer(start));
        }

        if let Some(end) = filter.end_time {
            sql_parts.push("AND timestamp < ?".to_string());
            param_values.push(rusqlite::types::Value::Integer(end));
        }

        if let Some(ref model) = filter.model {
            sql_parts.push("AND model = ?".to_string());
            param_values.push(rusqlite::types::Value::Text(model.clone()));
        }

        if let Some(ref backend) = filter.backend {
            sql_parts.push("AND backend = ?".to_string());
            param_values.push(rusqlite::types::Value::Text(backend.clone()));
        }

        if let Some(success) = filter.success {
            sql_parts.push("AND success = ?".to_string());
            param_values.push(rusqlite::types::Value::Integer(if success { 1 } else { 0 }));
        }

        sql_parts.push("ORDER BY timestamp DESC".to_string());

        if let Some(limit) = filter.limit {
            sql_parts.push("LIMIT ?".to_string());
            param_values.push(rusqlite::types::Value::Integer(limit as i64));
            if let Some(offset) = filter.offset {
                sql_parts.push("OFFSET ?".to_string());
                param_values.push(rusqlite::types::Value::Integer(offset as i64));
            }
        }

        let sql = sql_parts.join(" ");

        let mut stmt = conn.prepare(&sql).map_err(|e| {
            StatisticsError::DatabaseError(format!("Failed to prepare query: {}", e))
        })?;

        // 使用 rusqlite 的 slice 参数 - 将 Value 转换为 &dyn ToSql
        let param_refs: Vec<&dyn rusqlite::ToSql> = param_values
            .iter()
            .map(|v| v as &dyn rusqlite::ToSql)
            .collect();

        let rows = stmt
            .query_map(param_refs.as_slice(), Self::map_row_to_event)
            .map_err(|e| {
                StatisticsError::DatabaseError(format!("Failed to query events: {}", e))
            })?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row.map_err(|e| {
                StatisticsError::DatabaseError(format!("Failed to read row: {}", e))
            })?);
        }

        Ok(events)
    }

    /// 将数据库行映射到 RoutingEvent
    fn map_row_to_event(row: &rusqlite::Row) -> rusqlite::Result<RoutingEvent> {
        Ok(RoutingEvent {
            timestamp: row.get(0)?,
            remote_addr: row.get(1)?,
            remote_port: row.get(2)?,
            method: row.get(3)?,
            path: row.get(4)?,
            input_port: row.get(5)?,
            model: row.get(6)?,
            routing_path: row.get(7)?,
            backend: row.get(8)?,
            success: row.get::<_, i32>(9)? == 1,
            duration_ms: row.get(10)?,
            error_type: row.get(11)?,
            request_size: row.get(12)?,
            response_size: row.get(13)?,
        })
    }

    /// 计算聚合统计
    pub fn compute_aggregation(&self, query: &AggQuery) -> Result<Vec<AggStats>> {
        let events = self.query_events_internal(&EventFilter {
            start_time: Some(query.start_time),
            end_time: Some(query.end_time),
            model: query.model.clone(),
            backend: query.backend.clone(),
            success: None,
            limit: None,
            offset: None,
        })?;

        Ok(crate::aggregator::Aggregator::aggregate(
            &events,
            query.window_size,
        ))
    }

    /// 从聚合表查询
    pub fn query_aggregated_table(&self, query: &AggQuery) -> Result<Vec<AggStats>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StatisticsError::DatabaseError(format!("Mutex poisoned: {e}")))?;

        // 构建参数数组
        let mut sql_parts = Vec::new();
        let mut param_values: Vec<rusqlite::types::Value> = Vec::new();

        sql_parts.push(
            "SELECT window_start, window_size, model, backend,
                    total_requests, success_count, fail_count,
                    avg_duration_ms, min_duration_ms, max_duration_ms,
                    p50_duration_ms, p90_duration_ms, p99_duration_ms
             FROM aggregated_stats WHERE 1=1"
                .to_string(),
        );

        // 计算窗口起始范围
        let window_size = query.window_size.get() as i64;
        let start_window = (query.start_time / 1000 / window_size) * window_size * 1000;
        let end_window = (query.end_time / 1000 / window_size) * window_size * 1000;

        sql_parts.push("AND window_start >= ? AND window_start <= ?".to_string());
        param_values.push(rusqlite::types::Value::Integer(start_window));
        param_values.push(rusqlite::types::Value::Integer(end_window));

        sql_parts.push("AND window_size = ?".to_string());
        param_values.push(rusqlite::types::Value::Integer(window_size));

        if let Some(ref model) = query.model {
            sql_parts.push("AND model = ?".to_string());
            param_values.push(rusqlite::types::Value::Text(model.clone()));
        }

        if let Some(ref backend) = query.backend {
            sql_parts.push("AND backend = ?".to_string());
            param_values.push(rusqlite::types::Value::Text(backend.clone()));
        }

        sql_parts.push("ORDER BY window_start ASC".to_string());

        let sql = sql_parts.join(" ");

        let mut stmt = conn.prepare(&sql).map_err(|e| {
            StatisticsError::DatabaseError(format!("Failed to prepare query: {}", e))
        })?;

        // 使用 rusqlite 的 slice 参数 - 将 Value 转换为 &dyn ToSql
        let param_refs: Vec<&dyn rusqlite::ToSql> = param_values
            .iter()
            .map(|v| v as &dyn rusqlite::ToSql)
            .collect();

        let rows = stmt
            .query_map(param_refs.as_slice(), Self::map_row_to_agg_stats)
            .map_err(|e| {
                StatisticsError::DatabaseError(format!("Failed to query aggregated stats: {}", e))
            })?;

        let mut stats = Vec::new();
        for row in rows {
            stats.push(row.map_err(|e| {
                StatisticsError::DatabaseError(format!("Failed to read row: {}", e))
            })?);
        }

        Ok(stats)
    }

    /// 将数据库行映射到 AggStats
    fn map_row_to_agg_stats(row: &rusqlite::Row) -> rusqlite::Result<AggStats> {
        Ok(AggStats {
            window_start: row.get(0)?,
            window_size: row.get(1)?,
            model: row.get(2)?,
            backend: row.get(3)?,
            total_requests: row.get(4)?,
            success_count: row.get(5)?,
            fail_count: row.get(6)?,
            avg_duration_ms: row.get(7)?,
            min_duration_ms: row.get(8)?,
            max_duration_ms: row.get(9)?,
            p50_duration_ms: row.get(10)?,
            p90_duration_ms: row.get(11)?,
            p99_duration_ms: row.get(12)?,
        })
    }

    /// 插入或更新聚合统计
    ///
    /// 用于后台聚合任务的预计算结果存储
    /// TODO: 实现后台聚合任务后使用此方法
    #[allow(dead_code)]
    fn upsert_aggregated_stats(&self, stats: &[AggStats]) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StatisticsError::DatabaseError(format!("Mutex poisoned: {e}")))?;

        let mut stmt = conn
            .prepare_cached(
                "
            INSERT INTO aggregated_stats (
                window_start, window_size, model, backend,
                total_requests, success_count, fail_count,
                avg_duration_ms, min_duration_ms, max_duration_ms,
                p50_duration_ms, p90_duration_ms, p99_duration_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ON CONFLICT(window_start, window_size, model, backend) DO UPDATE SET
                total_requests = excluded.total_requests,
                success_count = excluded.success_count,
                fail_count = excluded.fail_count,
                avg_duration_ms = excluded.avg_duration_ms,
                min_duration_ms = excluded.min_duration_ms,
                max_duration_ms = excluded.max_duration_ms,
                p50_duration_ms = excluded.p50_duration_ms,
                p90_duration_ms = excluded.p90_duration_ms,
                p99_duration_ms = excluded.p99_duration_ms
            ",
            )
            .map_err(|e| {
                StatisticsError::DatabaseError(format!("Failed to prepare upsert: {}", e))
            })?;

        for stat in stats {
            stmt.execute(params![
                stat.window_start,
                stat.window_size,
                stat.model,
                stat.backend,
                stat.total_requests,
                stat.success_count,
                stat.fail_count,
                stat.avg_duration_ms,
                stat.min_duration_ms,
                stat.max_duration_ms,
                stat.p50_duration_ms,
                stat.p90_duration_ms,
                stat.p99_duration_ms,
            ])
            .map_err(|e| {
                StatisticsError::DatabaseError(format!("Failed to upsert stats: {}", e))
            })?;
        }

        Ok(())
    }

    /// 清理过期数据
    pub fn cleanup_old_internal(&self, before: i64) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StatisticsError::DatabaseError(format!("Mutex poisoned: {}", e)))?;

        // 删除 events 表中的旧数据
        let deleted_events = conn
            .execute("DELETE FROM events WHERE timestamp < ?", [before])
            .map_err(|e| {
                StatisticsError::DatabaseError(format!("Failed to cleanup events: {}", e))
            })?;

        // 删除 aggregated_stats 表中的旧数据
        let deleted_stats = conn
            .execute(
                "DELETE FROM aggregated_stats WHERE window_start < ?",
                [before],
            )
            .map_err(|e| {
                StatisticsError::DatabaseError(format!("Failed to cleanup aggregated_stats: {}", e))
            })?;

        Ok(deleted_events + deleted_stats)
    }

    /// 获取事件总数
    pub fn count_events_internal(&self) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StatisticsError::DatabaseError(format!("Mutex poisoned: {}", e)))?;

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .map_err(|e| {
                StatisticsError::DatabaseError(format!("Failed to count events: {}", e))
            })?;

        Ok(count)
    }
}

impl SqliteStore {
    /// 记录单个事件
    pub async fn record_event(&self, event: &RoutingEvent) -> Result<()> {
        // 在后台任务中执行，不阻塞异步运行时
        let store = self.clone();
        let event = event.clone();
        tokio::task::spawn_blocking(move || store.insert_event(&event))
            .await
            .map_err(|e| StatisticsError::DatabaseError(format!("Spawn failed: {}", e)))??;
        Ok(())
    }

    /// 查询原始事件
    pub async fn query_events(&self, filter: EventFilter) -> Result<Vec<RoutingEvent>> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || store.query_events_internal(&filter))
            .await
            .map_err(|e| StatisticsError::DatabaseError(format!("Spawn failed: {}", e)))?
    }

    /// 获取聚合统计
    pub async fn get_aggregated_stats(&self, query: AggQuery) -> Result<Vec<AggStats>> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            // 先尝试从聚合表查询
            let stats = store.query_aggregated_table(&query)?;
            if !stats.is_empty() {
                return Ok(stats);
            }

            // 如果没有预计算数据，实时计算
            store.compute_aggregation(&query)
        })
        .await
        .map_err(|e| StatisticsError::DatabaseError(format!("Spawn failed: {}", e)))?
    }

    /// 清理过期数据
    pub async fn cleanup_old(&self, before: i64) -> Result<usize> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || store.cleanup_old_internal(before))
            .await
            .map_err(|e| StatisticsError::DatabaseError(format!("Spawn failed: {}", e)))?
    }

    /// 获取事件总数
    pub async fn count_events(&self) -> Result<i64> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || store.count_events_internal())
            .await
            .map_err(|e| StatisticsError::DatabaseError(format!("Spawn failed: {}", e)))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_schema() {
        let store = SqliteStore::in_memory().unwrap();
        let conn = store.conn.lock().unwrap();

        // 验证表存在
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap();
        let names: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(names.contains(&"events".to_string()));
        assert!(names.contains(&"aggregated_stats".to_string()));
    }
}
