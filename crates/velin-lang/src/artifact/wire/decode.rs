//! Bounded accessors for decoding the compact artifact wire format.

use super::{ArtifactError, MAX_ARTIFACT_BYTES};
use serde::Deserialize;
use serde::de::{self, DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor};
use std::collections::HashSet;

const MAX_WIRE_DEPTH: usize = 64;

pub(in crate::artifact) fn decode_wire<'de, T>(
    input: &'de [u8],
) -> Result<(T, usize), ArtifactError>
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

#[path = "deserialize.rs"]
mod deserialize;

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
