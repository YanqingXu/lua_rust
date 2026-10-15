use crate::model::{self, Decide, Decision};
use lua_compiler::{compile, CompileErrorKind};
use lua_core::Value;

// ANCHOR: repl_classification_contract
pub fn structured_errors_choose_the_next_repl_action(decide: Decide) {
    assert_eq!(decide(&compile(b"return 42\n")), Decision::Execute);
    for (name, prefix, suffix) in model::unfinished() {
        let compilation = compile(prefix);
        let error = compilation.as_ref().unwrap_err();
        assert_eq!(error.kind, CompileErrorKind::NeedMoreInput, "{name}");
        assert_eq!(decide(&compilation), Decision::Preserve, "{name}");
        let full = [prefix, suffix].concat();
        assert!(
            compile(&full).is_ok(),
            "{name}: suffix must complete source"
        );
    }
    for (name, source) in model::syntax_errors() {
        let compilation = compile(source);
        assert_eq!(
            compilation.as_ref().unwrap_err().kind,
            CompileErrorKind::Invalid,
            "{name}"
        );
        assert_eq!(decide(&compilation), Decision::Reject, "{name}");
    }
    let mut pending = compile(b"return 40 +\n").unwrap_err();
    pending.message = "please finish this expression".into();
    assert_eq!(decide(&Err(pending)), Decision::Preserve);
    let mut invalid = compile(b"return @\n").unwrap_err();
    invalid.message = "unterminated diagnostic wording is not a kind".into();
    assert_eq!(decide(&Err(invalid)), Decision::Reject);
    assert_eq!(
        model::complete(b"return 40 +\n2\n").unwrap(),
        [Value::Number(42.0)]
    );
}
// ANCHOR_END: repl_classification_contract
