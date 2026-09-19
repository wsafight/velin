//! Compact tagged wire encoding for artifact JSON payloads.

use super::{ArtifactError, MAX_ARTIFACT_BYTES, MAX_ARTIFACT_ENTRIES};
use serde::de::{self, DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashSet;
use velin_bytecode::Program;

const MAX_WIRE_DEPTH: usize = 64;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct TypeSiteWire {
    pub(super) pc: usize,
    pub(super) expression: velin_syntax::Expr,
    pub(super) kind: TypeCheckKindWire,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) enum TypeCheckKindWire {
    Expression,
    Condition,
    Assignment,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct HostSiteWire {
    pub(super) host_id: u32,
    pub(super) arguments: usize,
    pub(super) bind: bool,
    pub(super) line: usize,
}

pub(super) fn validate_check_sites(
    program: &Program,
    hosts: &[String],
    type_sites: &[TypeSiteWire],
    host_sites: &[HostSiteWire],
) -> Result<(), ArtifactError> {
    if type_sites.len() > MAX_ARTIFACT_ENTRIES || host_sites.len() > MAX_ARTIFACT_ENTRIES {
        return Err(ArtifactError::new(
            "artifact has too many static check sites",
        ));
    }
    if type_sites.iter().any(|site| site.pc >= program.ops.len()) {
        return Err(ArtifactError::new(
            "type check site points outside the program",
        ));
    }
    if host_sites
        .iter()
        .any(|site| usize::try_from(site.host_id).map_or(true, |id| id >= hosts.len()))
    {
        return Err(ArtifactError::new(
            "host check site references an unknown host id",
        ));
    }
    Ok(())
}

pub(super) fn encode_wire_value(
    value: &JsonValue,
    output: &mut Vec<u8>,
) -> Result<(), ArtifactError> {
    match value {
        JsonValue::Null => output.push(0),
        JsonValue::Bool(false) => output.push(1),
        JsonValue::Bool(true) => output.push(2),
        JsonValue::Number(number) => {
            let number = number
                .as_i64()
                .ok_or_else(|| ArtifactError::new("artifact contains a non-integer number"))?;
            output.push(3);
            output.extend_from_slice(&number.to_le_bytes());
        }
        JsonValue::String(text) => {
            output.push(4);
            write_wire_bytes(text.as_bytes(), output)?;
        }
        JsonValue::Array(items) => {
            output.push(5);
            write_wire_len(items.len(), output)?;
            for item in items {
                encode_wire_value(item, output)?;
            }
        }
        JsonValue::Object(fields) => {
            output.push(6);
            write_wire_len(fields.len(), output)?;
            for (key, value) in fields {
                write_wire_bytes(key.as_bytes(), output)?;
                encode_wire_value(value, output)?;
            }
        }
    }
    if output.len() > MAX_ARTIFACT_BYTES {
        return Err(ArtifactError::new(format!(
            "artifact exceeds {MAX_ARTIFACT_BYTES} bytes"
        )));
    }
    Ok(())
}

fn write_wire_len(length: usize, output: &mut Vec<u8>) -> Result<(), ArtifactError> {
    let length =
        u32::try_from(length).map_err(|_| ArtifactError::new("artifact count overflow"))?;
    output.extend_from_slice(&length.to_le_bytes());
    Ok(())
}

fn write_wire_bytes(bytes: &[u8], output: &mut Vec<u8>) -> Result<(), ArtifactError> {
    write_wire_len(bytes.len(), output)?;
    output.extend_from_slice(bytes);
    Ok(())
}

pub(super) fn decode_wire<'de, T>(input: &'de [u8]) -> Result<(T, usize), ArtifactError>
where
    T: Deserialize<'de>,
{
    let mut deserializer = WireDeserializer {
        input,
        cursor: 0,
        depth: 0,
    };
    let value = T::deserialize(&mut deserializer)
        .map_err(|error| ArtifactError::new(format!("cannot decode artifact payload: {error}")))?;
    Ok((value, deserializer.cursor))
}

struct WireDeserializer<'de> {
    input: &'de [u8],
    cursor: usize,
    depth: usize,
}

impl<'de> WireDeserializer<'de> {
    fn read_tag(&mut self) -> Result<u8, ArtifactError> {
        read_wire_byte(self.input, &mut self.cursor)
    }

    fn read_integer(&mut self) -> Result<i64, ArtifactError> {
        let tag = self.read_tag()?;
        if tag != 3 {
            return Err(expected_tag("integer", tag));
        }
        let bytes = read_wire_slice(self.input, &mut self.cursor, 8)?;
        Ok(i64::from_le_bytes(
            bytes.try_into().expect("wire integer length is eight"),
        ))
    }

