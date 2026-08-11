use super::*;

#[test]
fn shell_quote_neutralizes_every_dangerous_character() {
    assert_eq!(shell_quote("hello"), "'hello'");
    assert_eq!(shell_quote("it's"), r#"'it'\''s'"#);
    assert_eq!(shell_quote("$(rm -rf /)"), "'$(rm -rf /)'");
    assert_eq!(shell_quote("a; rm -rf /"), "'a; rm -rf /'");
    assert_eq!(shell_quote("`whoami`"), "'`whoami`'");
}
