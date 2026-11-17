//! Allocation-bounded manifest encoding for private receipt prototypes. This does
//! not select or standardize a durable receipt schema.
use crate::{ModelManifest, ReceiptError, MAX_MODEL_MANIFEST_BYTES};
use alloc::vec::Vec;
use core::fmt;
use serde::ser::{self, Serialize};
use sha2::{Digest, Sha256};

pub fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub fn encode_manifest_json(manifest: &ModelManifest) -> Result<Vec<u8>, ReceiptError> {
    let mut budget = Budget { used: 0 };
    manifest
        .serialize(Counter(&mut budget))
        .map_err(|_| ReceiptError::Capacity)?;
    manifest
        .validate()
        .map_err(|_| ReceiptError::InvalidManifest)?;
    let bytes = serde_json::to_vec(manifest).map_err(|_| ReceiptError::InvalidManifest)?;
    if bytes.len() > MAX_MODEL_MANIFEST_BYTES || bytes.len() != budget.used {
        return Err(ReceiptError::Capacity);
    }
    Ok(bytes)
}
#[derive(Debug)]
struct Limit;
impl fmt::Display for Limit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded manifest capacity")
    }
}
impl core::error::Error for Limit {}
impl ser::Error for Limit {
    fn custom<T: fmt::Display>(_: T) -> Self {
        Self
    }
}
struct Budget {
    used: usize,
}
impl Budget {
    fn add(&mut self, n: usize) -> Result<(), Limit> {
        self.used = self
            .used
            .checked_add(n)
            .filter(|v| *v <= MAX_MODEL_MANIFEST_BYTES)
            .ok_or(Limit)?;
        Ok(())
    }
    fn string(&mut self, s: &str) -> Result<(), Limit> {
        self.add(2)?;
        for b in s.bytes() {
            self.add(match b {
                b'"' | b'\\' | 8 | 9 | 10 | 12 | 13 => 2,
                0..=31 => 6,
                _ => 1,
            })?;
        }
        Ok(())
    }
    fn scalar<T: Serialize>(&mut self, n: T) -> Result<(), Limit> {
        let b = serde_json::to_vec(&n).map_err(|_| Limit)?;
        self.add(b.len())
    }
}
struct Counter<'a>(&'a mut Budget);
struct Compound<'a> {
    budget: &'a mut Budget,
    first: bool,
    end: usize,
}
impl Compound<'_> {
    fn sep(&mut self) -> Result<(), Limit> {
        if !self.first {
            self.budget.add(1)?;
        }
        self.first = false;
        Ok(())
    }
    fn value<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Limit> {
        self.sep()?;
        v.serialize(Counter(self.budget))
    }
    fn field<T: Serialize + ?Sized>(&mut self, k: &str, v: &T) -> Result<(), Limit> {
        self.sep()?;
        self.budget.string(k)?;
        self.budget.add(1)?;
        v.serialize(Counter(self.budget))
    }
    fn finish(self) -> Result<(), Limit> {
        self.budget.add(self.end)
    }
}
macro_rules! scalar {
    ($name:ident,$t:ty) => {
        fn $name(self, v: $t) -> Result<(), Limit> {
            self.0.scalar(v)
        }
    };
}
impl<'a> ser::Serializer for Counter<'a> {
    type Ok = ();
    type Error = Limit;
    type SerializeSeq = Compound<'a>;
    type SerializeTuple = Compound<'a>;
    type SerializeTupleStruct = Compound<'a>;
    type SerializeTupleVariant = Compound<'a>;
    type SerializeMap = Compound<'a>;
    type SerializeStruct = Compound<'a>;
    type SerializeStructVariant = Compound<'a>;
    scalar!(serialize_bool, bool);
    scalar!(serialize_i8, i8);
    scalar!(serialize_i16, i16);
    scalar!(serialize_i32, i32);
    scalar!(serialize_i64, i64);
    scalar!(serialize_i128, i128);
    scalar!(serialize_u8, u8);
    scalar!(serialize_u16, u16);
    scalar!(serialize_u32, u32);
    scalar!(serialize_u64, u64);
    scalar!(serialize_u128, u128);
    scalar!(serialize_f32, f32);
    scalar!(serialize_f64, f64);
    fn serialize_char(self, v: char) -> Result<(), Limit> {
        let mut s = [0; 4];
        self.0.string(v.encode_utf8(&mut s))
    }
    fn serialize_str(self, v: &str) -> Result<(), Limit> {
        self.0.string(v)
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<(), Limit> {
        let mut seq = self.serialize_seq(Some(v.len()))?;
        for n in v {
            seq.value(n)?;
        }
        seq.finish()
    }
    fn serialize_none(self) -> Result<(), Limit> {
        self.0.add(4)
    }
    fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> Result<(), Limit> {
        v.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), Limit> {
        self.0.add(4)
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), Limit> {
        self.serialize_unit()
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, v: &'static str) -> Result<(), Limit> {
        self.0.string(v)
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        v: &T,
    ) -> Result<(), Limit> {
        v.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        k: &'static str,
        v: &T,
    ) -> Result<(), Limit> {
        self.0.add(1)?;
        self.0.string(k)?;
        self.0.add(1)?;
        v.serialize(Counter(self.0))?;
        self.0.add(1)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Compound<'a>, Limit> {
        self.0.add(1)?;
        Ok(Compound {
            budget: self.0,
            first: true,
            end: 1,
        })
    }
    fn serialize_tuple(self, n: usize) -> Result<Compound<'a>, Limit> {
        self.serialize_seq(Some(n))
    }
    fn serialize_tuple_struct(self, _: &'static str, n: usize) -> Result<Compound<'a>, Limit> {
        self.serialize_seq(Some(n))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        k: &'static str,
        n: usize,
    ) -> Result<Compound<'a>, Limit> {
        self.0.add(1)?;
        self.0.string(k)?;
        self.0.add(1)?;
        let mut c = self.serialize_seq(Some(n))?;
        c.end = 2;
        Ok(c)
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Compound<'a>, Limit> {
        self.0.add(1)?;
        Ok(Compound {
            budget: self.0,
            first: true,
            end: 1,
        })
    }
    fn serialize_struct(self, _: &'static str, n: usize) -> Result<Compound<'a>, Limit> {
        self.serialize_map(Some(n))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        k: &'static str,
        n: usize,
    ) -> Result<Compound<'a>, Limit> {
        self.0.add(1)?;
        self.0.string(k)?;
        self.0.add(1)?;
        let mut c = self.serialize_map(Some(n))?;
        c.end = 2;
        Ok(c)
    }
    fn collect_str<T: fmt::Display + ?Sized>(self, _: &T) -> Result<(), Limit> {
        Err(Limit)
    }
}
macro_rules! elements {
    ($trait:ident,$method:ident) => {
        impl ser::$trait for Compound<'_> {
            type Ok = ();
            type Error = Limit;
            fn $method<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Limit> {
                self.value(v)
            }
            fn end(self) -> Result<(), Limit> {
                self.finish()
            }
        }
    };
}
elements!(SerializeSeq, serialize_element);
elements!(SerializeTuple, serialize_element);
elements!(SerializeTupleStruct, serialize_field);
elements!(SerializeTupleVariant, serialize_field);
impl ser::SerializeMap for Compound<'_> {
    type Ok = ();
    type Error = Limit;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, k: &T) -> Result<(), Limit> {
        self.sep()?;
        k.serialize(Counter(self.budget))?;
        self.budget.add(1)
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Limit> {
        v.serialize(Counter(self.budget))
    }
    fn end(self) -> Result<(), Limit> {
        self.finish()
    }
}
macro_rules! fields {
    ($trait:ident) => {
        impl ser::$trait for Compound<'_> {
            type Ok = ();
            type Error = Limit;
            fn serialize_field<T: Serialize + ?Sized>(
                &mut self,
                k: &'static str,
                v: &T,
            ) -> Result<(), Limit> {
                self.field(k, v)
            }
            fn end(self) -> Result<(), Limit> {
                self.finish()
            }
        }
    };
}
fields!(SerializeStruct);
fields!(SerializeStructVariant);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_encoding_roundtrips_actual_manifest_and_escaped_text() {
        let mut m =
            ModelManifest::parse_json(include_bytes!("../tests/fixtures/granite-4.2-3b.json"))
                .unwrap();
        m.display_name = alloc::string::String::from("quoted \"model\" 日本語");
        let bytes = encode_manifest_json(&m).unwrap();
        assert_eq!(ModelManifest::parse_json(&bytes), Ok(m));
    }
    #[test]
    fn oversized_valid_collection_is_rejected_before_encoding_allocation() {
        let mut m =
            ModelManifest::parse_json(include_bytes!("../tests/fixtures/granite-4.2-3b.json"))
                .unwrap();
        m.roles = (0..10000)
            .map(|i| crate::RoleId::new(alloc::format!("role.{i}")).unwrap())
            .collect();
        assert_eq!(encode_manifest_json(&m), Err(ReceiptError::Capacity));
    }
}
