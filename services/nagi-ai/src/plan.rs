use alloc::{collections::BTreeMap, string::String, vec::Vec};
use serde::Deserialize;
use serde_json::Value;

pub const NAGI_PLAN_VERSION: u16 = 1;
pub const MAX_PLAN_JSON_BYTES: usize = 64 * 1024;
pub const MAX_PLAN_STEPS: usize = 16;
pub const MAX_INTENT_BYTES: usize = 128;
pub const MAX_PARAMETERS_PER_STEP: usize = 32;
pub const MAX_OBJECTS_PER_STEP: usize = 64;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NagiPlan {
    pub plan_version: u16,
    pub intent: String,
    pub steps: Vec<PlanStep>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlanStep {
    pub action: String,
    #[serde(default)]
    pub object_ids: Vec<u64>,
    #[serde(default)]
    pub parameters: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanParseError {
    TooLarge,
    InvalidJson,
}

impl NagiPlan {
    /// Parses one complete JSON document. Partial provider streams are never
    /// accepted as executable plans.
    pub fn parse_complete(json: &str) -> Result<Self, PlanParseError> {
        if json.len() > MAX_PLAN_JSON_BYTES {
            return Err(PlanParseError::TooLarge);
        }
        serde_json::from_str(json).map_err(|_| PlanParseError::InvalidJson)
    }
}