    fn read_text(&mut self) -> Result<&'de str, ArtifactError> {
        let tag = self.read_tag()?;
        if tag != 4 {
            return Err(expected_tag("string", tag));
        }
        read_wire_str(self.input, &mut self.cursor)
    }

    fn nested<T>(
        &mut self,
        deserialize: impl FnOnce(&mut Self) -> Result<T, ArtifactError>,
    ) -> Result<T, ArtifactError> {
        self.depth += 1;
        if self.depth > MAX_WIRE_DEPTH {
            self.depth -= 1;
            return Err(ArtifactError::new("artifact nesting exceeds limit"));
        }
        let result = deserialize(self);
        self.depth -= 1;
        result
    }

    fn deserialize_nested<T>(&mut self, seed: T) -> Result<T::Value, ArtifactError>
    where
        T: DeserializeSeed<'de>,
    {
        self.nested(|deserializer| seed.deserialize(deserializer))
    }

    fn visit_sequence<V>(&mut self, visitor: V) -> Result<V::Value, ArtifactError>
    where
        V: Visitor<'de>,
    {
        let tag = self.read_tag()?;
        if tag != 5 {
            return Err(expected_tag("array", tag));
        }
        let count = read_wire_len(self.input, &mut self.cursor)?;
        visitor.visit_seq(WireSeqAccess {
            deserializer: self,
            remaining: count,
        })
    }

    fn visit_map<V>(&mut self, visitor: V) -> Result<V::Value, ArtifactError>
    where
        V: Visitor<'de>,
    {
        let tag = self.read_tag()?;
        if tag != 6 {
            return Err(expected_tag("object", tag));
        }
        let count = read_wire_len(self.input, &mut self.cursor)?;
        visitor.visit_map(WireMapAccess {
            deserializer: self,
            remaining: count,
            value_pending: false,
            keys: WireKeys::new(count),
        })
    }
}

struct WireSeqAccess<'a, 'de> {
    deserializer: &'a mut WireDeserializer<'de>,
    remaining: usize,
}

impl<'de> SeqAccess<'de> for WireSeqAccess<'_, 'de> {
    type Error = ArtifactError;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
    where
        T: DeserializeSeed<'de>,
    {
        if self.remaining == 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        self.deserializer.deserialize_nested(seed).map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.remaining)
    }
}

struct WireMapAccess<'a, 'de> {
    deserializer: &'a mut WireDeserializer<'de>,
    remaining: usize,
    value_pending: bool,
    keys: WireKeys<'de>,
}

enum WireKeys<'de> {
    Inline {
        keys: [Option<&'de str>; 8],
        length: usize,
    },
    Heap(HashSet<&'de str>),
}

impl<'de> WireKeys<'de> {
    fn new(count: usize) -> Self {
        if count <= 8 {
            Self::Inline {
                keys: [None; 8],
                length: 0,
            }
        } else {
            Self::Heap(HashSet::with_capacity(count.min(1_024)))
        }
    }

    fn insert(&mut self, key: &'de str) -> bool {
        match self {
            Self::Inline { keys, length } => {
                if keys[..*length].contains(&Some(key)) {
                    return false;
                }
                keys[*length] = Some(key);
                *length += 1;
                true
            }
            Self::Heap(keys) => keys.insert(key),
        }
    }
}

impl<'de> MapAccess<'de> for WireMapAccess<'_, 'de> {
    type Error = ArtifactError;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: DeserializeSeed<'de>,
    {
        if self.value_pending {
            return Err(ArtifactError::new("artifact object value was not decoded"));
        }
        if self.remaining == 0 {
            return Ok(None);
        }
        let key = read_wire_str(self.deserializer.input, &mut self.deserializer.cursor)?;
        if !self.keys.insert(key) {
            return Err(ArtifactError::new(
                "artifact contains duplicate object keys",
            ));
        }
        self.value_pending = true;
        seed.deserialize(de::value::BorrowedStrDeserializer::<ArtifactError>::new(
            key,
        ))
        .map(Some)
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: DeserializeSeed<'de>,
    {
        if !self.value_pending {
            return Err(ArtifactError::new("artifact object key is missing"));
        }
        self.value_pending = false;
        self.remaining -= 1;
        self.deserializer.deserialize_nested(seed)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.remaining)
    }
}

struct WireEnumAccess<'a, 'de> {
    deserializer: &'a mut WireDeserializer<'de>,
    variant: &'de str,
}

impl<'a, 'de> EnumAccess<'de> for WireEnumAccess<'a, 'de> {
    type Error = ArtifactError;
    type Variant = WireVariantAccess<'a, 'de>;

    fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant), Self::Error>
    where
        V: DeserializeSeed<'de>,
    {
        let variant = seed.deserialize(
            de::value::BorrowedStrDeserializer::<ArtifactError>::new(self.variant),
        )?;
        Ok((
            variant,
            WireVariantAccess {
                deserializer: self.deserializer,
            },
        ))
    }
}

