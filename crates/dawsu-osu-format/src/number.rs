//! Numbers read the way lazer reads them (Responsibility 8): lazer's `Parsing` class at the
//! pin, `osu.Game/Beatmaps/Formats/Parsing.cs:14-50` (ppy/osu 131fe72, unchanged since b77b028).

/// `Parsing.MAX_PARSE_VALUE`: `int.MaxValue`.
pub const MAX_PARSE_VALUE: f64 = i32::MAX as f64;
/// `Parsing.MAX_COORDINATE_VALUE`.
pub const MAX_COORDINATE_VALUE: f64 = 131_072.0;

/// .NET's number parsing allows leading and trailing white space.
fn trim(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_ascii_whitespace() || c == '\u{000B}')
}

/// `Parsing.ParseInt`: a whole number; a decimal point fails, as `int.Parse` does.
pub fn parse_int(s: &str) -> Result<i32, String> {
    let t = trim(s);
    let v: i64 = t.parse().map_err(|_| format!("\"{s}\" is not a whole number"))?;
    let limit = i64::from(i32::MAX);
    if v < -limit {
        return Err(format!("\"{s}\" is too low"));
    }
    if v > limit {
        return Err(format!("\"{s}\" is too high"));
    }
    Ok(v as i32)
}

fn parse_decimal(s: &str) -> Result<f64, String> {
    let t = trim(s);
    // Rust also accepts "inf" and "infinity"; they fail the range check below, as in lazer.
    t.parse::<f64>().map_err(|_| format!("\"{s}\" is not a number"))
}

/// `Parsing.ParseDouble`.
pub fn parse_double(s: &str, limit: f64, allow_nan: bool) -> Result<f64, String> {
    let v = parse_decimal(s)?;
    if v < -limit {
        return Err(format!("\"{s}\" is too low"));
    }
    if v > limit {
        return Err(format!("\"{s}\" is too high"));
    }
    if !allow_nan && v.is_nan() {
        return Err(format!("\"{s}\" is not a number"));
    }
    Ok(v)
}

/// `Parsing.ParseFloat`.
pub fn parse_float(s: &str, limit: f64) -> Result<f32, String> {
    let t = trim(s);
    let v: f32 = t.parse().map_err(|_| format!("\"{s}\" is not a number"))?;
    if f64::from(v) < -limit {
        return Err(format!("\"{s}\" is too low"));
    }
    if f64::from(v) > limit {
        return Err(format!("\"{s}\" is too high"));
    }
    if v.is_nan() {
        return Err(format!("\"{s}\" is not a number"));
    }
    Ok(v)
}

/// C#'s `int.TryParse(s, out int v)`: 0 when it cannot be read.
pub fn try_parse_int(s: &str) -> i32 {
    trim(s).parse::<i32>().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_numbers_reject_decimals() {
        assert_eq!(parse_int("2"), Ok(2));
        assert_eq!(parse_int(" -3 "), Ok(-3));
        assert!(parse_int("2.5").is_err());
        assert!(parse_int("1.0").is_err());
        assert!(parse_int("").is_err());
        assert!(parse_int("-2147483648").is_err(), "below -int.MaxValue");
        assert!(parse_int("2147483648").is_err());
    }

    #[test]
    fn decimals_and_limits() {
        assert_eq!(parse_double("333.33", MAX_PARSE_VALUE, false), Ok(333.33));
        assert!(parse_double("NaN", MAX_PARSE_VALUE, false).is_err());
        assert!(parse_double("NaN", MAX_PARSE_VALUE, true).unwrap().is_nan());
        assert!(parse_double("inf", MAX_PARSE_VALUE, true).is_err());
        assert!(parse_double("200000", MAX_COORDINATE_VALUE, false).is_err());
        assert_eq!(parse_float("256.7", MAX_COORDINATE_VALUE), Ok(256.7));
    }

    #[test]
    fn try_parse_gives_zero() {
        assert_eq!(try_parse_int("8"), 8);
        assert_eq!(try_parse_int("2.5"), 0);
        assert_eq!(try_parse_int(""), 0);
    }
}
