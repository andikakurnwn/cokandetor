//! Database connections ("storage cabinets" in the office). Each cabinet owns
//! a worker thread holding one live connection; the UI talks to it over channels.

use serde::{Deserialize, Serialize};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

pub const MAX_ROWS: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DbKind {
    Postgres,
    MySql,
    Sqlite,
}

impl DbKind {
    pub const ALL: [DbKind; 3] = [Self::Postgres, Self::MySql, Self::Sqlite];

    pub fn label(self) -> &'static str {
        match self {
            Self::Postgres => "PostgreSQL",
            Self::MySql => "MySQL / MariaDB",
            Self::Sqlite => "SQLite (file)",
        }
    }

    /// Two-letter code painted on the cabinet.
    pub fn code(self) -> &'static str {
        match self {
            Self::Postgres => "PG",
            Self::MySql => "MY",
            Self::Sqlite => "SQ",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Self::Postgres => 5432,
            Self::MySql => 3306,
            Self::Sqlite => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SslMode {
    Disable,
    Prefer,
    Require,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DbConfig {
    pub name: String,
    pub kind: DbKind,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
    /// SQLite database file.
    pub file: String,
    pub ssl: SslMode,
    /// Accept self-signed / unverified certificates.
    pub trust_cert: bool,
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            name: "Database".into(),
            kind: DbKind::Postgres,
            host: "localhost".into(),
            port: 5432,
            user: "postgres".into(),
            password: String::new(),
            database: "postgres".into(),
            file: String::new(),
            ssl: SslMode::Prefer,
            trust_cert: false,
        }
    }
}

