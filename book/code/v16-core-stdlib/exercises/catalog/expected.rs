//! 独立公开合同：不从 BASE_FUNCTIONS/LIBRARIES 派生；改 catalog 不会自动改变此期望。
use crate::model::Bindings;
use lua_core::{
    BaseFunction as B, MathFunction as M, NativeFunction as N, StringFunction as S,
    TableFunction as T,
};

const EXPECTED: &[(&str, &[(&str, N)])] = &[
    (
        "",
        &[
            ("assert", N::Base(B::Assert)),
            ("collectgarbage", N::Base(B::CollectGarbage)),
            ("error", N::Base(B::Error)),
            ("getmetatable", N::GetMetatable),
            ("ipairs", N::IPairs),
            ("next", N::Next),
            ("pairs", N::Pairs),
            ("pcall", N::Base(B::PCall)),
            ("print", N::Base(B::Print)),
            ("rawequal", N::Base(B::RawEqual)),
            ("rawget", N::RawGet),
            ("rawset", N::RawSet),
            ("select", N::Base(B::Select)),
            ("setmetatable", N::SetMetatable),
            ("tonumber", N::Base(B::ToNumber)),
            ("tostring", N::ToString),
            ("type", N::Base(B::Type)),
            ("unpack", N::Base(B::Unpack)),
            ("xpcall", N::Base(B::XPCall)),
        ],
    ),
    (
        "math",
        &[
            ("abs", N::Math(M::Abs)),
            ("acos", N::Math(M::Acos)),
            ("asin", N::Math(M::Asin)),
            ("atan", N::Math(M::Atan)),
            ("atan2", N::Math(M::Atan2)),
            ("ceil", N::Math(M::Ceil)),
            ("cos", N::Math(M::Cos)),
            ("cosh", N::Math(M::Cosh)),
            ("deg", N::Math(M::Deg)),
            ("exp", N::Math(M::Exp)),
            ("floor", N::Math(M::Floor)),
            ("fmod", N::Math(M::Fmod)),
            ("frexp", N::Math(M::Frexp)),
            ("ldexp", N::Math(M::Ldexp)),
            ("log", N::Math(M::Log)),
            ("log10", N::Math(M::Log10)),
            ("max", N::Math(M::Max)),
            ("min", N::Math(M::Min)),
            ("modf", N::Math(M::Modf)),
            ("pow", N::Math(M::Pow)),
            ("rad", N::Math(M::Rad)),
            ("random", N::Math(M::Random)),
            ("randomseed", N::Math(M::RandomSeed)),
            ("sin", N::Math(M::Sin)),
            ("sinh", N::Math(M::Sinh)),
            ("sqrt", N::Math(M::Sqrt)),
            ("tan", N::Math(M::Tan)),
            ("tanh", N::Math(M::Tanh)),
        ],
    ),
    (
        "string",
        &[
            ("byte", N::String(S::Byte)),
            ("char", N::String(S::Char)),
            ("find", N::String(S::Find)),
            ("format", N::String(S::Format)),
            ("len", N::String(S::Len)),
            ("lower", N::String(S::Lower)),
            ("rep", N::String(S::Rep)),
            ("reverse", N::String(S::Reverse)),
            ("sub", N::String(S::Sub)),
            ("upper", N::String(S::Upper)),
        ],
    ),
    (
        "table",
        &[
            ("concat", N::Table(T::Concat)),
            ("insert", N::Table(T::Insert)),
            ("maxn", N::Table(T::MaxN)),
            ("remove", N::Table(T::Remove)),
            ("sort", N::Table(T::Sort)),
        ],
    ),
    (
        "coroutine",
        &[
            ("create", N::CoroutineCreate),
            ("resume", N::CoroutineResume),
            ("yield", N::CoroutineYield),
            ("status", N::CoroutineStatus),
            ("running", N::CoroutineRunning),
            ("wrap", N::CoroutineWrap),
        ],
    ),
];

pub fn bindings() -> Bindings {
    let mut result = Bindings::new();
    for &(scope, functions) in EXPECTED {
        for &(name, function) in functions {
            assert!(result
                .insert(
                    (scope.as_bytes().to_vec(), name.as_bytes().to_vec()),
                    function
                )
                .is_none());
        }
    }
    assert_eq!(result.len(), 68);
    result
}
