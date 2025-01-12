<div align="center">
 <img src="https://raw.githubusercontent.com/LukeMathWalker/pavex/main/logo.webp" width="300" alt="pavex" style="border-radius: 5%">
 <br>
 <strong>
   Re-imagining backend development in Rust
 </strong>
</div>

<br>

## What is `pavex`?

`pavex` is a new framework to build APIs with Rust.  
It is remarkably different: you can think of `pavex` as a **specialised compiler** for building Rust APIs.  

It takes as input a high-level description of what your application should do, a `Blueprint`:

```rust
pub fn app_blueprint() -> Blueprint {
    let mut bp = Blueprint::new();
    bp.constructor(f!(crate::http_client), Lifecycle::Singleton);
    bp.constructor(f!(crate::extract_path), Lifecycle::RequestScoped);
    bp.constructor(f!(crate::logger), Lifecycle::Transient);
    bp.route(GET, "/home", f!(crate::stream_file));
    bp
}
```

It generates as output the source code for a fully-fleshed web-server, behaving according to your specification, ready
to be launched.

`pavex` aims to have it all: great ergonomics and high performance.   
As **easy** to use as Rails, Django or ASP.NET Core.  
As **fast** as a handwritten solution that strips away all abstractions.

### Great error messages

`pavex` operates at the **right level of abstraction**—it understands the specific challenges and requirements of API development.  
The intermediate code generation step (`Blueprint` -> runtime code) allows `pavex` to perform in-depth static analysis. Potential issues
are caught at compile-time with an **informative error message**:

```text
ERROR:
  × `app::get_home` is trying to extract route parameters using `RouteParams<HomeRouteParams>`.
  │ Every struct field in `app::HomeRouteParams` must be named after one of the route parameters 
  | that appear in `/address/:address_id/home/:home_id`:
  │ - `home_id`
  │ - `address_id`
  │
  │ There is no route parameter named `street_id`, but there is a struct field named
  │ `street_id` in `app::HomeRouteParams`. This is going to cause a runtime error!
  │
  │     ╭─[src/lib.rs:43:1]
  │  43 │     ));
  │  44 │     bp.route(GET, "/home/:id", f!(crate::get_home));
  │     ·                                ───────────┬──────
  │     ·             The request handler asking for `RouteParams<app::HomeRouteParams>`
  │  45 │     
  │     ╰────
  │   help: Remove or rename the fields that do not map to a valid route parameter.
```

### Compile-time dependency injection

Your `Blueprint` tells `pavex`:

- What routes should be exposed;
- What components can be built, what inputs they need and what is their lifecycle;
- How to handle errors.

`pavex` uses this information to perform **compile-time dependency injection**.  

Each endpoint gets its own **call graph** and `pavex` makes sure to **exclusively** build what is required to invoke every single endpoint, 
avoiding unnecessary computations.

```mermaid
flowchart TB
    handler["app::stream_file(Pathbuf, Logger, &Client)"]
    client[Client]
    logger["logger() -> Logger"]
    state[ServerState]
    path["extract_path(&Request)->PathBuf"]
    request[Request]

    state --> client
    client --&--> handler
    logger --> handler
    path --> handler
    request --&--> path
```

You write straight-forward Rust, `pavex` takes care of assembling together all the components. 

## Project status

`pavex` is under active development and far from being ready for hobby or production usage.  
It has not yet been released on crates.io and you can expect breaking changes on every commit to the `main` branch.

We are aiming for an alpha release by the end of summer 2023.

We publish project updates every 4-6 weeks:

- [Vision](https://www.lpalmieri.com/posts/a-taste-of-pavex-rust-web-framework/) [December 2023]
- [Progress report #1](https://www.lpalmieri.com/posts/pavex-progress-report-01/) [January 2023]
- [Progress report #2](https://www.lpalmieri.com/posts/pavex-progress-report-02/) [March 2023]
- [Progress report #3](https://www.lpalmieri.com/posts/pavex-progress-report-03/) [April 2023]

## Examples

You can see `pavex` at work in the [`/examples` folder](./examples):

- In [`examples/skeleton/app_blueprint/src/lib.rs`](./examples/skeleton/app_blueprint/src/lib.rs) we specify the app's behavior in
  a `Blueprint`—
  the endpoints it exposes and their request handlers, as well as the required constructors for the application state;
- In [`examples/skeleton/app_blueprint/src/bin.rs`](./examples/skeleton/app_blueprint/src/bin.rs) we serialize the `Blueprint` and
  invoke `pavex`'s CLI to generate the server code that will execute at runtime, which you can find in
  [`examples/skeleton/generated_app/src/lib.rs`](./examples/skeleeton/generated_app/src/lib.rs).

In [`examples/app_blueprint/blueprint.ron`](./examples/skeleton/app_blueprint/blueprint.ron) you can have a peek at what
the `Blueprint` looks like when serialized.

## Architectural Overview

If the section above was enough to get you intrigued, you can check out the architectural deep-dive
in [`ARCHITECTURE.md`](ARCHITECTURE.md) to learn how `pavex` works under the hood.

## Contributing

This project is not open to unsolicited code contributions (for the time being).  
If you want to play around with it, you can find instructions in [`CONTRIBUTING.md`](CONTRIBUTING.md).

## License

Licensed under the Apache License, Version 2.0.
Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this crate by you, as
defined in the Apache-2.0 license, shall be licensed as above, without any additional terms or conditions.
