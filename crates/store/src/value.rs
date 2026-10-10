//! SQL values, rows, and decoding rows into Rust types with serde.
//!
//! Both backends hand back rows as column names plus [`Value`]s, so decoding is
//! the same everywhere. A row decodes into a struct (by column name) or a tuple
//! (by position). SQLite has no boolean type, so `bool` fields decode from the
//! integers 0 and 1, as they are stored.

use serde::de::{
    self, DeserializeOwned, DeserializeSeed, Deserializer, IntoDeserializer, MapAccess, SeqAccess,
    Visitor,
};

use crate::error::{Error, Result};

/// A value bound to, or read from, a statement.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Self::Int(v)
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Self::Int(v.into())
    }
}

impl From<u32> for Value {
    fn from(v: u32) -> Self {
        Self::Int(v.into())
    }
}

impl From<u64> for Value {
    /// Values above `i64::MAX` saturate; rafi never stores any.
    fn from(v: u64) -> Self {
        Self::Int(i64::try_from(v).unwrap_or(i64::MAX))
    }
}

impl From<usize> for Value {
    fn from(v: usize) -> Self {
        Self::Int(i64::try_from(v).unwrap_or(i64::MAX))
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Self::Int(v.into())
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Self::Real(v)
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Self::Text(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Self::Text(v.to_owned())
    }
}

impl From<&String> for Value {
    fn from(v: &String) -> Self {
        Self::Text(v.clone())
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map_or(Self::Null, Into::into)
    }
}

/// One result row: column names and values, in select order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Row {
    pub columns: Vec<String>,
    pub values: Vec<Value>,
}

impl Row {
    /// Decodes the row into `T`: a struct by column name, or a tuple by position.
    pub fn decode<T: DeserializeOwned>(self) -> Result<T> {
        T::deserialize(RowDe(self)).map_err(|e| Error::Decode(e.0))
    }
}

#[derive(Debug)]
pub struct DeError(String);

impl std::fmt::Display for DeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DeError {}

impl de::Error for DeError {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        Self(msg.to_string())
    }
}

struct RowDe(Row);

impl<'de> Deserializer<'de> for RowDe {
    type Error = DeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_map(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_map(RowMap {
            columns: self.0.columns.into_iter(),
            values: self.0.values.into_iter(),
            pending: None,
        })
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DeError> {
        self.deserialize_map(visitor)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_seq(RowSeq(self.0.values.into_iter()))
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        self.deserialize_seq(visitor)
    }

    /// A single-column row decodes as its only value (`SELECT COUNT(*)`).
    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        visitor.visit_newtype_struct(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct enum identifier ignored_any
    }
}

struct RowMap {
    columns: std::vec::IntoIter<String>,
    values: std::vec::IntoIter<Value>,
    pending: Option<Value>,
}

impl<'de> MapAccess<'de> for RowMap {
    type Error = DeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, DeError> {
        let Some(column) = self.columns.next() else {
            return Ok(None);
        };
        self.pending = self.values.next();
        seed.deserialize(column.into_deserializer()).map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, DeError> {
        seed.deserialize(ValueDe(self.pending.take().unwrap_or(Value::Null)))
    }
}

struct RowSeq(std::vec::IntoIter<Value>);

impl<'de> SeqAccess<'de> for RowSeq {
    type Error = DeError;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, DeError> {
        match self.0.next() {
            Some(v) => seed.deserialize(ValueDe(v)).map(Some),
            None => Ok(None),
        }
    }
}

struct ValueDe(Value);

impl<'de> Deserializer<'de> for ValueDe {
    type Error = DeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.0 {
            Value::Null => visitor.visit_unit(),
            Value::Int(i) => visitor.visit_i64(i),
            Value::Real(f) => visitor.visit_f64(f),
            Value::Text(s) => visitor.visit_string(s),
        }
    }

    /// SQLite stores booleans as 0 and 1.
    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.0 {
            Value::Int(i) => visitor.visit_bool(i != 0),
            Value::Real(f) => visitor.visit_bool(f != 0.0),
            other => Err(de::Error::custom(format!(
                "expected a boolean, found {other:?}"
            ))),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.0 {
            Value::Null => visitor.visit_none(),
            other => visitor.visit_some(ValueDe(other)),
        }
    }

    /// D1 returns every number as a JavaScript number; integers that went
    /// through a float are accepted where an integer is expected.
    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.0 {
            Value::Real(f) if f.fract() == 0.0 => visitor.visit_i64(f as i64),
            other => ValueDe(other).deserialize_any(visitor),
        }
    }

    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_i64(visitor)
    }

    fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_i64(visitor)
    }

    fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_i64(visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        visitor.visit_newtype_struct(self)
    }

    /// Unit variants are stored by name: an unknown name is an error.
    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DeError> {
        match self.0 {
            Value::Text(s) => visitor.visit_enum(s.into_deserializer()),
            other => Err(de::Error::custom(format!(
                "expected a variant name, found {other:?}"
            ))),
        }
    }

    serde::forward_to_deserialize_any! {
        i8 i16 i128 u8 u16 u128 f32 f64 char str string bytes byte_buf unit unit_struct
        seq tuple tuple_struct map struct identifier ignored_any
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    fn row(cols: &[&str], vals: Vec<Value>) -> Row {
        Row {
            columns: cols.iter().map(|c| (*c).to_owned()).collect(),
            values: vals,
        }
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Thing {
        id: String,
        n: i64,
        flag: bool,
        maybe: Option<String>,
        #[serde(default)]
        missing: u32,
    }

    #[test]
    fn decodes_structs_by_name() {
        let r = row(
            &["n", "id", "flag", "maybe", "extra"],
            vec![
                Value::Real(3.0),
                Value::Text("a".into()),
                Value::Int(1),
                Value::Null,
                Value::Int(9),
            ],
        );
        assert_eq!(
            r.decode::<Thing>().unwrap(),
            Thing {
                id: "a".into(),
                n: 3,
                flag: true,
                maybe: None,
                missing: 0
            }
        );
    }

    #[test]
    fn decodes_tuples_by_position() {
        let r = row(&["a", "b"], vec![Value::Text("x".into()), Value::Int(0)]);
        assert_eq!(r.decode::<(String, bool)>().unwrap(), ("x".into(), false));
        let r = row(&["c"], vec![Value::Int(5)]);
        assert_eq!(r.decode::<(i64,)>().unwrap(), (5,));
    }

    #[test]
    fn reports_mismatches() {
        let r = row(&["id", "n", "flag", "maybe"], vec![Value::Int(1)]);
        assert!(matches!(r.decode::<Thing>(), Err(Error::Decode(_))));
    }
}
