use alloc::string::String;
use core::cmp::Ordering;
use serde::de::{Deserialize, Deserializer, Error as DeError, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

pub const STRUCTURED_GENERATION_CAPABILITY_ID: &str = "structured.generate";
pub const MAX_STRUCTURED_SCHEMA_BYTES: usize = 16 * 1024;
pub const MAX_STRUCTURED_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_SCHEMA_DEPTH: usize = 16;
const MAX_OUTPUT_DEPTH: usize = 32;
const MAX_SCHEMA_NODES: usize = 256;
const MAX_OUTPUT_NODES: usize = 4096;
const MAX_CONTAINER_ITEMS: usize = 256;
const MAX_STRING_LENGTH: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StructuredOutputError {
    EmptySchema,
    SchemaTooLarge,
    MalformedSchema,
    UnsupportedSchema,
    InvalidSchema,
    OutputTooLarge,
    MalformedOutput,
    OutputRejected,
    ComplexityLimit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// A bounded JSON Schema subset for structured model output.
///
/// References, arbitrary regular expressions, and unknown keywords are rejected.
/// The three pattern forms used by `NagiPlan@1` are checked by fixed recognizers.
pub struct StructuredOutputSchema {
    document: Value,
}

impl StructuredOutputSchema {
    pub fn parse_json(bytes: &[u8]) -> Result<Self, StructuredOutputError> {
        if bytes.is_empty() {
            return Err(StructuredOutputError::EmptySchema);
        }
        if bytes.len() > MAX_STRUCTURED_SCHEMA_BYTES {
            return Err(StructuredOutputError::SchemaTooLarge);
        }
        let document: UniqueJsonValue =
            serde_json::from_slice(bytes).map_err(|_| StructuredOutputError::MalformedSchema)?;
        let document = document.0;
        let mut nodes = 0;
        validate_schema_node(&document, 0, &mut nodes)?;
        Ok(Self { document })
    }

    pub fn document(&self) -> &Value {
        &self.document
    }

    pub fn validate_json(&self, bytes: &[u8]) -> Result<(), StructuredOutputError> {
        if bytes.len() > MAX_STRUCTURED_OUTPUT_BYTES {
            return Err(StructuredOutputError::OutputTooLarge);
        }
        let value: UniqueJsonValue =
            serde_json::from_slice(bytes).map_err(|_| StructuredOutputError::MalformedOutput)?;
        validate_output_complexity(&value.0)?;
        let mut nodes = 0;
        validate_value(&self.document, &value.0, 0, &mut nodes)
    }
}

struct UniqueJsonValue(Value);

impl<'de> Deserialize<'de> for UniqueJsonValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueJsonValueVisitor)
    }
}

struct UniqueJsonValueVisitor;

impl<'de> Visitor<'de> for UniqueJsonValueVisitor {
    type Value = UniqueJsonValue;

    fn expecting(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("a JSON value with unique object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        let number = Number::from_f64(value).ok_or_else(|| E::custom("invalid JSON number"))?;
        Ok(UniqueJsonValue(Value::Number(number)))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::String(String::from(value))))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::String(value)))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Null))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Null))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = alloc::vec::Vec::new();
        while let Some(value) = sequence.next_element::<UniqueJsonValue>()? {
            values.push(value.0);
        }
        Ok(UniqueJsonValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(A::Error::custom("duplicate JSON object key"));
            }
            let value = map.next_value::<UniqueJsonValue>()?;
            values.insert(key, value.0);
        }
        Ok(UniqueJsonValue(Value::Object(values)))
    }
}