struct WireVariantAccess<'a, 'de> {
    deserializer: &'a mut WireDeserializer<'de>,
}

impl<'de> VariantAccess<'de> for WireVariantAccess<'_, 'de> {
    type Error = ArtifactError;

    fn unit_variant(self) -> Result<(), Self::Error> {
        Err(ArtifactError::new(
            "artifact unit enum variant must use a string",
        ))
    }

    fn newtype_variant_seed<T>(self, seed: T) -> Result<T::Value, Self::Error>
    where
        T: DeserializeSeed<'de>,
    {
        self.deserializer.deserialize_nested(seed)
    }

    fn tuple_variant<V>(self, length: usize, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserializer.nested(|deserializer| {
            de::Deserializer::deserialize_tuple(deserializer, length, visitor)
        })
    }

    fn struct_variant<V>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserializer.nested(|deserializer| {
            de::Deserializer::deserialize_struct(deserializer, "variant", fields, visitor)
        })
    }
}

macro_rules! deserialize_signed {
    ($method:ident, $visit:ident, $type:ty) => {
        fn $method<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            let value = self.read_integer()?;
            let value = <$type>::try_from(value)
                .map_err(|_| ArtifactError::new("artifact integer is out of range"))?;
            visitor.$visit(value)
        }
    };
}

macro_rules! deserialize_unsigned {
    ($method:ident, $visit:ident, $type:ty) => {
        fn $method<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            let value = self.read_integer()?;
            let value = <$type>::try_from(value)
                .map_err(|_| ArtifactError::new("artifact integer is out of range"))?;
            visitor.$visit(value)
        }
    };
}

impl<'de> de::Deserializer<'de> for &mut WireDeserializer<'de> {
    type Error = ArtifactError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let tag = self.read_tag()?;
        match tag {
            0 => visitor.visit_unit(),
            1 => visitor.visit_bool(false),
            2 => visitor.visit_bool(true),
            3 => {
                let bytes = read_wire_slice(self.input, &mut self.cursor, 8)?;
                visitor.visit_i64(i64::from_le_bytes(
                    bytes.try_into().expect("wire integer length is eight"),
                ))
            }
            4 => visitor.visit_borrowed_str(read_wire_str(self.input, &mut self.cursor)?),
            5 => {
                let count = read_wire_len(self.input, &mut self.cursor)?;
                visitor.visit_seq(WireSeqAccess {
                    deserializer: self,
                    remaining: count,
                })
            }
            6 => {
                let count = read_wire_len(self.input, &mut self.cursor)?;
                visitor.visit_map(WireMapAccess {
                    deserializer: self,
                    remaining: count,
                    value_pending: false,
                    keys: WireKeys::new(count),
                })
            }
            _ => Err(ArtifactError::new("artifact contains an unknown wire tag")),
        }
    }

    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.read_tag()? {
            1 => visitor.visit_bool(false),
            2 => visitor.visit_bool(true),
            tag => Err(expected_tag("boolean", tag)),
        }
    }

    deserialize_signed!(deserialize_i8, visit_i8, i8);
    deserialize_signed!(deserialize_i16, visit_i16, i16);
    deserialize_signed!(deserialize_i32, visit_i32, i32);
    deserialize_signed!(deserialize_i64, visit_i64, i64);
    deserialize_signed!(deserialize_i128, visit_i128, i128);
    deserialize_unsigned!(deserialize_u8, visit_u8, u8);
    deserialize_unsigned!(deserialize_u16, visit_u16, u16);
    deserialize_unsigned!(deserialize_u32, visit_u32, u32);
    deserialize_unsigned!(deserialize_u64, visit_u64, u64);
    deserialize_unsigned!(deserialize_u128, visit_u128, u128);

    fn deserialize_f32<V>(self, _visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(ArtifactError::new(
            "artifact payload does not support floating-point numbers",
        ))
    }

    fn deserialize_f64<V>(self, _visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(ArtifactError::new(
            "artifact payload does not support floating-point numbers",
        ))
    }

    fn deserialize_char<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let text = self.read_text()?;
        let mut characters = text.chars();
        let character = characters
            .next()
            .ok_or_else(|| ArtifactError::new("artifact character is empty"))?;
        if characters.next().is_some() {
            return Err(ArtifactError::new(
                "artifact character contains multiple characters",
            ));
        }
        visitor.visit_char(character)
    }

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_borrowed_str(self.read_text()?)
    }

    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_string(self.read_text()?.to_owned())
    }

    fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.visit_sequence(visitor)
    }

    fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.visit_sequence(visitor)
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if self.input.get(self.cursor) == Some(&0) {
            self.cursor += 1;
            visitor.visit_none()
        } else {
            visitor.visit_some(self)
        }
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let tag = self.read_tag()?;
        if tag == 0 {
            visitor.visit_unit()
        } else {
            Err(expected_tag("null", tag))
        }
    }

    fn deserialize_unit_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.visit_sequence(visitor)
    }

    fn deserialize_tuple<V>(self, _length: usize, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.visit_sequence(visitor)
    }

    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        _length: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.visit_sequence(visitor)
    }

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.visit_map(visitor)
    }

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.visit_map(visitor)
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let tag = self.read_tag()?;
        match tag {
            4 => {
                let variant = read_wire_str(self.input, &mut self.cursor)?;
                visitor.visit_enum(de::value::BorrowedStrDeserializer::<ArtifactError>::new(
                    variant,
                ))
            }
            6 => {
                let count = read_wire_len(self.input, &mut self.cursor)?;
                if count != 1 {
                    return Err(ArtifactError::new(
                        "artifact enum object must contain one variant",
                    ));
                }
                let variant = read_wire_str(self.input, &mut self.cursor)?;
                visitor.visit_enum(WireEnumAccess {
                    deserializer: self,
                    variant,
                })
            }
            _ => Err(expected_tag("enum", tag)),
        }
    }

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_str(visitor)
    }

    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        skip_wire_value(self.input, &mut self.cursor, self.depth)?;
        visitor.visit_unit()
    }

    fn is_human_readable(&self) -> bool {
        true
    }
}

