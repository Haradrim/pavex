use pavex_builder::{constructor::Lifecycle, f, router::GET, Blueprint};

pub fn handler(_logger: dep_1::Logger, _logger_2: dep_2::Logger) -> pavex_runtime::response::Response {
    todo!()
}

pub fn blueprint() -> Blueprint {
    let mut bp = Blueprint::new();
    dep_1::add_logger(&mut bp);
    dep_2::add_logger(&mut bp);
    bp.route(GET, "/home", f!(crate::handler));
    bp
}
