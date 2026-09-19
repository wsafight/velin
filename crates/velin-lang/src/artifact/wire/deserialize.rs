//! Serde deserializer implementation for the compact artifact wire format.

use super::{
    ArtifactError, Visitor, WireDeserializer, WireEnumAccess, WireKeys, WireMapAccess,
    WireSeqAccess, de, expected_tag, read_wire_len, read_wire_slice, read_wire_str,
    skip_wire_value,
};

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
