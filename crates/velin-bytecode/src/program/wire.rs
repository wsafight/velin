use super::{ExprChunk, ExprChunkRef, Op, Program, SlotTable};
use serde::ser::{SerializeSeq, SerializeStruct};
use serde::{Deserialize, Serialize};

impl Serialize for Program {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("Program", 3)?;
        state.serialize_field("ops", &self.ops)?;
        state.serialize_field("chunks", &SerializedChunks(self))?;
        state.serialize_field("slots", &self.slots)?;
        state.end()
    }
}

struct SerializedChunks<'a>(&'a Program);

impl Serialize for SerializedChunks<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.0.chunks.len()))?;
        for id in 0..self.0.chunks.len() {
            let id = u32::try_from(id).map_err(serde::ser::Error::custom)?;
            let chunk = self
                .0
                .chunk(id)
                .ok_or_else(|| serde::ser::Error::custom("invalid expression arena range"))?;
            sequence.serialize_element(&SerializedChunk(chunk))?;
        }
        sequence.end()
    }
}

struct SerializedChunk<'a>(ExprChunkRef<'a>);

impl Serialize for SerializedChunk<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("ExprChunk", 5)?;
        state.serialize_field("ops", self.0.ops)?;
        state.serialize_field("constants", self.0.constants)?;
        state.serialize_field("registers", &self.0.registers)?;
        state.serialize_field("result", &self.0.result)?;
        state.serialize_field("line", &self.0.line)?;
        state.end()
    }
}

// `SlotTable` is compile-time-only state, but programs are serialized for
// tooling/inspection, so it derives the same traits via a thin manual impl.
impl Serialize for SlotTable {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.names().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SlotTable {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let names = Vec::<String>::deserialize(deserializer)?;
        let mut table = SlotTable::new();
        for name in names {
            if table.get(&name).is_some() {
                return Err(serde::de::Error::custom(format!(
                    "duplicate slot name `{name}`"
                )));
            }
            table.intern(&name);
        }
        Ok(table)
    }
}

impl<'de> Deserialize<'de> for Program {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct ProgramWire {
            ops: Vec<Op>,
            chunks: Vec<ExprChunk>,
            slots: SlotTable,
        }

        let wire = ProgramWire::deserialize(deserializer)?;
        let program = Self::from_chunks(wire.ops, wire.chunks, wire.slots);
        program.validate().map_err(serde::de::Error::custom)?;
        Ok(program)
    }
}