impl DbConfig {
    pub fn target(&self) -> String {
        match self.kind {
            DbKind::Sqlite => self.file.clone(),
            _ => format!("{}@{}:{}/{}", self.user, self.host, self.port, self.database),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    pub affected: Option<u64>,
    pub truncated: bool,
    pub elapsed_ms: u128,
}

#[derive(Clone, Debug)]
pub struct TableInfo {
    pub schema: String,
    pub name: String,
    pub is_view: bool,
}

impl TableInfo {
    /// Quoted, schema-qualified name for SQL.
    pub fn qualified(&self, kind: DbKind) -> String {
        match kind {
            DbKind::MySql => format!("`{}`", self.name.replace('`', "``")),
            _ if self.schema.is_empty() || self.schema == "main" => format!("\"{}\"", self.name.replace('"', "\"\"")),
            _ => format!(
                "\"{}\".\"{}\"",
                self.schema.replace('"', "\"\""),
                self.name.replace('"', "\"\"")
            ),
        }
    }

    pub fn key(&self) -> String {
        format!("{}.{}", self.schema, self.name)
    }
}

#[derive(Clone, Debug)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

pub enum Request {
    Connect,
    Query(String),
    Tables,
    Columns(TableInfo),
}

pub enum Response {
    Connected(String),
    Tables(Vec<TableInfo>),
    Columns(String, Vec<ColumnInfo>),
    Result(QueryResult),
    Error(String),
    /// A queued request dropped because the connection could not be made.
    Skipped,
}

trait Backend {
    fn query(&mut self, sql: &str) -> Result<QueryResult, String>;
    fn version(&mut self) -> String;
    /// Is the connection still usable after an error?
    fn alive(&mut self) -> bool;
}

// ---------------------------------------------------------------- PostgreSQL

struct Pg {
    // A small runtime drives the tokio-postgres connection while we wait.
    rt: tokio::runtime::Runtime,
    client: tokio_postgres::Client,
}

fn connect_pg(c: &DbConfig) -> Result<Pg, String> {
    use tokio_postgres::config::SslMode as PgSsl;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?;
    let mut cfg = tokio_postgres::Config::new();
    cfg.host(&c.host)
        .port(c.port)
        .user(&c.user)
        .dbname(&c.database)
        .connect_timeout(Duration::from_secs(10))
        .application_name("cokandetor");
    if !c.password.is_empty() {
        cfg.password(&c.password);
    }
    let client = rt.block_on(async {
        if c.ssl == SslMode::Disable {
            cfg.ssl_mode(PgSsl::Disable);
            let (client, conn) = cfg.connect(tokio_postgres::NoTls).await.map_err(|e| pg_err(&e))?;
            tokio::spawn(async move {
                let _ = conn.await;
            });
            Ok::<_, String>(client)
        } else {
            cfg.ssl_mode(if c.ssl == SslMode::Require { PgSsl::Require } else { PgSsl::Prefer });
            // Like psql: "prefer" encrypts without verifying; "require" verifies
            // unless the user chose to trust the server certificate.
            let skip_verify = c.ssl == SslMode::Prefer || c.trust_cert;
            let tls = native_tls::TlsConnector::builder()
                .danger_accept_invalid_certs(skip_verify)
                .danger_accept_invalid_hostnames(skip_verify)
                .build()
                .map_err(|e| format!("TLS: {e}"))?;
            let (client, conn) = cfg
                .connect(postgres_native_tls::MakeTlsConnector::new(tls))
                .await
                .map_err(|e| pg_err(&e))?;
            tokio::spawn(async move {
                let _ = conn.await;
            });
            Ok(client)
        }
    })?;
    Ok(Pg { rt, client })
}

fn pg_err(e: &tokio_postgres::Error) -> String {
    match e.as_db_error() {
        Some(db) => format!("{}: {}", db.severity(), db.message()),
        None => {
            let mut s = e.to_string();
            let mut src = std::error::Error::source(e);
            while let Some(inner) = src {
                s.push_str(&format!(" — {inner}"));
                src = inner.source();
            }
            s
        }
    }
}

impl Backend for Pg {
    fn query(&mut self, sql: &str) -> Result<QueryResult, String> {
        use futures_util::{StreamExt, pin_mut};
        use tokio_postgres::SimpleQueryMessage as M;
        let client = &self.client;
        self.rt.block_on(async {
            // Stream the rows so a huge SELECT never lands in memory at once.
            let stream = client.simple_query_raw(sql).await.map_err(|e| pg_err(&e))?;
            pin_mut!(stream);
            // Keep the last statement that returned columns, else the last command.
            let mut out = QueryResult::default();
            let mut cur = QueryResult::default();
            while let Some(m) = stream.next().await {
                match m.map_err(|e| pg_err(&e))? {
                    M::RowDescription(cols) => {
                        cur = QueryResult {
                            columns: cols.iter().map(|c| c.name().to_string()).collect(),
                            ..Default::default()
                        };
                    }
                    M::Row(row) => {
                        if cur.columns.is_empty() {
                            cur.columns = row.columns().iter().map(|c| c.name().to_string()).collect();
                        }
                        if cur.rows.len() >= MAX_ROWS {
                            // Stop reading; the connection discards the rest.
                            cur.truncated = true;
                            return Ok(cur);
                        }
                        cur.rows.push((0..row.len()).map(|i| row.get(i).map(str::to_string)).collect());
                    }
                    M::CommandComplete(n) => {
                        cur.affected = Some(n);
                        if !cur.columns.is_empty() || out.columns.is_empty() {
                            out = std::mem::take(&mut cur);
                        }
                    }
                    _ => {}
                }
            }
            Ok(out)
        })
    }

    fn version(&mut self) -> String {
        self.query("SELECT version()")
            .ok()
            .and_then(|r| r.rows.first().and_then(|row| row.first().cloned().flatten()))
            .unwrap_or_else(|| "PostgreSQL".into())
    }

    fn alive(&mut self) -> bool {
        !self.client.is_closed()
    }
}

// ---------------------------------------------------------------- MySQL

struct My(mysql::Conn);

fn connect_my(c: &DbConfig) -> Result<My, String> {
    let mut opts = mysql::OptsBuilder::new()
        .ip_or_hostname(Some(c.host.clone()))
        .tcp_port(c.port)
        .user(Some(c.user.clone()))
        .db_name((!c.database.is_empty()).then(|| c.database.clone()))
        .tcp_connect_timeout(Some(Duration::from_secs(10)));
    if !c.password.is_empty() {
        opts = opts.pass(Some(c.password.clone()));
    }
    if c.ssl != SslMode::Disable {
        let skip_verify = c.ssl == SslMode::Prefer || c.trust_cert;
        opts = opts.ssl_opts(mysql::SslOpts::default().with_danger_accept_invalid_certs(skip_verify));
    }
    match mysql::Conn::new(opts.clone()) {
        Ok(conn) => Ok(My(conn)),
        // "Prefer" falls back to plain text only when the server has no TLS.
        Err(e) if c.ssl == SslMode::Prefer && matches!(e, mysql::Error::DriverError(mysql::DriverError::TlsNotSupported)) => {
            mysql::Conn::new(opts.ssl_opts(None)).map(My).map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

fn my_value(v: &mysql::Value) -> Option<String> {
    use mysql::Value::*;
    Some(match v {
        NULL => return None,
        Bytes(b) => String::from_utf8_lossy(b).into_owned(),
        Int(i) => i.to_string(),
        UInt(u) => u.to_string(),
        Float(f) => f.to_string(),
        Double(d) => d.to_string(),
        Date(y, m, d, h, mi, s, us) => {
            if *us > 0 {
                format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}.{us:06}")
            } else {
                format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}")
            }
        }
        Time(neg, days, h, mi, s, _) => {
            format!("{}{:02}:{mi:02}:{s:02}", if *neg { "-" } else { "" }, *days * 24 + *h as u32)
        }
    })
}

impl Backend for My {
    fn query(&mut self, sql: &str) -> Result<QueryResult, String> {
        use mysql::prelude::Queryable;
        let mut result = self.0.query_iter(sql).map_err(|e| e.to_string())?;
        let mut out = QueryResult::default();
        while let Some(set) = result.iter() {
            let columns: Vec<String> = set.columns().as_ref().iter().map(|c| c.name_str().into_owned()).collect();
            let affected = set.affected_rows();
            let mut cur = QueryResult {
                columns,
                affected: Some(affected),
                ..Default::default()
            };
            for row in set {
                let row = row.map_err(|e| e.to_string())?;
                if cur.rows.len() < MAX_ROWS {
                    cur.rows.push((0..row.len()).map(|i| row.as_ref(i).and_then(my_value)).collect());
                } else {
                    cur.truncated = true;
                }
            }
            if !cur.columns.is_empty() || out.columns.is_empty() {
                if !cur.columns.is_empty() {
                    cur.affected = Some(cur.rows.len() as u64);
                }
                out = cur;
            }
        }
        Ok(out)
    }

    fn version(&mut self) -> String {
        let (a, b, c) = self.0.server_version();
        format!("MySQL {a}.{b}.{c}")
    }

    fn alive(&mut self) -> bool {
        self.0.ping().is_ok()
    }
}

// ---------------------------------------------------------------- SQLite

struct Lite(rusqlite::Connection);

fn connect_lite(c: &DbConfig) -> Result<Lite, String> {
    if c.file.trim().is_empty() {
        return Err("Pilih file database SQLite.".into());
    }
    let path = std::path::Path::new(c.file.trim());
    if !path.is_file() {
        return Err(format!("File database tidak ditemukan: {}", path.display()));
    }
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
        | rusqlite::OpenFlags::SQLITE_OPEN_URI
        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = rusqlite::Connection::open_with_flags(path, flags).map_err(|e| e.to_string())?;
    let _ = conn.busy_timeout(Duration::from_secs(5));
    Ok(Lite(conn))
}

impl Backend for Lite {
    fn query(&mut self, sql: &str) -> Result<QueryResult, String> {
        use rusqlite::fallible_iterator::FallibleIterator;
        use rusqlite::types::ValueRef;
        let mut batch = rusqlite::Batch::new(&self.0, sql);
        let mut out = QueryResult::default();
        while let Some(mut stmt) = batch.next().map_err(|e| e.to_string())? {
            let columns: Vec<String> = stmt.column_names().map(str::to_string).collect();
            if columns.is_empty() {
                let n = stmt.raw_execute().map_err(|e| e.to_string())?;
                if out.columns.is_empty() {
                    out = QueryResult {
                        affected: Some(n as u64),
                        ..Default::default()
                    };
                }
                continue;
            }
            let n = columns.len();
            let mut cur = QueryResult {
                columns,
                ..Default::default()
            };
            let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                if cur.rows.len() >= MAX_ROWS {
                    cur.truncated = true;
                    break;
                }
                let mut vals = Vec::with_capacity(n);
                for i in 0..n {
                    vals.push(match row.get_ref(i).map_err(|e| e.to_string())? {
                        ValueRef::Null => None,
                        ValueRef::Integer(v) => Some(v.to_string()),
                        ValueRef::Real(v) => Some(v.to_string()),
                        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
                        ValueRef::Blob(b) => Some(format!("<blob {} bytes>", b.len())),
                    });
                }
                cur.rows.push(vals);
            }
            cur.affected = Some(cur.rows.len() as u64);
            out = cur;
        }
        Ok(out)
    }

    fn version(&mut self) -> String {
        format!("SQLite {}", rusqlite::version())
    }

    fn alive(&mut self) -> bool {
        true
    }
}

// ---------------------------------------------------------------- worker

fn connect(c: &DbConfig) -> Result<Box<dyn Backend>, String> {
    Ok(match c.kind {
        DbKind::Postgres => Box::new(connect_pg(c)?),
        DbKind::MySql => Box::new(connect_my(c)?),
        DbKind::Sqlite => Box::new(connect_lite(c)?),
    })
}

fn sql_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn tables_sql(kind: DbKind) -> &'static str {
    match kind {
        DbKind::Postgres => {
            "SELECT table_schema, table_name, table_type FROM information_schema.tables \
             WHERE table_schema NOT IN ('pg_catalog', 'information_schema') ORDER BY 1, 2"
        }
        DbKind::MySql => {
            "SELECT table_schema, table_name, table_type FROM information_schema.tables \
             WHERE table_schema = DATABASE() ORDER BY 2"
        }
        DbKind::Sqlite => {
            "SELECT 'main', name, type FROM sqlite_master \
             WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY name"
        }
    }
}

fn columns_sql(kind: DbKind, t: &TableInfo) -> String {
    match kind {
        DbKind::Postgres => format!(
            "SELECT column_name, data_type, is_nullable FROM information_schema.columns \
             WHERE table_schema = {} AND table_name = {} ORDER BY ordinal_position",
            sql_str(&t.schema),
            sql_str(&t.name)
        ),
        DbKind::MySql => format!(
            "SELECT column_name, column_type, is_nullable FROM information_schema.columns \
             WHERE table_schema = DATABASE() AND table_name = {} ORDER BY ordinal_position",
            sql_str(&t.name)
        ),
        DbKind::Sqlite => format!(
            "SELECT name, type, CASE \"notnull\" WHEN 1 THEN 'NO' ELSE 'YES' END FROM pragma_table_info({})",
            sql_str(&t.name)
        ),
    }
}

pub struct Worker {
    tx: Sender<Request>,
    pub rx: Receiver<Response>,
}

impl Worker {
    pub fn spawn(cfg: DbConfig, ctx: egui::Context) -> Self {
        let (tx, req_rx) = channel::<Request>();
        let (resp_tx, rx) = channel::<Response>();
        std::thread::spawn(move || {
            let mut conn: Option<Box<dyn Backend>> = None;
            let send = |r: Response| {
                let _ = resp_tx.send(r);
                ctx.request_repaint();
            };
            while let Ok(req) = req_rx.recv() {
                if conn.is_none() || matches!(req, Request::Connect) {
                    match connect(&cfg) {
                        Ok(c) => conn = Some(c),
                        Err(e) => {
                            send(Response::Error(format!("Gagal terhubung: {e}")));
                            // Don't retry the connection once per queued request.
                            while req_rx.try_recv().is_ok() {
                                send(Response::Skipped);
                            }
                            continue;
                        }
                    }
                }
                let Some(c) = conn.as_mut() else { continue };
                let resp = match req {
                    Request::Connect => Response::Connected(c.version()),
                    Request::Query(sql) => {
                        let t = Instant::now();
                        match c.query(&sql) {
                            Ok(mut r) => {
                                r.elapsed_ms = t.elapsed().as_millis();
                                Response::Result(r)
                            }
                            Err(e) => Response::Error(e),
                        }
                    }
                    Request::Tables => match c.query(tables_sql(cfg.kind)) {
                        Ok(r) => Response::Tables(
                            r.rows
                                .into_iter()
                                .map(|row| {
                                    let get = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
                                    TableInfo {
                                        schema: get(0),
                                        name: get(1),
                                        is_view: get(2).to_uppercase().contains("VIEW"),
                                    }
                                })
                                .collect(),
                        ),
                        Err(e) => Response::Error(e),
                    },
                    Request::Columns(t) => match c.query(&columns_sql(cfg.kind, &t)) {
                        Ok(r) => Response::Columns(
                            t.key(),
                            r.rows
                                .into_iter()
                                .map(|row| {
                                    let get = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
                                    ColumnInfo {
                                        name: get(0),
                                        data_type: get(1),
                                        nullable: get(2).eq_ignore_ascii_case("YES"),
                                    }
                                })
                                .collect(),
                        ),
                        Err(e) => Response::Error(e),
                    },
                };
                // Only a dead connection is replaced: a plain SQL error must not
                // lose the session (open transaction, temp tables, variables).
                if matches!(resp, Response::Error(_)) && !c.alive() {
                    conn = None;
                }
                send(resp);
            }
        });
        Self { tx, rx }
    }

