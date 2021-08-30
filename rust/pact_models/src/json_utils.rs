//! Collection of utilities for working with JSON

use std::collections::{HashMap, BTreeMap};
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use base64::decode;
use serde::Deserialize;
use serde_json::{self, Map, Value, json};

use crate::bodies::OptionalBody;
use crate::content_types::{ContentType, detect_content_type_from_string};

/// Trait to convert a JSON structure to a number
pub trait JsonToNum<T> {
  /// Converts the JSON field in the map to a Number
  fn json_to_number(map: &serde_json::Map<String, Value>, field: &str, default: T) -> T;
}

impl JsonToNum<i32> for i32 {
  fn json_to_number(map: &serde_json::Map<String, Value>, field: &str, default: i32) -> i32 {
    match map.get(field) {
      Some(val) => match val {
        Value::Number(num) => match num.as_i64() {
          Some(num) => num as i32,
          None => default
        },
        _ => default
      },
      None => default
    }
  }
}

impl JsonToNum<u16> for u16 {
  fn json_to_number(map: &serde_json::Map<String, Value>, field: &str, default: u16) -> u16 {
    match map.get(field) {
      Some(val) => match val {
        Value::Number(num) => match num.as_u64() {
          Some(num) => num as u16,
          None => default
        },
        _ => default
      },
      None => default
    }
  }
}

/// Converts the JSON struct to a String, first checking if it is a JSON String
pub fn json_to_string(value: &Value) -> String {
  match value {
    Value::String(s) => s.clone(),
    _ => value.to_string()
  }
}

/// Converts an optional JSON struct to a usize, returning `None` if it is not a numeric type.
pub fn json_to_num(value: Option<Value>) -> Option<usize> {
  if let Some(value) = value {
    match value {
      Value::Number(n) => if n.is_i64() && n.as_i64().unwrap() > 0 { Some(n.as_i64().unwrap() as usize) }
        else if n.is_f64() { Some(n.as_f64().unwrap() as usize) }
        else if n.is_u64() { Some(n.as_u64().unwrap() as usize) }
        else { None },
      Value::String(ref s) => usize::from_str(&s.clone()).ok(),
      _ => None
    }
  } else {
    None
  }
}

/// Hash function for JSON struct
pub fn hash_json<H: Hasher>(v: &Value, state: &mut H) {
  match v {
    Value::Bool(b) => b.hash(state),
    Value::Number(n) => {
      if let Some(num) = n.as_u64() {
        num.hash(state);
      }
      if let Some(num) = n.as_f64() {
        num.to_string().hash(state);
      }
      if let Some(num) = n.as_i64() {
        num.hash(state);
      }
    }
    Value::String(s) => s.hash(state),
    Value::Array(values) => for value in values {
      hash_json(value, state);
    }
    Value::Object(map) => for (k, v) in map {
      k.hash(state);
      hash_json(v, state);
    }
    _ => ()
  }
}

/// Look up a field and return it as a string value
pub fn get_field_as_string(field: &str, map: &Map<String, Value>) -> Option<String> {
  map.get(field).map(|f| json_to_string(f))
}

/// Returns the headers from a JSON struct as Map String -> Vec<String>
pub fn headers_from_json(request: &Value) -> Option<HashMap<String, Vec<String>>> {
  match request.get("headers") {
    Some(v) => match *v {
      Value::Object(ref m) => Some(m.iter().map(|(key, val)| {
        match val {
          &Value::String(ref s) => (key.clone(), s.clone().split(',').map(|v| v.trim().to_string()).collect()),
          &Value::Array(ref v) => (key.clone(), v.iter().map(|val| {
            match val {
              &Value::String(ref s) => s.clone(),
              _ => val.to_string()
            }
          }).collect()),
          _ => (key.clone(), vec![val.to_string()])
        }
      }).collect()),
      _ => None
    },
    None => None
  }
}

/// Converts the headers map into a JSON struct
pub fn headers_to_json(headers: &HashMap<String, Vec<String>>) -> Value {
  json!(headers.iter().fold(BTreeMap::new(), |mut map, kv| {
    map.insert(kv.0.clone(), Value::String(kv.1.join(", ")));
    map
  }))
}

#[derive(Deserialize)]
#[serde(untagged)]
enum JsonParsable {
  JsonStringValue(String),
  KeyValue(HashMap<String, Value>)
}

