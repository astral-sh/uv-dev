use std::str::FromStr;

use anyhow::Result;
use serde::ser::{Error, Impossible, SerializeSeq};
use serde::{Serialize, Serializer};
use serde_json::Value;

use uv_pep508::MarkerTree;
use uv_pypi_types::SupportedEnvironments;

type SerializeError = serde::de::value::Error;

struct KnownLengthSequence {
    declared: usize,
    values: Vec<String>,
}

impl SerializeSeq for KnownLengthSequence {
    type Ok = Vec<String>;
    type Error = SerializeError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        match serde_json::to_value(value).map_err(SerializeError::custom)? {
            Value::String(value) => {
                self.values.push(value);
                Ok(())
            }
            _ => Err(SerializeError::custom("expected a marker string")),
        }
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        if self.declared == self.values.len() {
            Ok(self.values)
        } else {
            Err(SerializeError::custom(format!(
                "declared {} elements but serialized {}",
                self.declared,
                self.values.len()
            )))
        }
    }
}

struct KnownLengthSerializer;

macro_rules! reject {
    ($($method:ident($($argument:ty),*) -> $output:ty;)*) => {
        $(fn $method(self, $(_: $argument),*) -> Result<$output, Self::Error> {
            Err(SerializeError::custom("expected a sequence"))
        })*
    };
}

impl Serializer for KnownLengthSerializer {
    type Ok = Vec<String>;
    type Error = SerializeError;
    type SerializeSeq = KnownLengthSequence;
    type SerializeTuple = Impossible<Self::Ok, Self::Error>;
    type SerializeTupleStruct = Impossible<Self::Ok, Self::Error>;
    type SerializeTupleVariant = Impossible<Self::Ok, Self::Error>;
    type SerializeMap = Impossible<Self::Ok, Self::Error>;
    type SerializeStruct = Impossible<Self::Ok, Self::Error>;
    type SerializeStructVariant = Impossible<Self::Ok, Self::Error>;

    reject! {
        serialize_bool(bool) -> Self::Ok;
        serialize_i8(i8) -> Self::Ok;
        serialize_i16(i16) -> Self::Ok;
        serialize_i32(i32) -> Self::Ok;
        serialize_i64(i64) -> Self::Ok;
        serialize_u8(u8) -> Self::Ok;
        serialize_u16(u16) -> Self::Ok;
        serialize_u32(u32) -> Self::Ok;
        serialize_u64(u64) -> Self::Ok;
        serialize_f32(f32) -> Self::Ok;
        serialize_f64(f64) -> Self::Ok;
        serialize_char(char) -> Self::Ok;
        serialize_str(&str) -> Self::Ok;
        serialize_bytes(&[u8]) -> Self::Ok;
        serialize_none() -> Self::Ok;
        serialize_unit() -> Self::Ok;
        serialize_unit_struct(&'static str) -> Self::Ok;
        serialize_unit_variant(&'static str, u32, &'static str) -> Self::Ok;
        serialize_tuple(usize) -> Self::SerializeTuple;
        serialize_tuple_struct(&'static str, usize) -> Self::SerializeTupleStruct;
        serialize_tuple_variant(&'static str, u32, &'static str, usize) -> Self::SerializeTupleVariant;
        serialize_map(Option<usize>) -> Self::SerializeMap;
        serialize_struct(&'static str, usize) -> Self::SerializeStruct;
        serialize_struct_variant(&'static str, u32, &'static str, usize) -> Self::SerializeStructVariant;
    }

    fn serialize_some<T: ?Sized + Serialize>(self, _: &T) -> Result<Self::Ok, Self::Error> {
        Err(SerializeError::custom("expected a sequence"))
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: &T,
    ) -> Result<Self::Ok, Self::Error> {
        Err(SerializeError::custom("expected a sequence"))
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<Self::Ok, Self::Error> {
        Err(SerializeError::custom("expected a sequence"))
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(KnownLengthSequence {
            declared: len.ok_or_else(|| SerializeError::custom("sequence length is required"))?,
            values: Vec::new(),
        })
    }
}

fn cases() -> Result<Vec<(Vec<MarkerTree>, Vec<&'static str>)>> {
    let windows = MarkerTree::from_str("sys_platform == 'win32'")?;
    let posix = MarkerTree::from_str("os_name == 'posix'")?;
    Ok(vec![
        (vec![], vec![]),
        (vec![MarkerTree::TRUE], vec![]),
        (vec![MarkerTree::TRUE, MarkerTree::TRUE], vec![]),
        (vec![MarkerTree::FALSE], vec!["python_version < '0'"]),
        (vec![windows], vec!["sys_platform == 'win32'"]),
        (
            vec![posix, windows, posix],
            vec![
                "os_name == 'posix'",
                "sys_platform == 'win32'",
                "os_name == 'posix'",
            ],
        ),
        (
            vec![
                MarkerTree::TRUE,
                windows,
                MarkerTree::TRUE,
                posix,
                MarkerTree::FALSE,
                windows,
                MarkerTree::TRUE,
            ],
            vec![
                "sys_platform == 'win32'",
                "os_name == 'posix'",
                "python_version < '0'",
                "sys_platform == 'win32'",
            ],
        ),
    ])
}

#[test]
fn serialize_declares_emitted_length() -> Result<()> {
    for (markers, expected) in cases()? {
        let environments = SupportedEnvironments::from_markers(markers);
        assert_eq!(environments.serialize(KnownLengthSerializer)?, expected);
    }
    Ok(())
}

#[test]
fn serialize_preserves_json_contents_and_order() -> Result<()> {
    for (markers, expected) in cases()? {
        let environments = SupportedEnvironments::from_markers(markers);
        assert_eq!(
            serde_json::to_string(&environments)?,
            serde_json::to_string(&expected)?
        );
    }
    Ok(())
}

#[test]
fn known_length_serializer_rejects_unknown_or_incorrect_lengths() {
    assert_eq!(
        KnownLengthSerializer
            .serialize_seq(None)
            .err()
            .map(|error| error.to_string()),
        Some("sequence length is required".to_owned())
    );
    assert_eq!(
        KnownLengthSerializer
            .serialize_seq(Some(1))
            .and_then(SerializeSeq::end)
            .map_err(|error| error.to_string()),
        Err("declared 1 elements but serialized 0".to_owned())
    );
}