    pub fn send(&self, r: Request) {
        let _ = self.tx.send(r);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Idle,
    Busy,
    Connected,
    Error,
}

/// UI-side state of one cabinet.
pub struct DbState {
    pub cfg: DbConfig,
    pub worker: Option<Worker>,
    pub status: Status,
    pub version: String,
    pub tables: Vec<TableInfo>,
    pub columns: std::collections::HashMap<String, Vec<ColumnInfo>>,
    pub expanded: std::collections::HashSet<String>,
    pub sql: String,
    pub result: Option<QueryResult>,
    pub error: Option<String>,
    pub pending: usize,
    pub history: Vec<String>,
    /// Did the most recent response fail?
    failed: bool,
}

pub fn default_sql(kind: DbKind) -> String {
    match kind {
        DbKind::Postgres => "-- Ctrl+Enter untuk menjalankan\nSELECT now(), current_database(), current_user;\n".into(),
        DbKind::MySql => "-- Ctrl+Enter untuk menjalankan\nSELECT NOW(), DATABASE(), CURRENT_USER();\n".into(),
        DbKind::Sqlite => "-- Ctrl+Enter untuk menjalankan\nSELECT sqlite_version(), datetime('now');\n".into(),
    }
}

impl DbState {
    pub fn new(cfg: DbConfig) -> Self {
        let sql = default_sql(cfg.kind);
        Self {
            cfg,
            worker: None,
            status: Status::Idle,
            version: String::new(),
            tables: Vec::new(),
            columns: Default::default(),
            expanded: Default::default(),
            sql,
            result: None,
            error: None,
            pending: 0,
            history: Vec::new(),
            failed: false,
        }
    }