fn validate_schema_node(
    schema: &Value,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), StructuredOutputError> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(StructuredOutputError::ComplexityLimit);
    }
    *nodes = nodes
        .checked_add(1)
        .ok_or(StructuredOutputError::ComplexityLimit)?;
    if *nodes > MAX_SCHEMA_NODES {
        return Err(StructuredOutputError::ComplexityLimit);
    }

    let object = schema
        .as_object()
        .ok_or(StructuredOutputError::InvalidSchema)?;
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "$schema"
                | "$id"
                | "title"
                | "description"
                | "type"
                | "const"
                | "enum"
                | "pattern"
                | "properties"
                | "required"
                | "additionalProperties"
                | "propertyNames"
                | "items"
                | "not"
                | "minLength"
                | "maxLength"
                | "minItems"
                | "maxItems"
                | "minProperties"
                | "maxProperties"
                | "uniqueItems"
                | "minimum"
                | "maximum"
        ) {
            return Err(StructuredOutputError::UnsupportedSchema);
        }
    }

    for key in ["$schema", "$id", "title", "description"] {
        if object
            .get(key)
            .is_some_and(|value| value.as_str().is_none_or(|text| text.len() > 512))
        {
            return Err(StructuredOutputError::InvalidSchema);
        }
    }

    if let Some(kind) = object.get("type") {
        if !valid_type_value(kind) {
            return Err(StructuredOutputError::UnsupportedSchema);
        }
    }
    if let Some(pattern) = object.get("pattern") {
        let Some(pattern) = pattern.as_str() else {
            return Err(StructuredOutputError::InvalidSchema);
        };
        if !supported_pattern(pattern) {
            return Err(StructuredOutputError::UnsupportedSchema);
        }
        if object.get("type").and_then(Value::as_str) != Some("string") {
            return Err(StructuredOutputError::InvalidSchema);
        }
    }
    if let Some(values) = object.get("enum") {
        if values
            .as_array()
            .is_none_or(|values| values.is_empty() || values.len() > MAX_CONTAINER_ITEMS)
        {
            return Err(StructuredOutputError::InvalidSchema);
        }
    }
    if let Some(required) = object.get("required") {
        let Some(required) = required.as_array() else {
            return Err(StructuredOutputError::InvalidSchema);
        };
        if required.len() > MAX_CONTAINER_ITEMS {
            return Err(StructuredOutputError::ComplexityLimit);
        }
        for (index, name) in required.iter().enumerate() {
            let Some(name) = name.as_str() else {
                return Err(StructuredOutputError::InvalidSchema);
            };
            if name.len() > 128
                || required[..index]
                    .iter()
                    .any(|previous| previous.as_str() == Some(name))
            {
                return Err(StructuredOutputError::InvalidSchema);
            }
        }
    }
    for key in [
        "minLength",
        "maxLength",
        "minItems",
        "maxItems",
        "minProperties",
        "maxProperties",
    ] {
        if object.get(key).is_some_and(|value| {
            value
                .as_u64()
                .is_none_or(|bound| bound > MAX_STRING_LENGTH as u64)
        }) {
            return Err(StructuredOutputError::InvalidSchema);
        }
    }
    if object
        .get("uniqueItems")
        .is_some_and(|value| value.as_bool().is_none())
    {
        return Err(StructuredOutputError::InvalidSchema);
    }

    for (minimum, maximum) in [
        ("minLength", "maxLength"),
        ("minItems", "maxItems"),
        ("minProperties", "maxProperties"),
    ] {
        if let (Some(minimum), Some(maximum)) = (
            object.get(minimum).and_then(Value::as_u64),
            object.get(maximum).and_then(Value::as_u64),
        ) {
            if minimum > maximum {
                return Err(StructuredOutputError::InvalidSchema);
            }
        }
    }

    let has_string_limits = object.contains_key("minLength") || object.contains_key("maxLength");
    let has_array_limits = ["minItems", "maxItems", "uniqueItems", "items"]
        .iter()
        .any(|key| object.contains_key(*key));
    let has_object_limits = [
        "minProperties",
        "maxProperties",
        "properties",
        "required",
        "additionalProperties",
        "propertyNames",
    ]
    .iter()
    .any(|key| object.contains_key(*key));
    let has_numeric_limits = object.contains_key("minimum") || object.contains_key("maximum");
    if (has_string_limits && !optional_type_includes(object.get("type"), "string"))
        || (has_array_limits && !optional_type_includes(object.get("type"), "array"))
        || (has_object_limits && !optional_type_includes(object.get("type"), "object"))
        || (has_numeric_limits
            && (!optional_type_includes(object.get("type"), "integer")
                || optional_type_includes(object.get("type"), "number")))
    {
        return Err(StructuredOutputError::InvalidSchema);
    }
    for key in ["minimum", "maximum"] {
        if object
            .get(key)
            .is_some_and(|value| value.as_i64().is_none() && value.as_u64().is_none())
        {
            return Err(StructuredOutputError::UnsupportedSchema);
        }
    }
    if let (Some(minimum), Some(maximum)) = (object.get("minimum"), object.get("maximum")) {
        if compare_integer_values(minimum, maximum)
            .is_none_or(|ordering| ordering == Ordering::Greater)
        {
            return Err(StructuredOutputError::InvalidSchema);
        }
    }

    if let Some(properties) = object.get("properties") {
        let Some(properties) = properties.as_object() else {
            return Err(StructuredOutputError::InvalidSchema);
        };
        if properties.len() > MAX_CONTAINER_ITEMS {
            return Err(StructuredOutputError::ComplexityLimit);
        }
        for (name, property_schema) in properties {
            if name.len() > 128 {
                return Err(StructuredOutputError::InvalidSchema);
            }
            validate_schema_node(property_schema, depth + 1, nodes)?;
        }
    }
    for key in ["items", "propertyNames", "not"] {
        if let Some(nested) = object.get(key) {
            validate_schema_node(nested, depth + 1, nodes)?;
        }
    }
    if let Some(additional) = object.get("additionalProperties") {
        match additional {
            Value::Bool(_) => {}
            Value::Object(_) => validate_schema_node(additional, depth + 1, nodes)?,
            _ => return Err(StructuredOutputError::InvalidSchema),
        }
    }

    if object.get("additionalProperties") == Some(&Value::Bool(false)) {
        if let Some(required) = object.get("required").and_then(Value::as_array) {
            let properties = object.get("properties").and_then(Value::as_object);
            if required.iter().any(|name| {
                name.as_str().is_none_or(|name| {
                    !properties.is_some_and(|properties| properties.contains_key(name))
                })
            }) {
                return Err(StructuredOutputError::InvalidSchema);
            }
        }
    }
    Ok(())
}

