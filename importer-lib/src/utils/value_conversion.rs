use serde_json::Value;

/// Convert a serde_json::Value to a string with fixed decimal precision
/// For floating point numbers, always formats with 6 decimal places
/// For integers, keeps them as integers
pub fn serde_json_value_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                // Check if it's a whole number
                if f.fract().abs() < f64::EPSILON {
                    // It's an integer, display without decimals
                    format!("{:.0}", f)
                } else {
                    // It's a decimal, display with 6 decimal places
                    format!("{:.6}", f)
                }
            } else {
                n.to_string()
            }
        }
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        Value::Array(_) => String::new(),
        Value::Object(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_serde_json_value_to_string_string() {
        assert_eq!(serde_json_value_to_string(&json!("hello")), "hello");
    }

    #[test]
    fn test_serde_json_value_to_string_bool() {
        assert_eq!(serde_json_value_to_string(&json!(true)), "true");
        assert_eq!(serde_json_value_to_string(&json!(false)), "false");
    }

    #[test]
    fn test_serde_json_value_to_string_null() {
        assert_eq!(serde_json_value_to_string(&json!(null)), "");
    }

    #[test]
    fn test_serde_json_value_to_string_array() {
        assert_eq!(serde_json_value_to_string(&json!([1, 2, 3])), "");
    }

    #[test]
    fn test_serde_json_value_to_string_object() {
        assert_eq!(serde_json_value_to_string(&json!({"key": "value"})), "");
    }

    #[test]
    fn test_serde_json_value_to_string_integers() {
        assert_eq!(serde_json_value_to_string(&json!(100)), "100");
        assert_eq!(serde_json_value_to_string(&json!(0)), "0");
    }

    #[test]
    fn test_serde_json_value_to_string_floats() {
        assert_eq!(serde_json_value_to_string(&json!(12.43033)), "12.430330");
        assert_eq!(serde_json_value_to_string(&json!(2.5)), "2.500000");
        assert_eq!(serde_json_value_to_string(&json!(1.1)), "1.100000");
    }
}
