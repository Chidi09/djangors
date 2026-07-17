use minijinja::value::Value;
use minijinja::{Error as MJError, ErrorKind as MJErrorKind};
use std::fmt::Write;

/// Format a date/time using a subset of Django's format specifiers.
///
/// Supported specifiers:
/// - `Y`: 4-digit year (e.g. 2026)
/// - `m`: 2-digit month (e.g. 07)
/// - `d`: 2-digit day of the month (e.g. 17)
/// - `H`: 2-digit hour, 24-hour format (e.g. 18)
/// - `i`: 2-digit minutes (e.g. 02)
/// - `s`: 2-digit seconds (e.g. 05)
///
/// Example:
/// `{{ value|date:"Y-m-d H:i:s" }}`
pub fn date(value: Value, format_str: Option<String>) -> Result<String, MJError> {
    let format_str = format_str.unwrap_or_else(|| "Y-m-d".to_string());

    let dt_str = value.to_string();
    let dt = if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(&dt_str) {
        parsed.with_timezone(&chrono::Utc)
    } else if let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(&dt_str, "%Y-%m-%d %H:%M:%S") {
        chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(parsed, chrono::Utc)
    } else if let Ok(parsed) = chrono::NaiveDate::parse_from_str(&dt_str, "%Y-%m-%d") {
        let ndt = parsed.and_hms_opt(0, 0, 0).unwrap();
        chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(ndt, chrono::Utc)
    } else {
        return Err(MJError::new(
            MJErrorKind::InvalidOperation,
            format!("date filter: could not parse date/time value: {}", dt_str),
        ));
    };

    let mut result = String::new();
    for c in format_str.chars() {
        match c {
            'Y' => write!(&mut result, "{:04}", dt.format("%Y")).unwrap(),
            'm' => write!(&mut result, "{:02}", dt.format("%m")).unwrap(),
            'd' => write!(&mut result, "{:02}", dt.format("%d")).unwrap(),
            'H' => write!(&mut result, "{:02}", dt.format("%H")).unwrap(),
            'i' => write!(&mut result, "{:02}", dt.format("%M")).unwrap(),
            's' => write!(&mut result, "{:02}", dt.format("%S")).unwrap(),
            other => result.push(other),
        }
    }

    Ok(result)
}

/// Round a float to N decimal places, or Django's default (1 decimal, trailing zeros trimmed)
/// if no argument is given.
///
/// Example:
/// `{{ value|floatformat:2 }}`
pub fn floatformat(value: Value, arg: Option<Value>) -> Result<String, MJError> {
    let num = match value.as_str() {
        Some(s) => s.parse::<f64>().map_err(|e| {
            MJError::new(
                MJErrorKind::InvalidOperation,
                format!("floatformat filter: value must be a number: {}", e),
            )
        })?,
        None => {
            if let Ok(f) = f64::try_from(value.clone()) {
                f
            } else if let Ok(i) = i64::try_from(value.clone()) {
                i as f64
            } else {
                return Err(MJError::new(
                    MJErrorKind::InvalidOperation,
                    format!("floatformat filter: value must be a number: {}", value),
                ));
            }
        }
    };

    if let Some(arg_val) = arg {
        let places = i32::try_from(arg_val).map_err(|e| {
            MJError::new(
                MJErrorKind::InvalidOperation,
                format!("floatformat filter: argument must be an integer: {}", e),
            )
        })?;

        if places < 0 {
            let abs_places = places.unsigned_abs() as usize;
            let formatted = format!("{:.*}", abs_places, num);
            if formatted.contains('.') {
                let parts: Vec<&str> = formatted.split('.').collect();
                if parts.len() == 2 && parts[1].chars().all(|c| c == '0') {
                    Ok(format!("{:.0}", num))
                } else {
                    Ok(formatted)
                }
            } else {
                Ok(formatted)
            }
        } else {
            let formatted = format!("{:.*}", places as usize, num);
            Ok(formatted)
        }
    } else {
        let formatted = format!("{:.1}", num);
        if formatted.ends_with(".0") {
            Ok(formatted[..formatted.len() - 2].to_string())
        } else {
            Ok(formatted)
        }
    }
}

/// Pluralize a suffix based on a count.
///
/// Returns "" if count == 1, "s" otherwise (or a custom suffix if one is given as an argument).
/// If custom suffix contains a comma, the part before the comma is for singular, and the part
/// after is for plural (e.g. "y,ies" for dynamic -> dynamics, or cherry -> cherries).
///
/// Example:
/// `{{ count }} item{{ count|pluralize }}`
/// `{{ count }} cherr{{ count|pluralize:"y,ies" }}`
pub fn pluralize(value: Value, arg: Option<String>) -> Result<String, MJError> {
    let count = if let Ok(i) = i64::try_from(value.clone()) {
        i as f64
    } else if let Ok(f) = f64::try_from(value.clone()) {
        f
    } else {
        return Err(MJError::new(
            MJErrorKind::InvalidOperation,
            format!("pluralize filter: value must be a number: {}", value),
        ));
    };

    let is_singular = count == 1.0;

    let arg = arg.unwrap_or_else(|| "s".to_string());
    if let Some((singular_suffix, plural_suffix)) = arg.split_once(',') {
        if is_singular {
            Ok(singular_suffix.to_string())
        } else {
            Ok(plural_suffix.to_string())
        }
    } else {
        if is_singular {
            Ok("".to_string())
        } else {
            Ok(arg)
        }
    }
}

/// Truncate a string to N words, appending "…" if truncation actually happened.
///
/// Example:
/// `{{ value|truncatewords:30 }}`
pub fn truncatewords(value: Value, arg: Value) -> Result<String, MJError> {
    let s = value.to_string();
    let num_words = usize::try_from(arg).map_err(|e| {
        MJError::new(
            MJErrorKind::InvalidOperation,
            format!("truncatewords filter: argument must be an integer: {}", e),
        )
    })?;

    let words: Vec<&str> = s.split_whitespace().collect();
    if words.len() <= num_words {
        Ok(s)
    } else {
        let truncated = words[..num_words].join(" ");
        Ok(format!("{}…", truncated))
    }
}
