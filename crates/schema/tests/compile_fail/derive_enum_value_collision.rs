use nebula_schema::EnumSelect;

// `HttpGet` and `HTTPGet` both snake_case to `http_get` — the derive must reject
// the collision with a spanned compile error rather than silently emitting two
// options with the same value.
#[derive(EnumSelect)]
enum Collide {
    HttpGet,
    HTTPGet,
}

fn main() {}
