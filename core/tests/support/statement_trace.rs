//! The statement hook for the tests that count, or read, the statements a call issues.

use rusqlite::trace::{TraceEvent, TraceEventCodes};
use rusqlite::Connection;

/// Calls `record` with each statement `conn` starts from now on; `None` removes the hook.
///
/// The text is what SQLite's legacy trace hook passed: a top-level statement with its bound
/// values expanded, so a test can read `project_id = 7`; a trigger's own line as SQLite wrote
/// it. The events are the legacy hook's too — one per statement start, one per trigger.
pub(crate) fn trace(conn: &mut Connection, record: Option<fn(&str)>) {
    let hook = record.map(|on_statement| {
        move |event: TraceEvent<'_>| {
            if let TraceEvent::Stmt(stmt, sql) = event {
                if sql == stmt.sql() {
                    on_statement(&stmt.expanded_sql().unwrap_or_else(|| sql.to_owned()));
                } else {
                    on_statement(sql);
                }
            }
        }
    });
    conn.trace_v2(TraceEventCodes::SQLITE_TRACE_STMT, hook)
        .expect("SQLite accepts the statement hook");
}