fn valid_type_value(value: &Value) -> bool {
    match value {
        Value::String(kind) => supported_type(kind),
        Value::Array(kinds) => {
            !kinds.is_empty()
                && kinds.len() <= 8
                && kinds
                    .iter()
                    .all(|kind| kind.as_str().is_some_and(supported_type))
        }
        _ => false,
    }
}

fn supported_type(kind: &str) -> bool {
    matches!(
        kind,
        "object" | "array" | "string" | "integer" | "number" | "boolean" | "null"
    )
}

fn type_value_includes(value: &Value, expected: &str) -> bool {
    match value {
        Value::String(kind) => kind == expected,
        Value::Array(kinds) => kinds.iter().any(|kind| kind.as_str() == Some(expected)),
        _ => false,
    }
}

fn optional_type_includes(value: Option<&Value>, expected: &str) -> bool {
    value.is_some_and(|kind| type_value_includes(kind, expected))
}

fn supported_pattern(pattern: &str) -> bool {
    matches!(
        pattern,
        "^[a-z0-9_-]+(\\.[a-z0-9_-]+)*$"
            | "^[a-z][a-z0-9_]*$"
            | "(path|command|shell|argv|script|executable)"
    )
}

fn validate_output_complexity(value: &Value) -> Result<(), StructuredOutputError> {
    fn visit(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), StructuredOutputError> {
        if depth > MAX_OUTPUT_DEPTH {
            return Err(StructuredOutputError::ComplexityLimit);
        }
        *nodes = nodes
            .checked_add(1)
            .ok_or(StructuredOutputError::ComplexityLimit)?;
        if *nodes > MAX_OUTPUT_NODES {
            return Err(StructuredOutputError::ComplexityLimit);
        }
        match value {
            Value::Array(items) => {
                if items.len() > MAX_CONTAINER_ITEMS {
                    return Err(StructuredOutputError::ComplexityLimit);
                }
                for item in items {
                    visit(item, depth + 1, nodes)?;
                }
            }
            Value::Object(properties) => {
                if properties.len() > MAX_CONTAINER_ITEMS {
                    return Err(StructuredOutputError::ComplexityLimit);
                }
                for property_value in properties.values() {
                    visit(property_value, depth + 1, nodes)?;
                }
            }
            Value::String(text) if text.len() > MAX_STRING_LENGTH => {
                return Err(StructuredOutputError::ComplexityLimit)
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
        Ok(())
    }

    let mut nodes = 0;
    visit(value, 0, &mut nodes)
}

fn validate_value(
    schema: &Value,
    value: &Value,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), StructuredOutputError> {
    if depth > MAX_OUTPUT_DEPTH {
        return Err(StructuredOutputError::ComplexityLimit);
    }
    *nodes = nodes
        .checked_add(1)
        .ok_or(StructuredOutputError::ComplexityLimit)?;
    if *nodes > MAX_OUTPUT_NODES {
        return Err(StructuredOutputError::ComplexityLimit);
    }
    let object = schema
        .as_object()
        .ok_or(StructuredOutputError::InvalidSchema)?;

    if let Some(kind) = object.get("type") {
        let matches_type = match kind {
            Value::String(kind) => value_matches_type(value, kind),
            Value::Array(kinds) => kinds
                .iter()
                .filter_map(Value::as_str)
                .any(|kind| value_matches_type(value, kind)),
            _ => false,
        };
        if !matches_type {
            return Err(StructuredOutputError::OutputRejected);
        }
    }
    if object
        .get("const")
        .is_some_and(|expected| expected != value)
    {
        return Err(StructuredOutputError::OutputRejected);
    }
    if object.get("enum").is_some_and(|values| {
        values
            .as_array()
            .is_none_or(|values| !values.iter().any(|expected| expected == value))
    }) {
        return Err(StructuredOutputError::OutputRejected);
    }
    if let Some(pattern) = object.get("pattern").and_then(Value::as_str) {
        let Some(text) = value.as_str() else {
            return Err(StructuredOutputError::OutputRejected);
        };
        if !pattern_matches(pattern, text).ok_or(StructuredOutputError::UnsupportedSchema)? {
            return Err(StructuredOutputError::OutputRejected);
        }
    }
    if let Some(negative) = object.get("not") {
        match validate_value(negative, value, depth + 1, nodes) {
            Ok(()) => return Err(StructuredOutputError::OutputRejected),
            Err(StructuredOutputError::OutputRejected) => {}
            Err(error) => return Err(error),
        }
    }

    match value {
        Value::String(text) => {
            let length = text.chars().count();
            check_min_max(object, length, "minLength", "maxLength")?;
        }
        Value::Array(items) => {
            if items.len() > MAX_CONTAINER_ITEMS {
                return Err(StructuredOutputError::ComplexityLimit);
            }
            check_min_max(object, items.len(), "minItems", "maxItems")?;
            if object.get("uniqueItems") == Some(&Value::Bool(true)) {
                for index in 0..items.len() {
                    if items[..index]
                        .iter()
                        .any(|previous| previous == &items[index])
                    {
                        return Err(StructuredOutputError::OutputRejected);
                    }
                }
            }
            if let Some(item_schema) = object.get("items") {
                for item in items {
                    validate_value(item_schema, item, depth + 1, nodes)?;
                }
            }
        }
        Value::Object(properties) => {
            if properties.len() > MAX_CONTAINER_ITEMS {
                return Err(StructuredOutputError::ComplexityLimit);
            }
            check_min_max(object, properties.len(), "minProperties", "maxProperties")?;
            if let Some(required) = object.get("required").and_then(Value::as_array) {
                if required.iter().any(|name| {
                    name.as_str()
                        .is_none_or(|name| !properties.contains_key(name))
                }) {
                    return Err(StructuredOutputError::OutputRejected);
                }
            }
            let declared = object.get("properties").and_then(Value::as_object);
            let additional = object.get("additionalProperties");
            let property_names = object.get("propertyNames");
            for (name, property_value) in properties {
                if let Some(name_schema) = property_names {
                    validate_value(
                        name_schema,
                        &Value::String(String::from(name.as_str())),
                        depth + 1,
                        nodes,
                    )?;
                }
                if let Some(property_schema) = declared.and_then(|declared| declared.get(name)) {
                    validate_value(property_schema, property_value, depth + 1, nodes)?;
                } else {
                    match additional {
                        Some(Value::Bool(false)) => {
                            return Err(StructuredOutputError::OutputRejected)
                        }
                        Some(Value::Object(_)) => {
                            validate_value(additional.unwrap(), property_value, depth + 1, nodes)?;
                        }
                        _ => {}
                    }
                }
            }
        }
        Value::Number(number) => {
            if let Some(minimum) = object.get("minimum") {
                if compare_integer_values(&Value::Number(number.clone()), minimum)
                    .is_none_or(|ordering| ordering == Ordering::Less)
                {
                    return Err(StructuredOutputError::OutputRejected);
                }
            }
            if let Some(maximum) = object.get("maximum") {
                if compare_integer_values(&Value::Number(number.clone()), maximum)
                    .is_none_or(|ordering| ordering == Ordering::Greater)
                {
                    return Err(StructuredOutputError::OutputRejected);
                }
            }
        }
        Value::Null | Value::Bool(_) => {}
    }
    Ok(())
}

fn value_matches_type(value: &Value, kind: &str) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    }
}

