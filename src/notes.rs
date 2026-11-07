//! The data model stored in `~/.config/sticky/notes.json`.
//!
//! The file is a JSON object mapping group names to lists of notes. Group
//! order matters (it is the order shown in the manager and the tray menu), so
//! an `IndexMap` is used.

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize};

/// One note. Unknown keys are kept so a round trip does not lose data.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NoteInfo {
    #[serde(default, deserialize_with = "lenient_int")]
    pub x: i32,
    #[serde(default, deserialize_with = "lenient_int")]
    pub y: i32,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_int")]
    pub height: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_int")]
    pub width: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub text: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

pub type NotesLists = IndexMap<String, Vec<NoteInfo>>;

// The Python version could store floating point coordinates (for notes
// placed in the center of the screen), so accept any JSON number.
fn number_to_i32(v: &serde_json::Value) -> Option<i32> {
    v.as_i64().map(|n| n as i32).or_else(|| v.as_f64().map(|f| f.round() as i32))
}

fn lenient_int<'de, D: Deserializer<'de>>(d: D) -> Result<i32, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(number_to_i32(&v).unwrap_or(0))
}

fn lenient_opt_int<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i32>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(number_to_i32(&v))
}

pub fn parse_notes(data: &str) -> Result<NotesLists, serde_json::Error> {
    serde_json::from_str(data)
}

/// Serialize like Python's `json.dumps(..., indent=4)`.
pub fn serialize_notes(lists: &NotesLists) -> String {
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut ser = serde_json::Serializer::with_formatter(&mut out, formatter);
    lists.serialize(&mut ser).expect("notes serialize");
    String::from_utf8(out).expect("utf-8 json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_order_and_unknown_keys() {
        let data = r#"{"Z": [{"x": 10.6, "y": 2, "color": "red", "title": "", "text": "hi", "pinned": true}], "A": []}"#;
        let lists = parse_notes(data).unwrap();
        assert_eq!(lists.keys().collect::<Vec<_>>(), ["Z", "A"]);
        let note = &lists["Z"][0];
        assert_eq!(note.x, 11);
        assert_eq!(note.width, None);
        assert_eq!(note.extra["pinned"], serde_json::Value::Bool(true));
        let again = parse_notes(&serialize_notes(&lists)).unwrap();
        assert_eq!(again, lists);
    }
}