/// Returns the body from the JSON struct with the provided field name
pub fn body_from_json(request: &Value, fieldname: &str, headers: &Option<HashMap<String, Vec<String>>>) -> OptionalBody {
  let content_type = match headers {
    &Some(ref h) => match h.iter().find(|kv| kv.0.to_lowercase() == "content-type") {
      Some(kv) => {
        match ContentType::parse(kv.1[0].as_str()) {
          Ok(v) => Some(v),
          Err(_) => None
        }
      },
      None => None
    },
    &None => None
  };

  match request.get(fieldname) {
    Some(v) => match v {
      Value::String(s) => {
        if s.is_empty() {
          OptionalBody::Empty
        } else {
          let content_type = content_type.unwrap_or_else(|| {
            detect_content_type_from_string(s).unwrap_or_default()
          });
          if content_type.is_json() {
            match serde_json::from_str::<JsonParsable>(&s) {
              Ok(_) => OptionalBody::Present(s.clone().into(), Some(content_type), None),
              Err(_) => OptionalBody::Present(format!("\"{}\"", s).into(), Some(content_type), None)
            }
          } else if content_type.is_text() {
            OptionalBody::Present(s.clone().into(), Some(content_type), None)
          } else {
            match decode(s) {
              Ok(bytes) => OptionalBody::Present(bytes.into(), None, None),
              Err(_) => OptionalBody::Present(s.clone().into(), None, None)
            }
          }
        }
      },
      Value::Null => OptionalBody::Null,
      _ => OptionalBody::Present(v.to_string().into(), None, None)
    },
    None => OptionalBody::Missing
  }
}

#[cfg(test)]
mod tests {
  use expectest::expect;
  use expectest::prelude::*;
  use serde_json::json;

  use super::*;

  #[test]
  fn json_to_int_test() {
    expect!(<i32>::json_to_number(&serde_json::Map::new(), "any", 1)).to(be_equal_to(1));
    expect!(<i32>::json_to_number(&json!({ "min": 5 }).as_object().unwrap(), "any", 1)).to(be_equal_to(1));
    expect!(<i32>::json_to_number(&json!({ "min": "5" }).as_object().unwrap(), "min", 1)).to(be_equal_to(1));
    expect!(<i32>::json_to_number(&json!({ "min": 5 }).as_object().unwrap(), "min", 1)).to(be_equal_to(5));
    expect!(<i32>::json_to_number(&json!({ "min": -5 }).as_object().unwrap(), "min", 1)).to(be_equal_to(-5));
    expect!(<i32>::json_to_number(&json!({ "min": 5.0 }).as_object().unwrap(), "min", 1)).to(be_equal_to(1));

    expect!(<u16>::json_to_number(&serde_json::Map::new(), "any", 1)).to(be_equal_to(1));
    expect!(<u16>::json_to_number(&json!({ "min": 5 }).as_object().unwrap(), "any", 1)).to(be_equal_to(1));
    expect!(<u16>::json_to_number(&json!({ "min": "5" }).as_object().unwrap(), "min", 1)).to(be_equal_to(1));
    expect!(<u16>::json_to_number(&json!({ "min": 5 }).as_object().unwrap(), "min", 1)).to(be_equal_to(5));
    expect!(<u16>::json_to_number(&json!({ "min": -5 }).as_object().unwrap(), "min", 1)).to(be_equal_to(1));
    expect!(<u16>::json_to_number(&json!({ "min": 5.0 }).as_object().unwrap(), "min", 1)).to(be_equal_to(1));
  }

  #[test]
  fn json_to_string_test() {
    expect!(json_to_string(&Value::from_str("\"test string\"").unwrap())).to(be_equal_to("test string".to_string()));
    expect!(json_to_string(&Value::from_str("null").unwrap())).to(be_equal_to("null".to_string()));
    expect!(json_to_string(&Value::from_str("100").unwrap())).to(be_equal_to("100".to_string()));
    expect!(json_to_string(&Value::from_str("100.10").unwrap())).to(be_equal_to("100.1".to_string()));
    expect!(json_to_string(&Value::from_str("{}").unwrap())).to(be_equal_to("{}".to_string()));
    expect!(json_to_string(&Value::from_str("[]").unwrap())).to(be_equal_to("[]".to_string()));
    expect!(json_to_string(&Value::from_str("true").unwrap())).to(be_equal_to("true".to_string()));
    expect!(json_to_string(&Value::from_str("false").unwrap())).to(be_equal_to("false".to_string()));
  }

  #[test]
  fn json_to_num_test() {
    expect!(json_to_num(Value::from_str("\"test string\"").ok())).to(be_none());
    expect!(json_to_num(Value::from_str("null").ok())).to(be_none());
    expect!(json_to_num(Value::from_str("{}").ok())).to(be_none());
    expect!(json_to_num(Value::from_str("[]").ok())).to(be_none());
    expect!(json_to_num(Value::from_str("true").ok())).to(be_none());
    expect!(json_to_num(Value::from_str("false").ok())).to(be_none());
    expect!(json_to_num(Value::from_str("100").ok())).to(be_some().value(100));
    expect!(json_to_num(Value::from_str("-100").ok())).to(be_none());
    expect!(json_to_num(Value::from_str("100.10").ok())).to(be_some().value(100));
  }