fn expected_tag(expected: &str, tag: u8) -> ArtifactError {
    if tag > 6 {
        ArtifactError::new("artifact contains an unknown wire tag")
    } else {
        ArtifactError::new(format!(
            "artifact expected {expected}, found wire tag {tag}"
        ))
    }
}

fn skip_wire_value(input: &[u8], cursor: &mut usize, depth: usize) -> Result<(), ArtifactError> {
    if depth > MAX_WIRE_DEPTH {
        return Err(ArtifactError::new("artifact nesting exceeds limit"));
    }
    match read_wire_byte(input, cursor)? {
        0..=2 => Ok(()),
        3 => {
            read_wire_slice(input, cursor, 8)?;
            Ok(())
        }
        4 => {
            read_wire_str(input, cursor)?;
            Ok(())
        }
        5 => {
            let count = read_wire_len(input, cursor)?;
            for _ in 0..count {
                skip_wire_value(input, cursor, depth + 1)?;
            }
            Ok(())
        }
        6 => {
            let count = read_wire_len(input, cursor)?;
            let mut keys = WireKeys::new(count);
            for _ in 0..count {
                let key = read_wire_str(input, cursor)?;
                if !keys.insert(key) {
                    return Err(ArtifactError::new(
                        "artifact contains duplicate object keys",
                    ));
                }
                skip_wire_value(input, cursor, depth + 1)?;
            }
            Ok(())
        }
        _ => Err(ArtifactError::new("artifact contains an unknown wire tag")),
    }
}

fn read_wire_byte(input: &[u8], cursor: &mut usize) -> Result<u8, ArtifactError> {
    let byte = *input
        .get(*cursor)
        .ok_or_else(|| ArtifactError::new("artifact payload is truncated"))?;
    *cursor += 1;
    Ok(byte)
}

fn read_wire_len(input: &[u8], cursor: &mut usize) -> Result<usize, ArtifactError> {
    let bytes = read_wire_slice(input, cursor, 4)?;
    let length = u32::from_le_bytes(bytes.try_into().expect("wire length is four"));
    let length =
        usize::try_from(length).map_err(|_| ArtifactError::new("artifact length overflow"))?;
    if length > MAX_ARTIFACT_BYTES {
        return Err(ArtifactError::new("artifact length exceeds limit"));
    }
    Ok(length)
}

fn read_wire_str<'a>(input: &'a [u8], cursor: &mut usize) -> Result<&'a str, ArtifactError> {
    let length = read_wire_len(input, cursor)?;
    let bytes = read_wire_slice(input, cursor, length)?;
    std::str::from_utf8(bytes).map_err(|_| ArtifactError::new("artifact contains invalid UTF-8"))
}

fn read_wire_slice<'a>(
    input: &'a [u8],
    cursor: &mut usize,
    length: usize,
) -> Result<&'a [u8], ArtifactError> {
    let end = cursor
        .checked_add(length)
        .ok_or_else(|| ArtifactError::new("artifact payload length overflow"))?;
    let bytes = input
        .get(*cursor..end)
        .ok_or_else(|| ArtifactError::new("artifact payload is truncated"))?;
    *cursor = end;
    Ok(bytes)
}
