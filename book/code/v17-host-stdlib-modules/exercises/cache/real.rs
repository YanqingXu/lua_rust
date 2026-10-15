//! Probes of unchanged v0.17 VM, separately from the success-only cache model.
use lua_compiler::compile;
use lua_core::Value;
use lua_vm::Vm;

pub struct MatrixProbe {
    pub name: &'static str,
    pub actual: Vec<Value>,
    pub expected: Vec<Value>,
}

// ANCHOR: require_cache_real_matrix
fn loader_case(body: &str) -> Vec<Value> {
    let source = format!(
        "local calls=0\n\
         package.preload.demo=function(name) calls=calls+1; {body} end\n\
         local first=require('demo')\n\
         local second=require('demo')\n\
         return first,second,package.loaded.demo,calls"
    );
    let proto = compile(source.as_bytes()).unwrap();
    Vm::new().execute_multi(&proto).unwrap()
}
// ANCHOR_END: require_cache_real_matrix

pub fn primitive_matrix() -> Vec<MatrixProbe> {
    use Value::{Boolean as B, Nil as N, Number as Num, String as S};
    let empty = S(lua_core::ByteString::new(&b""[..]));
    let cases = [
        ("false", "return false", B(false), B(false), 2),
        ("true", "return true", B(true), B(true), 1),
        ("nil", "return nil", B(true), B(true), 1),
        ("empty", "return", B(true), B(true), 1),
        ("zero", "return 0", Num(0.0), Num(0.0), 1),
        ("empty-string", "return ''", empty.clone(), empty, 1),
        (
            "cleared",
            "package.loaded.demo=nil; return nil",
            B(true),
            N,
            2,
        ),
        (
            "published-false",
            "package.loaded.demo=false; return nil",
            B(false),
            B(false),
            2,
        ),
        (
            "published-number",
            "package.loaded.demo=17; return nil",
            Num(17.0),
            Num(17.0),
            1,
        ),
        (
            "published-number-empty",
            "package.loaded.demo=17; return",
            Num(17.0),
            Num(17.0),
            1,
        ),
        ("first-nil", "return nil,99", B(true), B(true), 1),
        (
            "published-zero-first-nil",
            "package.loaded.demo=0; return nil,99",
            Num(0.0),
            Num(0.0),
            1,
        ),
        (
            "first-false-overrides",
            "package.loaded.demo=17; return false,99",
            B(false),
            B(false),
            2,
        ),
        (
            "cleared-first-nil",
            "package.loaded.demo=nil; return nil,99",
            B(true),
            N,
            2,
        ),
        (
            "cleared-empty",
            "package.loaded.demo=nil; return",
            B(true),
            N,
            2,
        ),
    ];
    cases
        .into_iter()
        .map(|(name, body, returned, slot, calls)| MatrixProbe {
            name,
            actual: loader_case(body),
            expected: vec![returned.clone(), returned, slot, Num(f64::from(calls))],
        })
        .collect()
}

pub fn table_identity_and_self_publication() -> Vec<Value> {
    let proto = compile(
        br#"
      local calls=0
      package.preload.normal=function(name)
        calls=calls+1
        return {answer=42}
      end
      local a=require('normal')
      local b=require('normal')
      local normal=a==b and a==package.loaded.normal and b.answer==42 and calls==1
      package.preload.published=function(name)
        calls=calls+1
        package.loaded[name]={answer=43}
        return nil,99
      end
      local c=require('published')
      local d=require('published')
      return normal,c==d and c==package.loaded.published and d.answer==43 and calls==2
    "#,
    )
    .unwrap();
    Vm::new().execute_multi(&proto).unwrap()
}

// ANCHOR: require_cache_real_owner
pub fn cached_table_across_execute_and_gc() -> Vec<Value> {
    let mut vm = Vm::new();
    let setup = compile(
        br#"
      cache_calls=0
      package.preload.demo=function(name)
        cache_calls=cache_calls+1
        return {answer=42}
      end
      local original=package.loaded
      local value=require('demo')
      package.loaded={}
      return value==original.demo and value.answer==42 and package.loaded.demo==nil
    "#,
    )
    .unwrap();
    let mut results = vm.execute_multi(&setup).unwrap();
    // Host keeps only a boolean. The finished frame owns no live local alias.
    vm.collect_garbage().unwrap();
    let again = compile(
        br#"
      local value=require('demo')
      return value.answer==42,cache_calls==1,package.loaded.demo==nil
    "#,
    )
    .unwrap();
    results.extend(vm.execute_multi(&again).unwrap());
    results
}
// ANCHOR_END: require_cache_real_owner