    fn request(&mut self, ctx: &egui::Context, r: Request) {
        if self.worker.is_none() {
            self.worker = Some(Worker::spawn(self.cfg.clone(), ctx.clone()));
        }
        if let Some(w) = &self.worker {
            w.send(r);
            self.pending += 1;
            self.status = Status::Busy;
        }
    }

    pub fn connect(&mut self, ctx: &egui::Context) {
        // A fresh worker picks up edited settings.
        self.worker = None;
        self.pending = 0;
        self.error = None;
        self.request(ctx, Request::Connect);
        self.request(ctx, Request::Tables);
    }

    pub fn disconnect(&mut self) {
        self.worker = None;
        self.pending = 0;
        self.status = Status::Idle;
        self.tables.clear();
        self.columns.clear();
    }

    pub fn run(&mut self, ctx: &egui::Context, sql: String) {
        if sql.trim().is_empty() {
            return;
        }
        if self.history.last() != Some(&sql) {
            self.history.push(sql.clone());
            if self.history.len() > 50 {
                self.history.remove(0);
            }
        }
        self.error = None;
        self.result = None;
        self.request(ctx, Request::Query(sql));
    }

    pub fn refresh_tables(&mut self, ctx: &egui::Context) {
        self.request(ctx, Request::Tables);
    }

