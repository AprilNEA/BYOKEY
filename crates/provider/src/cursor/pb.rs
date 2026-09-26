//! Minimal protobuf wire codec for Cursor's `agent.v1` messages.
//!
//! Cursor ships no public schema; the field numbers used by the rest of this
//! module were read off the CLI's own traffic. A hand-rolled codec keeps that
//! knowledge next to the code that needs it instead of in a fake `.proto`.

use bytes::{BufMut, Bytes, BytesMut};

/// A decoded field value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value<'a> {
    Varint(u64),
    Fixed64(u64),
    Bytes(&'a [u8]),
    Fixed32(u32),
}

/// A parsed message: `(field number, value)` pairs in wire order.
#[derive(Debug, Default)]
pub struct Fields<'a>(Vec<(u32, Value<'a>)>);

fn read_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut out = 0u64;
    for shift in (0..64).step_by(7) {
        let b = *buf.get(*pos)?;
        *pos += 1;
        out |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some(out);
        }
    }
    None
}

impl<'a> Fields<'a> {
    /// Parse `buf`. Truncated or malformed input yields the fields read so far;
    /// groups (wire types 3/4) end parsing, since Cursor never sends them.
    #[must_use]
    pub fn parse(buf: &'a [u8]) -> Self {
        let mut out = Vec::new();
        let mut pos = 0;
        while pos < buf.len() {
            let Some(key) = read_varint(buf, &mut pos) else {
                break;
            };
            let Ok(field) = u32::try_from(key >> 3) else {
                break;
            };
            let value = match key & 7 {
                0 => match read_varint(buf, &mut pos) {
                    Some(v) => Value::Varint(v),
                    None => break,
                },
                1 => match buf.get(pos..pos + 8) {
                    Some(b) => {
                        pos += 8;
                        Value::Fixed64(u64::from_le_bytes(b.try_into().expect("8 bytes")))
                    }
                    None => break,
                },
                2 => {
                    let Some(len) = read_varint(buf, &mut pos) else {
                        break;
                    };
                    let Ok(len) = usize::try_from(len) else { break };
                    let Some(b) = buf.get(pos..pos.saturating_add(len)) else {
                        break;
                    };
                    pos += len;
                    Value::Bytes(b)
                }
                5 => match buf.get(pos..pos + 4) {
                    Some(b) => {
                        pos += 4;
                        Value::Fixed32(u32::from_le_bytes(b.try_into().expect("4 bytes")))
                    }
                    None => break,
                },
                _ => break,
            };
            out.push((field, value));
        }
        Self(out)
    }

    #[must_use]
    pub fn has(&self, field: u32) -> bool {
        self.0.iter().any(|(f, _)| *f == field)
    }

    /// Field numbers present, in wire order.
    pub fn numbers(&self) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().map(|(f, _)| *f)
    }

    /// First length-delimited value of `field`.
    #[must_use]
    pub fn bytes(&self, field: u32) -> Option<&'a [u8]> {
        self.0.iter().find_map(|(f, v)| match v {
            Value::Bytes(b) if *f == field => Some(*b),
            _ => None,
        })
    }

    /// Every length-delimited value of `field` (a repeated field).
    pub fn all_bytes(&self, field: u32) -> impl Iterator<Item = &'a [u8]> + '_ {
        self.0.iter().filter_map(move |(f, v)| match v {
            Value::Bytes(b) if *f == field => Some(*b),
            _ => None,
        })
    }

    #[must_use]
    pub fn message(&self, field: u32) -> Option<Fields<'a>> {
        self.bytes(field).map(Fields::parse)
    }

    #[must_use]
    pub fn str(&self, field: u32) -> Option<&'a str> {
        self.bytes(field).and_then(|b| std::str::from_utf8(b).ok())
    }

    #[must_use]
    pub fn varint(&self, field: u32) -> Option<u64> {
        self.0.iter().find_map(|(f, v)| match v {
            Value::Varint(n) if *f == field => Some(*n),
            _ => None,
        })
    }
}

/// Protobuf message builder. Fields are written in call order; nothing is
/// packed and default values are still emitted, matching Cursor's client.
#[derive(Debug, Default, Clone)]
pub struct Msg(BytesMut);

impl Msg {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn key(&mut self, field: u32, wire: u8) {
        self.put_varint(u64::from(field) << 3 | u64::from(wire));
    }

    fn put_varint(&mut self, mut v: u64) {
        while v >= 0x80 {
            // Truncation is the encoding: the low 7 bits plus a continuation bit.
            #[allow(clippy::cast_possible_truncation)]
            self.0.put_u8((v as u8 & 0x7f) | 0x80);
            v >>= 7;
        }
        #[allow(clippy::cast_possible_truncation)]
        self.0.put_u8(v as u8);
    }

