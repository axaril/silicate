//! The shader is only compiled when the app creates its pipeline, so check
//! that it parses and validates here.

use wgpu::naga;

#[test]
fn shader_is_valid_wgsl() {
    let source = include_str!("../src/shader.wgsl");
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|err| panic!("{}", err.emit_to_string(source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .unwrap_or_else(|err| panic!("{}", err.emit_to_string(source)));
}