  #[test]
  fn body_from_text_plain_type_returns_the_same_formatted_body() {
    let json : serde_json::Value = serde_json::from_str(r#"
      {
          "path": "/",
          "query": "",
          "headers": {"Content-Type": "text/plain"},
          "body": "\"This is a string\""
      }
     "#).unwrap();
    let headers = headers_from_json(&json);
    let body = body_from_json(&json, "body", &headers);
    expect!(body).to(be_equal_to(OptionalBody::Present("\"This is a string\"".into(), Some("text/plain".into()), None)));
  }

  #[test]
  fn body_from_text_html_type_returns_the_same_formatted_body() {
    let json : serde_json::Value = serde_json::from_str(r#"
      {
          "path": "/",
          "query": "",
          "headers": {"Content-Type": "text/html"},
          "body": "\"This is a string\""
      }
     "#).unwrap();
    let headers = headers_from_json(&json);
    let body = body_from_json(&json, "body", &headers);
    expect!(body).to(be_equal_to(OptionalBody::Present("\"This is a string\"".into(), Some("text/html".into()), None)));
  }

  #[test]
  fn body_from_json_returns_the_a_json_formatted_body_if_the_body_is_a_string_and_the_content_type_is_json() {
    let json : serde_json::Value = serde_json::from_str(r#"
      {
          "path": "/",
          "query": "",
          "headers": {"Content-Type": "application/json"},
          "body": "This is actually a JSON string"
      }
     "#).unwrap();
    let headers = headers_from_json(&json);
    let body = body_from_json(&json, "body", &headers);
    expect!(body).to(be_equal_to(OptionalBody::Present("\"This is actually a JSON string\"".into(), Some("application/json".into()), None)));
  }

  #[test]
  fn body_from_json_returns_the_a_json_formatted_body_if_the_body_is_a_valid_json_string_and_the_content_type_is_json() {
    let json : serde_json::Value = serde_json::from_str(r#"
      {
          "path": "/",
          "query": "",
          "headers": {"Content-Type": "application/json"},
          "body": "\"This is actually a JSON string\""
      }
     "#).unwrap();
    let headers = headers_from_json(&json);
    let body = body_from_json(&json, "body", &headers);
    expect!(body).to(be_equal_to(OptionalBody::Present("\"This is actually a JSON string\"".into(), Some("application/json".into()), None)));
  }

  #[test]
  fn body_from_json_returns_the_body_if_the_content_type_is_json() {
    let json : serde_json::Value = serde_json::from_str(r#"
      {
          "path": "/",
          "query": "",
          "headers": {"Content-Type": "application/json"},
          "body": "{\"test\":true}"
      }
     "#).unwrap();
    let headers = headers_from_json(&json);
    let body = body_from_json(&json, "body", &headers);
    expect!(body).to(be_equal_to(OptionalBody::Present("{\"test\":true}".into(), Some("application/json".into()), None)));
  }

  #[test]
  fn body_from_json_returns_missing_if_there_is_no_body() {
    let json : serde_json::Value = serde_json::from_str(r#"
      {
          "path": "/",
          "query": "",
          "headers": {},
          "matchingRules": {
            "*.path": {}
          }
      }
     "#).unwrap();
    let body = body_from_json(&json, "body", &None);
    expect!(body).to(be_equal_to(OptionalBody::Missing));
  }

  #[test]
  fn body_from_json_returns_null_if_the_body_is_null() {
    let json : serde_json::Value = serde_json::from_str(r#"
      {
          "path": "/",
          "query": "",
          "headers": {},
          "body": null
      }
     "#).unwrap();
    let body = body_from_json(&json, "body", &None);
    expect!(body).to(be_equal_to(OptionalBody::Null));
  }

  #[test]
  fn body_from_json_returns_json_string_if_the_body_is_json_but_not_a_string() {
    let json : serde_json::Value = serde_json::from_str(r#"
      {
          "path": "/",
          "query": "",
          "headers": {},
          "body": {
            "test": true
          }
      }
     "#).unwrap();
    let body = body_from_json(&json, "body", &None);
    expect!(body).to(be_equal_to(OptionalBody::Present("{\"test\":true}".into(), None, None)));
  }

  #[test]
  fn body_from_json_returns_empty_if_the_body_is_an_empty_string() {
    let json : serde_json::Value = serde_json::from_str(r#"
      {
          "path": "/",
          "query": "",
          "headers": {},
          "body": ""
      }
     "#).unwrap();
    let body = body_from_json(&json, "body", &None);
    expect!(body).to(be_equal_to(OptionalBody::Empty));
  }

  #[test]
  fn body_from_json_returns_the_body_if_the_body_is_a_string() {
    let json : serde_json::Value = serde_json::from_str(r#"
      {
          "path": "/",
          "query": "",
          "headers": {},
          "body": "<?xml version=\"1.0\"?> <body></body>"
      }
     "#).unwrap();
    let body = body_from_json(&json, "body", &None);
    expect!(body).to(be_equal_to(OptionalBody::Present("<?xml version=\"1.0\"?> <body></body>".into(), Some("application/xml".into()), None)));
  }
}
