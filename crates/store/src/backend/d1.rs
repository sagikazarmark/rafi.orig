//! Cloudflare D1.
//!
//! D1 hands every number to JavaScript as a float, so integers are exact up to
//! 2^53; rafi stores unix seconds and small counters, which fit. A batch runs
//! as one transaction.

use std::{future::Future, sync::Arc};

use js_sys::{Array, Object};
use wasm_bindgen::{JsCast, JsValue};
use worker::{D1Database, D1PreparedStatement, send::SendFuture};

use crate::{
    error::{Error, Result},
    query::{Db, Statement},
    value::{Row, Value},
};

#[derive(Clone)]
pub(crate) struct D1 {
    db: Arc<D1Database>,
}

impl Db {
    /// Wraps a D1 binding, for example `env.d1("DB")?`. Migrations run with
    /// `wrangler d1 migrations apply`, from the same files as the native build.
    pub fn d1(db: D1Database) -> Self {
        Self {
            backend: D1 { db: Arc::new(db) },
        }
    }
}

fn err(e: impl std::fmt::Display) -> Error {
    Error::from_message(e.to_string())
}

/// A rejected JavaScript promise: D1 throws `Error`s whose message carries the
/// SQLite error (`D1_ERROR: UNIQUE constraint failed: ...`).
fn js_err(e: JsValue) -> Error {
    let msg = e
        .dyn_ref::<js_sys::Error>()
        .map(|e| String::from(e.message()))
        .or_else(|| e.as_string())
        .unwrap_or_else(|| format!("{e:?}"));
    Error::from_message(msg)
}

fn js(value: &Value) -> JsValue {
    match value {
        Value::Null => JsValue::null(),
        Value::Int(i) => JsValue::from_f64(*i as f64),
        Value::Real(f) => JsValue::from_f64(*f),
        Value::Text(s) => JsValue::from_str(s),
    }
}

fn from_js(value: JsValue) -> Value {
    if value.is_null() || value.is_undefined() {
        Value::Null
    } else if let Some(f) = value.as_f64() {
        if f.fract() == 0.0 && f.abs() < 9_007_199_254_740_992.0 {
            Value::Int(f as i64)
        } else {
            Value::Real(f)
        }
    } else if let Some(b) = value.as_bool() {
        Value::Int(b.into())
    } else if let Some(s) = value.as_string() {
        Value::Text(s)
    } else {
        // BLOBs come back as arrays of bytes; rafi stores none.
        Value::Text(format!("{value:?}"))
    }
}

/// The backend's operations, as `Send` futures. D1's own futures hold
/// JavaScript values and aren't `Send`, but a Worker runs on one thread, so
/// nothing can actually move them across threads; this keeps [`Db`]'s futures
/// `Send` on every target, as axum and the Restate SDK require.
impl D1 {
    pub(crate) fn all(
        &self,
        stmt: Statement,
    ) -> impl Future<Output = Result<Vec<Row>>> + Send + '_ {
        SendFuture::new(self.all_js(stmt))
    }

    pub(crate) fn run(&self, stmt: Statement) -> impl Future<Output = Result<u64>> + Send + '_ {
        SendFuture::new(self.run_js(stmt))
    }

    pub(crate) fn batch(
        &self,
        stmts: Vec<Statement>,
    ) -> impl Future<Output = Result<Vec<u64>>> + Send + '_ {
        SendFuture::new(self.batch_js(stmts))
    }

    pub(crate) fn batch_rows(
        &self,
        stmts: Vec<Statement>,
    ) -> impl Future<Output = Result<Vec<Vec<Row>>>> + Send + '_ {
        SendFuture::new(self.batch_rows_js(stmts))
    }
}

impl D1 {
    fn prepare(&self, stmt: &Statement) -> Result<D1PreparedStatement> {
        let params: Vec<JsValue> = stmt.params.iter().map(js).collect();
        self.db
            .prepare(stmt.sql.as_str())
            .bind(&params)
            .map_err(err)
    }

    /// Reads the JavaScript result directly: `D1Result::results` decodes
    /// through serde, which loses the column order tuples rely on.
    async fn all_js(&self, stmt: Statement) -> Result<Vec<Row>> {
        let prepared = self.prepare(&stmt)?;
        let promise = prepared.inner().all().map_err(js_err)?;
        let result = wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .map_err(js_err)?;
        rows_of(result)
    }

    async fn run_js(&self, stmt: Statement) -> Result<u64> {
        let result = self.prepare(&stmt)?.run().await.map_err(err)?;
        changes(&result)
    }

    async fn batch_js(&self, stmts: Vec<Statement>) -> Result<Vec<u64>> {
        let prepared = stmts
            .iter()
            .map(|s| self.prepare(s))
            .collect::<Result<Vec<_>>>()?;
        let results = self.db.batch(prepared).await.map_err(err)?;
        results.iter().map(changes).collect()
    }
}

impl D1 {
    async fn batch_rows_js(&self, stmts: Vec<Statement>) -> Result<Vec<Vec<Row>>> {
        let prepared = stmts
            .iter()
            .map(|s| self.prepare(s))
            .collect::<Result<Vec<_>>>()?;
        let statements = Array::new();
        for p in &prepared {
            statements.push(p.inner());
        }
        // `D1Database::batch` hands back `D1Result`s, whose rows only decode
        // through serde; call `batch` on the binding to keep column order.
        let binding: &JsValue = (*self.db).as_ref();
        let batch = js_sys::Reflect::get(binding, &JsValue::from_str("batch"))
            .map_err(js_err)?
            .dyn_into::<js_sys::Function>()
            .map_err(|_| Error::Backend("D1 binding has no batch".into()))?;
        let promise: js_sys::Promise = batch
            .call1(binding, &statements)
            .map_err(js_err)?
            .dyn_into()
            .map_err(|_| Error::Backend("D1 batch did not return a promise".into()))?;
        let results: Array = wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .map_err(js_err)?
            .dyn_into()
            .map_err(|_| Error::Decode("D1 batch results are not an array".into()))?;
        results.iter().map(rows_of).collect()
    }
}

fn rows_of(result: JsValue) -> Result<Vec<Row>> {
    let rows = js_sys::Reflect::get(&result, &JsValue::from_str("results")).map_err(js_err)?;
    if rows.is_undefined() || rows.is_null() {
        return Ok(Vec::new());
    }
    let rows: Array = rows
        .dyn_into()
        .map_err(|_| Error::Decode("D1 results are not an array".into()))?;
    rows.iter().map(to_row).collect()
}

fn changes(result: &worker::D1Result) -> Result<u64> {
    if let Some(e) = result.error() {
        return Err(Error::from_message(e));
    }
    Ok(result
        .meta()
        .map_err(err)?
        .and_then(|m| m.changes)
        .unwrap_or(0) as u64)
}

/// A result object, in column order (JavaScript keeps insertion order).
fn to_row(object: JsValue) -> Result<Row> {
    let object: Object = object
        .dyn_into()
        .map_err(|_| Error::Decode("a D1 row is not an object".into()))?;
    let mut row = Row::default();
    for entry in Object::entries(&object).iter() {
        let pair: Array = entry.unchecked_into();
        row.columns
            .push(pair.get(0).as_string().unwrap_or_default());
        row.values.push(from_js(pair.get(1)));
    }
    Ok(row)
}
