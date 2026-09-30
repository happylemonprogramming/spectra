// Lays the disc, rendered off-screen with multisampling, over the UI.

@group(0) @binding(0) var disc: texture_2d<f32>;
@group(0) @binding(1) var disc_sampler: sampler;
// x: 1 when the target does not encode sRGB itself.
@group(0) @binding(2) var<uniform> options: vec4<f32>;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// One triangle that covers the viewport.
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VertexOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VertexOut;
    out.clip = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

fn encode(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs_main(frag: VertexOut) -> @location(0) vec4<f32> {
    let c = textureSample(disc, disc_sampler, frag.uv);
    if options.x > 0.5 && c.a > 0.0 {
        return vec4<f32>(encode(c.rgb / c.a) * c.a, c.a);
    }
    return c;
}