    #[must_use]
    pub fn varint(mut self, field: u32, v: u64) -> Self {
        self.key(field, 0);
        self.put_varint(v);
        self
    }

    #[must_use]
    pub fn bool(self, field: u32, v: bool) -> Self {
        self.varint(field, u64::from(v))
    }

    #[must_use]
    pub fn bytes(mut self, field: u32, v: &[u8]) -> Self {
        self.key(field, 2);
        self.put_varint(v.len() as u64);
        self.0.put_slice(v);
        self
    }

    #[must_use]
    pub fn str(self, field: u32, v: &str) -> Self {
        self.bytes(field, v.as_bytes())
    }

    #[must_use]
    pub fn msg(self, field: u32, v: &Msg) -> Self {
        self.bytes(field, &v.0)
    }

    #[must_use]
    pub fn finish(self) -> Bytes {
        self.0.freeze()
    }
}

/// Decode a `google.protobuf.Value` into JSON.
#[must_use]
pub fn decode_json_value(buf: &[u8]) -> serde_json::Value {
    use serde_json::Value as J;
    let f = Fields::parse(buf);
    match f.0.first() {
        Some((2, Value::Fixed64(bits))) => {
            let n = f64::from_bits(*bits);
            // Integral doubles round-trip as integers so `{"limit": 5}` stays 5.
            #[allow(clippy::cast_possible_truncation, clippy::float_cmp)]
            if n.fract() == 0.0 && n.abs() < 9.0e15 {
                J::from(n as i64)
            } else {
                serde_json::Number::from_f64(n).map_or(J::Null, J::Number)
            }
        }
        Some((3, Value::Bytes(b))) => J::String(String::from_utf8_lossy(b).into_owned()),
        Some((4, Value::Varint(v))) => J::Bool(*v != 0),
        Some((5, Value::Bytes(b))) => J::Object(decode_struct(b)),
        Some((6, Value::Bytes(b))) => J::Array(
            Fields::parse(b)
                .all_bytes(1)
                .map(decode_json_value)
                .collect(),
        ),
        _ => J::Null,
    }
}

/// Decode a `google.protobuf.Struct` (`map<string, Value> fields = 1`).
#[must_use]
pub fn decode_struct(buf: &[u8]) -> serde_json::Map<String, serde_json::Value> {
    Fields::parse(buf)
        .all_bytes(1)
        .filter_map(|entry| {
            let e = Fields::parse(entry);
            Some((
                e.str(1)?.to_owned(),
                e.bytes(2)
                    .map_or(serde_json::Value::Null, decode_json_value),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_nested_messages() {
        let inner = Msg::new().str(1, "hi").varint(2, 300);
        let outer = Msg::new().msg(3, &inner).bool(4, true).finish();
        let f = Fields::parse(&outer);
        let i = f.message(3).unwrap();
        assert_eq!(i.str(1), Some("hi"));
        assert_eq!(i.varint(2), Some(300));
        assert_eq!(f.varint(4), Some(1));
    }

    #[test]
    fn repeated_fields_keep_every_value() {
        let buf = Msg::new().str(1, "a").str(1, "b").finish();
        let f = Fields::parse(&buf);
        assert_eq!(
            f.all_bytes(1).collect::<Vec<_>>(),
            vec![b"a".as_slice(), b"b"]
        );
        assert_eq!(f.str(1), Some("a"));
    }

    #[test]
    fn truncated_input_keeps_complete_fields() {
        let mut buf = Msg::new().str(1, "ok").finish().to_vec();
        buf.extend_from_slice(&[0x12, 0x05, b'x']);
        let f = Fields::parse(&buf);
        assert_eq!(f.str(1), Some("ok"));
        assert!(!f.has(2));
    }

    fn value_str(s: &str) -> Msg {
        Msg::new().str(3, s)
    }

    #[test]
    fn decodes_protobuf_values_to_json() {
        let num = Msg::new().bytes(2, &[]);
        let mut n = BytesMut::new();
        n.put_u8(2 << 3 | 1);
        n.put_f64_le(5.0);
        let list = Msg::new()
            .msg(1, &value_str("x"))
            .msg(1, &Msg::new().bool(4, true));
        let entry = Msg::new().str(1, "k").msg(2, &value_str("v"));
        let obj = Msg::new().msg(5, &Msg::new().msg(1, &entry));
        assert_eq!(decode_json_value(&n), serde_json::json!(5));
        assert_eq!(
            decode_json_value(&Msg::new().msg(6, &list).finish()),
            serde_json::json!(["x", true])
        );
        assert_eq!(
            decode_json_value(&obj.finish()),
            serde_json::json!({"k": "v"})
        );
        assert_eq!(
            decode_json_value(&Msg::new().varint(1, 0).finish()),
            serde_json::Value::Null
        );
        let _ = num;
    }
}