    pub fn load_columns(&mut self, ctx: &egui::Context, t: TableInfo) {
        self.request(ctx, Request::Columns(t));
    }

    pub fn poll(&mut self) {
        let mut got = Vec::new();
        if let Some(w) = &self.worker {
            while let Ok(r) = w.rx.try_recv() {
                got.push(r);
            }
        }
        self.apply(got);
    }

    fn apply(&mut self, got: Vec<Response>) {
        let answered = !got.is_empty();
        for r in got {
            self.pending = self.pending.saturating_sub(1);
            match r {
                Response::Connected(v) => {
                    self.version = v;
                    self.failed = false;
                }
                Response::Tables(t) => {
                    self.tables = t;
                    self.failed = false;
                }
                Response::Columns(key, cols) => {
                    self.columns.insert(key, cols);
                    self.failed = false;
                }
                Response::Result(r) => {
                    self.result = Some(r);
                    self.error = None;
                    self.failed = false;
                }
                Response::Error(e) => {
                    self.error = Some(e);
                    self.failed = true;
                }
                Response::Skipped => {}
            }
        }
        if self.pending > 0 {
            self.status = Status::Busy;
        } else if answered || self.status == Status::Busy {
            self.status = if self.failed { Status::Error } else { Status::Connected };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_roundtrip() {
        let dir = std::env::temp_dir().join(format!("cokandetor-test-{}.db", std::process::id()));
        let cfg = DbConfig {
            kind: DbKind::Sqlite,
            file: dir.to_string_lossy().into_owned(),
            ..Default::default()
        };
        assert!(connect(&cfg).is_err(), "a missing file must not be created");
        drop(rusqlite::Connection::open(&dir).unwrap());
        let mut c = connect(&cfg).unwrap();
        c.query("CREATE TABLE IF NOT EXISTS t(id INTEGER, name TEXT); DELETE FROM t; INSERT INTO t VALUES (1, 'a'), (2, NULL);")
            .unwrap();
        let r = c.query("SELECT id, name FROM t ORDER BY id").unwrap();
        assert_eq!(r.columns, vec!["id", "name"]);
        assert_eq!(r.rows, vec![vec![Some("1".into()), Some("a".into())], vec![Some("2".into()), None]]);
        let tables = c.query(tables_sql(DbKind::Sqlite)).unwrap();
        assert_eq!(tables.rows[0][1].as_deref(), Some("t"));
        let cols = c.query(&columns_sql(DbKind::Sqlite, &TableInfo { schema: "main".into(), name: "t".into(), is_view: false })).unwrap();
        assert_eq!(cols.rows.len(), 2);
        drop(c);
        let _ = std::fs::remove_file(dir);
    }

    #[test]
    fn status_settles_after_column_lookup() {
        let mut d = DbState::new(DbConfig::default());
        d.status = Status::Busy;
        d.pending = 1;
        d.apply(vec![Response::Columns("public.t".into(), vec![])]);
        assert_eq!(d.status, Status::Connected);
        d.pending = 2;
        d.apply(vec![Response::Error("boom".into()), Response::Skipped]);
        assert_eq!(d.status, Status::Error);
        assert_eq!(d.pending, 0);
    }
}
