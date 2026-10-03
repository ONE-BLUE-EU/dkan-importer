fn _normalize_coordinate(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_was_space = false;

    for c in s.chars() {
        if c == ',' {
            out.push('.');
            prev_was_space = false;
        } else if c == '.' {
            // Remove any preceding spaces before decimal point
            while out.ends_with(' ') {
                out.pop();
            }
            out.push('.');
            prev_was_space = false;
        } else if c == ' ' {
            // Only add space if previous character wasn't a space (collapse multiple spaces)
            if !prev_was_space {
                out.push(' ');
            }
            prev_was_space = true;
        } else {
            out.push(c);
            prev_was_space = false;
        }
    }

    return out;
}

fn _parse_coordinate(s: &str) -> Result<loose_dms::Coordinate, Box<dyn std::error::Error>> {
    return loose_dms::parse(s).map_err(|e| e.into());
}

pub fn parse_latitude(s: &str) -> Result<f64, Box<dyn std::error::Error>> {
    let coordinate = _normalize_coordinate(s);
    let result = _parse_coordinate(&coordinate)?;
    return Ok(result.lat);
}

pub fn parse_longitude(s: &str) -> Result<f64, Box<dyn std::error::Error>> {
    // loose_dms will return this as latitude if we don't provide a longtitude.
    // So, i provide a default 0.0 latitude.
    let coordinate = format!("0.0, {}", _normalize_coordinate(s));
    let result = _parse_coordinate(&coordinate)?;
    return Ok(result.lng);
}
