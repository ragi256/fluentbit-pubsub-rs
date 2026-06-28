use crate::error::PluginError;
use rmpv::Value;
use rmpv::decode::read_value;
use std::collections::HashMap;

pub fn rmp_to_json(val: &Value) -> serde_json::Value {
    match val {
        Value::Nil => serde_json::Value::Null,
        Value::Boolean(b) => serde_json::Value::Bool(*b),
        Value::Integer(i) => {
            if let Some(n) = i.as_u64() {
                serde_json::json!(n)
            } else if let Some(n) = i.as_i64() {
                serde_json::json!(n)
            } else {
                serde_json::json!(0)
            }
        }
        Value::F32(f) => serde_json::json!(*f),
        Value::F64(f) => serde_json::json!(*f),
        Value::String(s) => serde_json::Value::String(s.as_str().unwrap_or("").to_string()),
        Value::Binary(b) => serde_json::json!(std::str::from_utf8(b).unwrap_or("")),
        Value::Array(a) => serde_json::Value::Array(a.iter().map(rmp_to_json).collect()),
        Value::Map(m) => {
            let mut map = serde_json::Map::new();
            for (k, v) in m {
                let key_str = match k {
                    Value::String(s) => s.as_str().unwrap_or("").to_string(),
                    _ => format!("{:?}", k),
                };
                map.insert(key_str, rmp_to_json(v));
            }
            serde_json::Value::Object(map)
        }
        Value::Ext(_, _) => serde_json::Value::Null,
    }
}

pub type RecordList = Vec<(u64, HashMap<String, Value>)>;

pub fn decode_records(data: &[u8]) -> Result<RecordList, PluginError> {
    let mut cursor = std::io::Cursor::new(data);
    let mut records = Vec::new();

    while cursor.position() < data.len() as u64 {
        let val = read_value(&mut cursor)
            .map_err(|e| PluginError::Codec(format!("decode error: {}", e)))?;

        let arr = match val {
            Value::Array(a) if a.len() == 2 => a,
            _ => continue,
        };

        let ts = match &arr[0] {
            Value::Integer(i) => i.as_u64().unwrap_or(0),
            _ => 0,
        };

        let map_val = match &arr[1] {
            Value::Map(m) => m,
            _ => continue,
        };

        let map: HashMap<String, Value> = map_val
            .iter()
            .filter_map(|(k, v)| {
                if let Value::String(s) = k {
                    s.as_str().map(|s_str| (s_str.to_string(), v.clone()))
                } else {
                    None
                }
            })
            .collect();

        records.push((ts, map));
    }

    Ok(records)
}

pub fn convert_to_json_bytes(record: &HashMap<String, Value>) -> Result<Vec<u8>, PluginError> {
    let mapped: serde_json::Map<String, serde_json::Value> = record
        .iter()
        .map(|(k, v)| (k.clone(), rmp_to_json(v)))
        .collect();

    let json = serde_json::Value::Object(mapped);
    serde_json::to_vec(&json).map_err(|e| PluginError::Codec(format!("Serialize error: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmpv::Value;
    use std::collections::HashMap;

    #[test]
    fn test_rmp_to_json() {
        assert_eq!(rmp_to_json(&Value::Nil), serde_json::Value::Null);
        assert_eq!(
            rmp_to_json(&Value::Boolean(true)),
            serde_json::Value::Bool(true)
        );
        assert_eq!(
            rmp_to_json(&Value::Integer(rmpv::Integer::from(42))),
            serde_json::json!(42)
        );
        assert_eq!(
            rmp_to_json(&Value::Integer(rmpv::Integer::from(-10))),
            serde_json::json!(-10)
        );
        assert_eq!(rmp_to_json(&Value::F32(3.5)), serde_json::json!(3.5_f32));
        assert_eq!(rmp_to_json(&Value::F64(2.71)), serde_json::json!(2.71));
        assert_eq!(
            rmp_to_json(&Value::String("hello".into())),
            serde_json::json!("hello")
        );
        assert_eq!(
            rmp_to_json(&Value::Binary(b"world".to_vec())),
            serde_json::json!("world")
        );

        // Array test
        let arr = Value::Array(vec![Value::Integer(1.into()), Value::String("two".into())]);
        assert_eq!(rmp_to_json(&arr), serde_json::json!([1, "two"]));

        // Map test
        let map = vec![
            (Value::String("key1".into()), Value::Boolean(false)),
            (Value::Integer(123.into()), Value::String("val".into())), // Non-string keys get converted to their debug representation, but integer key doesn't work well with json macro directly, so let's test a simple string key map.
        ];
        let rmp_map = Value::Map(map);
        let expected_json = serde_json::json!({
            "key1": false,
            "Integer(PosInt(123))": "val" // Given current implementation, this is what `format!("{:?}", k)` outputs for Integer
        });
        assert_eq!(rmp_to_json(&rmp_map), expected_json);
    }

    #[test]
    fn test_decode_records() {
        // Create a Fluent Bit MessagePack payload: [ [ timestamp, { "message": "hello test" } ] ]
        let timestamp = 1678888888_u64;
        let map = Value::Map(vec![(
            Value::String("message".into()),
            Value::String("hello test".into()),
        )]);
        let record = Value::Array(vec![Value::Integer(timestamp.into()), map]);

        let mut buf = Vec::new();
        rmpv::encode::write_value(&mut buf, &record).unwrap();

        let decoded = decode_records(&buf).unwrap();
        assert_eq!(decoded.len(), 1);

        let (ts, record_map) = &decoded[0];
        assert_eq!(*ts, timestamp);
        assert_eq!(
            record_map.get("message").unwrap(),
            &Value::String("hello test".into())
        );
    }

    #[test]
    fn test_decode_records_multiple() {
        // [ [ ts1, { "k1": "v1" } ], [ ts2, { "k2": "v2" } ] ]
        // Note: decode_records expects the data to be a stream of arrays, not an array of arrays!
        // FluentBit sends them concatenated: Array(ts, map) Array(ts, map)
        let map1 = Value::Map(vec![(
            Value::String("k1".into()),
            Value::String("v1".into()),
        )]);
        let rec1 = Value::Array(vec![Value::Integer(100.into()), map1]);

        let map2 = Value::Map(vec![(
            Value::String("k2".into()),
            Value::String("v2".into()),
        )]);
        let rec2 = Value::Array(vec![Value::Integer(200.into()), map2]);

        let mut buf = Vec::new();
        rmpv::encode::write_value(&mut buf, &rec1).unwrap();
        rmpv::encode::write_value(&mut buf, &rec2).unwrap();

        let decoded = decode_records(&buf).unwrap();
        assert_eq!(decoded.len(), 2);

        assert_eq!(decoded[0].0, 100);
        assert_eq!(decoded[0].1.get("k1").unwrap(), &Value::String("v1".into()));

        assert_eq!(decoded[1].0, 200);
        assert_eq!(decoded[1].1.get("k2").unwrap(), &Value::String("v2".into()));
    }

    #[test]
    fn test_convert_to_json_bytes() {
        let mut record = HashMap::new();
        record.insert("field1".to_string(), Value::String("value1".into()));
        record.insert("field2".to_string(), Value::Integer(100.into()));

        let json_bytes = convert_to_json_bytes(&record).expect("Failed to convert to json");
        let parsed_json: serde_json::Value = serde_json::from_slice(&json_bytes).unwrap();

        assert_eq!(parsed_json["field1"], "value1");
        assert_eq!(parsed_json["field2"], 100);
    }
}
