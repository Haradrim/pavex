use std::path::PathBuf;

use pavex_builder::{
    constructor::{CloningStrategy, Lifecycle},
    f,
    router::GET,
    Blueprint,
};
use pavex_runtime::response::Response;
use pavex_runtime::extract::route::RawRouteParams;

// The call graph looks like this:
//
// RawRouteParams
//  / \
// B   |&
//  \  |
// handler
//
// `RawRouteParams` is a framework built-in type.
// `RawRouteParams` cannot be borrowed by `handler` after it has been moved to construct `B`.
// `RawRouteParams` is cloneable though!
// Pavex should detect this and clone `RawRouteParams` before calling `B`'s constructor.

pub struct B;

pub fn b(_p: RawRouteParams) -> B {
    todo!()
}

pub fn handler(_p: &RawRouteParams, _b: B) -> Response {
    todo!()
}

pub fn blueprint() -> Blueprint {
    let mut bp = Blueprint::new();
    bp.constructor(f!(crate::b), Lifecycle::RequestScoped);
    bp.route(GET, "/home", f!(crate::handler));
    bp
}
