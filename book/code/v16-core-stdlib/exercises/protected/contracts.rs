use crate::model::{body_error, error_text, handler_error, text, Packer, XPacker};
use lua_core::Value::{Boolean, Nil, Number};

pub fn complete_pcall_results(pack: Packer) {
    for values in [
        vec![],
        vec![Nil],
        vec![Boolean(false)],
        vec![Number(7.0), Nil, Number(9.0), Nil],
    ] {
        let mut expected = vec![Boolean(true)];
        expected.extend(values.clone());
        assert_eq!(pack(Ok(values)), expected, "成功数目与 nil 位置必须保留");
    }
    assert_eq!(
        pack(Err(body_error())),
        [Boolean(false), error_text(body_error())]
    );
}

pub fn conditional_xpcall_handler(pack: XPacker) {
    for values in [
        vec![],
        vec![Nil],
        vec![Boolean(false)],
        vec![Number(7.0), Nil, Number(9.0), Nil],
    ] {
        let mut calls = 0;
        let actual = pack(Ok(values.clone()), &mut |_| {
            calls += 1;
            Ok(vec![text(b"unexpected")])
        });
        let mut expected = vec![Boolean(true)];
        expected.extend(values);
        assert_eq!(actual, expected, "body 成功时不能转进 handler");
        assert_eq!(calls, 0);
    }
    for handled in [
        vec![Number(42.0), text(b"ignored")],
        vec![],
        vec![Nil, Number(99.0)],
        vec![Boolean(false)],
    ] {
        let mut seen = Vec::new();
        let expected = handled.first().cloned().unwrap_or(Nil);
        let actual = pack(Err(body_error()), &mut |message| {
            seen.push(message);
            Ok(handled.clone())
        });
        assert_eq!(
            actual,
            [Boolean(false), expected],
            "只选择 handler 首结果，无结果补 nil"
        );
        assert_eq!(
            seen,
            [error_text(body_error())],
            "handler 只收到一个格式化错误字符串，恰好调用一次"
        );
    }
    let mut seen = Vec::new();
    let actual = pack(Err(body_error()), &mut |message| {
        seen.push(message);
        Err(handler_error())
    });
    assert_eq!(seen, [error_text(body_error())]);
    assert_eq!(actual, [Boolean(false), error_text(handler_error())]);
}