fn check_min_max(
    object: &serde_json::Map<String, Value>,
    actual: usize,
    minimum: &str,
    maximum: &str,
) -> Result<(), StructuredOutputError> {
    let actual = actual as u64;
    if object
        .get(minimum)
        .and_then(Value::as_u64)
        .is_some_and(|minimum| actual < minimum)
        || object
            .get(maximum)
            .and_then(Value::as_u64)
            .is_some_and(|maximum| actual > maximum)
    {
        return Err(StructuredOutputError::OutputRejected);
    }
    Ok(())
}

fn compare_integer_values(left: &Value, right: &Value) -> Option<Ordering> {
    let left_signed = left.as_i64();
    let right_signed = right.as_i64();
    let left_unsigned = left.as_u64();
    let right_unsigned = right.as_u64();
    match (left_signed, left_unsigned, right_signed, right_unsigned) {
        (_, Some(left), _, Some(right)) => Some(left.cmp(&right)),
        (Some(left), _, Some(right), _) => Some(left.cmp(&right)),
        (Some(left), _, _, Some(right)) => Some(i128::from(left).cmp(&i128::from(right))),
        (_, Some(left), Some(right), _) => Some(i128::from(left).cmp(&i128::from(right))),
        _ => None,
    }
}

fn pattern_matches(pattern: &str, value: &str) -> Option<bool> {
    Some(match pattern {
        "^[a-z0-9_-]+(\\.[a-z0-9_-]+)*$" => {
            let mut segment_has_character = false;
            for byte in value.bytes() {
                match byte {
                    b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' => {
                        segment_has_character = true;
                    }
                    b'.' if segment_has_character => segment_has_character = false,
                    _ => return Some(false),
                }
            }
            segment_has_character
        }
        "^[a-z][a-z0-9_]*$" => {
            let mut bytes = value.bytes();
            bytes.next().is_some_and(|first| first.is_ascii_lowercase())
                && bytes
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        }
        "(path|command|shell|argv|script|executable)" => {
            ["path", "command", "shell", "argv", "script", "executable"]
                .iter()
                .any(|forbidden| {
                    value
                        .as_bytes()
                        .windows(forbidden.len())
                        .any(|part| part.eq_ignore_ascii_case(forbidden.as_bytes()))
                })
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{StructuredOutputError, StructuredOutputSchema, MAX_STRUCTURED_OUTPUT_BYTES};
    use alloc::{format, string::String, vec};

    const PLAN_SCHEMA: &[u8] = include_bytes!("../../../schemas/NagiPlan@1.json");

    #[test]
    fn validates_nagi_plan_schema_subset_without_replacing_the_plan_validator() {
        let schema = StructuredOutputSchema::parse_json(PLAN_SCHEMA).expect("plan schema");
        schema
            .validate_json(
                br#"{"plan_version":1,"intent":"search","steps":[{"action":"file.search","object_ids":[4,18446744073709551615],"parameters":{"query":"notes"}}]}"#,
            )
            .expect("valid plan document");
    }

    #[test]
    fn rejects_invalid_plan_shape_and_security_sensitive_parameter_names() {
        let schema = StructuredOutputSchema::parse_json(PLAN_SCHEMA).expect("plan schema");
        for invalid in [
            br#"{"plan_version":2,"intent":"search","steps":[{"action":"file.search"}]}"#.as_slice(),
            br#"{"plan_version":1,"intent":"search","steps":[{"action":"file.search","parameters":{"shell_command":"ls"}}]}"#.as_slice(),
            br#"{"plan_version":1,"intent":"search","steps":[{"action":"FILE.SEARCH"}]}"#.as_slice(),
            br#"{"plan_version":1,"intent":"search","steps":[{"action":"file.search","object_ids":[4,4]}]}"#.as_slice(),
            br#"{"plan_version":1,"intent":"search","steps":[{"action":"file.search","object_ids":[18446744073709551616]}]}"#.as_slice(),
            br#"{"plan_version":1,"intent":"search","steps":[{"action":"file.search","unknown":true}]}"#.as_slice(),
        ] {
            assert_eq!(
                schema.validate_json(invalid),
                Err(StructuredOutputError::OutputRejected)
            );
        }
    }

    #[test]
    fn schema_parser_rejects_unbounded_or_unsupported_definitions() {
        assert_eq!(
            StructuredOutputSchema::parse_json(b""),
            Err(StructuredOutputError::EmptySchema)
        );
        assert_eq!(
            StructuredOutputSchema::parse_json(br#"{"type":"string","pattern":".*"}"#),
            Err(StructuredOutputError::UnsupportedSchema)
        );
        assert_eq!(
            StructuredOutputSchema::parse_json(br#"{"type":"string","maxLength":4,"minLength":5}"#),
            Err(StructuredOutputError::InvalidSchema)
        );
    }

    #[test]
    fn rejects_duplicate_keys_in_schema_and_generated_json() {
        assert_eq!(
            StructuredOutputSchema::parse_json(br#"{"type":"object","type":"string"}"#),
            Err(StructuredOutputError::MalformedSchema)
        );
        let schema =
            StructuredOutputSchema::parse_json(br#"{"type":"object","additionalProperties":true}"#)
                .expect("bounded object schema");
        assert_eq!(
            schema.validate_json(br#"{"result":{"ok":true,"ok":false}}"#),
            Err(StructuredOutputError::MalformedOutput)
        );
    }

    #[test]
    fn output_and_containers_are_bounded_before_acceptance() {
        let schema = StructuredOutputSchema::parse_json(
            br#"{"type":"array","maxItems":1,"items":{"type":"string"}}"#,
        )
        .expect("bounded array schema");
        assert_eq!(
            schema.validate_json(br#"["a","b"]"#),
            Err(StructuredOutputError::OutputRejected)
        );
        assert_eq!(
            schema.validate_json(&vec![b' '; MAX_STRUCTURED_OUTPUT_BYTES + 1]),
            Err(StructuredOutputError::OutputTooLarge)
        );
    }

    #[test]
    fn complexity_limits_cover_unconstrained_output_subtrees() {
        let schema =
            StructuredOutputSchema::parse_json(br#"{"type":"object","additionalProperties":true}"#)
                .expect("unconstrained bounded schema");
        let deeply_nested = format!(
            "{{\"tree\":{}0{}}}",
            "[".repeat(super::MAX_OUTPUT_DEPTH + 1),
            "]".repeat(super::MAX_OUTPUT_DEPTH + 1)
        );
        assert_eq!(
            schema.validate_json(deeply_nested.as_bytes()),
            Err(StructuredOutputError::ComplexityLimit)
        );

        let mut many_nodes = String::from("{\"tree\":[");
        for outer in 0..256 {
            if outer > 0 {
                many_nodes.push(',');
            }
            many_nodes.push('[');
            for inner in 0..16 {
                if inner > 0 {
                    many_nodes.push(',');
                }
                many_nodes.push('0');
            }
            many_nodes.push(']');
        }
        many_nodes.push_str("]}");
        assert_eq!(
            schema.validate_json(many_nodes.as_bytes()),
            Err(StructuredOutputError::ComplexityLimit)
        );
    }

    #[test]
    fn pattern_schemas_reject_non_string_type_unions() {
        assert_eq!(
            StructuredOutputSchema::parse_json(
                br#"{"type":["string","null"],"pattern":"^[a-z][a-z0-9_]*$"}"#
            ),
            Err(StructuredOutputError::InvalidSchema)
        );
    }
}
