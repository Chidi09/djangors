//! Forms and validation for the Djangors web framework.
//!
//! Provides the core traits, fields, and error structures for form validation.

pub mod error;
pub mod fields;

pub use djangors_macros::Form;
pub use error::{FieldError, FormErrors};
pub use fields::{BooleanField, CharField, EmailField, FormField, IntegerField};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_char_field() {
        // required + present
        let field = CharField {
            max_length: None,
            required: true,
        };
        assert_eq!(field.clean(Some("hello")), Ok("hello".to_string()));

        // required + absent
        assert_eq!(
            field.clean(None),
            Err(FieldError(vec!["This field is required.".to_string()]))
        );

        // required + empty string
        assert_eq!(
            field.clean(Some("")),
            Err(FieldError(vec!["This field is required.".to_string()]))
        );

        // not-required + absent
        let optional_field = CharField {
            max_length: None,
            required: false,
        };
        assert_eq!(optional_field.clean(None), Ok("".to_string()));
        assert_eq!(optional_field.clean(Some("")), Ok("".to_string()));

        // max_length respected
        let limit_field = CharField {
            max_length: Some(5),
            required: true,
        };
        assert_eq!(limit_field.clean(Some("abcde")), Ok("abcde".to_string()));
        assert_eq!(
            limit_field.clean(Some("abcdef")),
            Err(FieldError(vec![
                "Ensure this value has at most 5 characters.".to_string()
            ]))
        );

        // no trimming happens
        let whitespace_field = CharField {
            max_length: None,
            required: false,
        };
        assert_eq!(
            whitespace_field.clean(Some("  hello  ")),
            Ok("  hello  ".to_string())
        );
    }

    #[test]
    fn test_integer_field() {
        let field = IntegerField {
            min: Some(5),
            max: Some(10),
            required: true,
        };

        // valid integer parses correctly
        assert_eq!(field.clean(Some("7")), Ok(Some(7)));

        // boundary values accepted (inclusive)
        assert_eq!(field.clean(Some("5")), Ok(Some(5)));
        assert_eq!(field.clean(Some("10")), Ok(Some(10)));

        // below min
        assert_eq!(
            field.clean(Some("4")),
            Err(FieldError(vec![
                "Ensure this value is greater than or equal to 5.".to_string()
            ]))
        );

        // above max
        assert_eq!(
            field.clean(Some("11")),
            Err(FieldError(vec![
                "Ensure this value is less than or equal to 10.".to_string()
            ]))
        );

        // non-numeric string errors clearly
        assert_eq!(
            field.clean(Some("abc")),
            Err(FieldError(vec!["Enter a whole number.".to_string()]))
        );

        // required / not-required absent branching
        assert_eq!(
            field.clean(None),
            Err(FieldError(vec!["This field is required.".to_string()]))
        );
        assert_eq!(
            field.clean(Some("")),
            Err(FieldError(vec!["This field is required.".to_string()]))
        );

        let optional_field = IntegerField {
            min: None,
            max: None,
            required: false,
        };
        assert_eq!(optional_field.clean(None), Ok(None));
        assert_eq!(optional_field.clean(Some("")), Ok(None));
    }

    #[test]
    fn test_boolean_field() {
        let required_field = BooleanField { required: true };
        let optional_field = BooleanField { required: false };

        // absent -> Ok(false) specifically (not required)
        assert_eq!(optional_field.clean(None), Ok(false));

        // absent + required:true -> Err (since false fails the required check)
        assert_eq!(
            required_field.clean(None),
            Err(FieldError(vec!["This field is required.".to_string()]))
        );

        // each of "on"/"true"/"1" -> Ok(true)
        assert_eq!(optional_field.clean(Some("on")), Ok(true));
        assert_eq!(optional_field.clean(Some("true")), Ok(true));
        assert_eq!(optional_field.clean(Some("1")), Ok(true));

        // "false"/"0"/"" -> Ok(false)
        assert_eq!(optional_field.clean(Some("false")), Ok(false));
        assert_eq!(optional_field.clean(Some("0")), Ok(false));
        assert_eq!(optional_field.clean(Some("")), Ok(false));
        assert_eq!(optional_field.clean(Some("anything_else")), Ok(false));

        // required:true + truthy -> Ok(true)
        assert_eq!(required_field.clean(Some("on")), Ok(true));
        assert_eq!(required_field.clean(Some("true")), Ok(true));
        assert_eq!(required_field.clean(Some("1")), Ok(true));

        // required:true + falsy -> Err
        assert_eq!(
            required_field.clean(Some("false")),
            Err(FieldError(vec!["This field is required.".to_string()]))
        );
        assert_eq!(
            required_field.clean(Some("0")),
            Err(FieldError(vec!["This field is required.".to_string()]))
        );
        assert_eq!(
            required_field.clean(Some("")),
            Err(FieldError(vec!["This field is required.".to_string()]))
        );
    }

    #[test]
    fn test_email_field() {
        let field = EmailField { required: true };

        // valid shapes accepted
        assert_eq!(
            field.clean(Some("test@example.com")),
            Ok("test@example.com".to_string())
        );
        assert_eq!(field.clean(Some("a@b.c")), Ok("a@b.c".to_string()));
        assert_eq!(
            field.clean(Some("first.last@sub.domain.co.uk")),
            Ok("first.last@sub.domain.co.uk".to_string())
        );

        // invalid shapes rejected
        assert_eq!(
            field.clean(Some("noatsign")),
            Err(FieldError(vec!["Enter a valid email address.".to_string()]))
        );
        assert_eq!(
            field.clean(Some("@domain.com")),
            Err(FieldError(vec!["Enter a valid email address.".to_string()]))
        );
        assert_eq!(
            field.clean(Some("user@")),
            Err(FieldError(vec!["Enter a valid email address.".to_string()]))
        );
        assert_eq!(
            field.clean(Some("user@.com")),
            Err(FieldError(vec!["Enter a valid email address.".to_string()]))
        );
        assert_eq!(
            field.clean(Some("user@domain.")),
            Err(FieldError(vec!["Enter a valid email address.".to_string()]))
        );
        assert_eq!(
            field.clean(Some("user@domain..com")),
            Err(FieldError(vec!["Enter a valid email address.".to_string()]))
        );
        assert_eq!(
            field.clean(Some("user@one@two.com")),
            Err(FieldError(vec!["Enter a valid email address.".to_string()]))
        );

        // required / not-required absent branching
        assert_eq!(
            field.clean(None),
            Err(FieldError(vec!["This field is required.".to_string()]))
        );
        assert_eq!(
            field.clean(Some("")),
            Err(FieldError(vec!["This field is required.".to_string()]))
        );

        let optional_field = EmailField { required: false };
        assert_eq!(optional_field.clean(None), Ok("".to_string()));
        assert_eq!(optional_field.clean(Some("")), Ok("".to_string()));
    }

    #[test]
    fn test_form_errors() {
        let mut errors = FormErrors::new();
        assert!(errors.is_empty());

        errors.add_field_error("name", "Name is too short.");
        assert!(!errors.is_empty());
        assert_eq!(
            errors.fields.get("name"),
            Some(&FieldError(vec!["Name is too short.".to_string()]))
        );

        errors.add_field_error("name", "Name cannot contain digits.");
        assert_eq!(
            errors.fields.get("name"),
            Some(&FieldError(vec![
                "Name is too short.".to_string(),
                "Name cannot contain digits.".to_string()
            ]))
        );

        errors.add_non_field_error("Passwords do not match.");
        assert_eq!(
            errors.non_field,
            vec!["Passwords do not match.".to_string()]
        );
    }
}
