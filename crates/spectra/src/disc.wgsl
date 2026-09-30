// The disc, ported from Rainbow Player's discScene.ts.
//
// The read side is not a texture. A CD's underside is a diffraction grating -
// a 1.6 um spiral pitch - so the rainbow is computed from the grating
// equation per fragment, which is why it sweeps correctly as the disc turns
// rather than sliding around like a decal.

struct Uniforms {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    camera: vec4<f32>,
    sky: vec4<f32>,
    floor_: vec4<f32>,
    key_dir: vec4<f32>,
    key_color: vec4<f32>,
    fill_dir: vec4<f32>,
    fill_color: vec4<f32>,
    accent: vec4<f32>,
    // Track pitch in nm, rainbow strength, opacity, whether there is artwork.
    params: vec4<f32>,
    // Outer radius, start and end of the data band, edge of the artwork.
    radii: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var art: texture_2d<f32>;
@group(0) @binding(2) var art_sampler: sampler;

const LABEL: u32 = 0u;
const READ: u32 = 1u;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) kind: u32,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) radial: vec3<f32>,
    @location(3) tangent: vec3<f32>,
    @location(4) r: f32,
    @location(5) uv: vec2<f32>,
    @location(6) @interpolate(flat) kind: u32,
};

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    let world = u.model * vec4<f32>(v.position, 1.0);
    let m = mat3x3<f32>(u.model[0].xyz, u.model[1].xyz, u.model[2].xyz);
    out.world = world.xyz;
    out.normal = normalize(m * v.normal);
    // The grating runs along the track, i.e. tangentially, so the grating
    // vector that matters points radially outward.
    out.radial = normalize(m * vec3<f32>(v.position.xy, 0.0));
    out.tangent = normalize(m * vec3<f32>(-v.position.y, v.position.x, 0.0));
    out.r = length(v.position.xy);
    let outer = u.radii.x;
    out.uv = vec2<f32>(0.5 + v.position.x / (2.0 * outer), 0.5 - v.position.y / (2.0 * outer));
    out.kind = v.kind;
    out.clip = u.view_proj * world;
    return out;
}

// An analytic environment (no HDRI to load, and it keeps the palette under
// our control): a sky-to-floor gradient plus two soft area highlights.
fn env_sample(dir: vec3<f32>) -> vec3<f32> {
    let up = clamp(dir.y * 0.5 + 0.5, 0.0, 1.0);
    let base = mix(u.floor_.rgb, u.sky.rgb, up * up);
    let d = normalize(dir);
    let key = pow(max(dot(d, u.key_dir.xyz), 0.0), 48.0);
    let fill = pow(max(dot(d, u.fill_dir.xyz), 0.0), 18.0);
    return base + u.key_color.rgb * key * 3.2 + u.fill_color.rgb * fill * 0.7;
}

// Piecewise linear approximation of the visible spectrum.
fn spectral(nm: f32) -> vec3<f32> {
    var c = vec3<f32>(1.0, 0.0, 0.0);
    if nm < 440.0 {
        c = vec3<f32>(-(nm - 440.0) / 60.0, 0.0, 1.0);
    } else if nm < 490.0 {
        c = vec3<f32>(0.0, (nm - 440.0) / 50.0, 1.0);
    } else if nm < 510.0 {
        c = vec3<f32>(0.0, 1.0, -(nm - 510.0) / 20.0);
    } else if nm < 580.0 {
        c = vec3<f32>((nm - 510.0) / 70.0, 1.0, 0.0);
    } else if nm < 645.0 {
        c = vec3<f32>(1.0, -(nm - 645.0) / 65.0, 0.0);
    }
    // Roll the intensity off at both ends of the range.
    var f = 1.0;
    if nm > 700.0 {
        f = max(0.0, 0.3 + 0.7 * (780.0 - nm) / 80.0);
    } else if nm < 420.0 {
        f = max(0.0, 0.3 + 0.7 * (nm - 380.0) / 40.0);
    }
    return c * f;
}

// Khronos PBR Neutral. ACES is a film curve that drains saturated artwork and
// greys white ink; this leaves everything below ~0.76 alone and only rolls off
// genuine highlights, so the label arrives as printed while the rainbow still
// has room above 1.0.
fn neutral(color: vec3<f32>) -> vec3<f32> {
    let start = 0.76;
    let desaturation = 0.15;
    let x = min(color.r, min(color.g, color.b));
    var offset = 0.04;
    if x < 0.08 {
        offset = x - 6.25 * x * x;
    }
    var c = color - offset;
    let peak = max(c.r, max(c.g, c.b));
    if peak < start {
        return c;
    }
    let d = 1.0 - start;
    let new_peak = 1.0 - d * d / (peak + d - start);
    c = c * (new_peak / peak);
    let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return mix(c, vec3<f32>(new_peak), g);
}

// Read side: aluminium mirror plus diffraction orders off the track spiral.
fn read_side(frag: VertexOut, n: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    let g = normalize(frag.radial);
    let r = reflect(-v, n);
    var col = env_sample(r) * vec3<f32>(0.86, 0.88, 0.92);

    // Only the data band is stamped with pits; the clear rim and the clamping
    // area inside it are smooth polycarbonate and show no rainbow.
    let band = smoothstep(u.radii.y - 0.03, u.radii.y + 0.03, frag.r)
        * (1.0 - smoothstep(u.radii.z - 0.03, u.radii.z + 0.03, frag.r));

    // Grating equation, summed over the first few orders. Two lights give the
    // disc more than one rainbow to sweep through.
    var rainbow = vec3<f32>(0.0);
    for (var k = 0; k < 2; k++) {
        let l = select(u.fill_dir.xyz, u.key_dir.xyz, k == 0);
        let weight = select(0.45, 1.0, k == 0);
        let s = dot(l, g) + dot(v, g);
        for (var m = 1; m <= 3; m++) {
            let lambda = abs(u.params.x * s / f32(m));
            if lambda > 380.0 && lambda < 780.0 {
                rainbow += spectral(lambda) / f32(m) * weight;
            }
        }
    }

    // Anisotropic streak along the track, which is what makes the rainbow read
    // as coming off grooves rather than being painted on.
    let aniso = pow(1.0 - abs(dot(v, normalize(frag.tangent))), 3.0);
    col += rainbow * band * u.params.y * (0.35 + 0.65 * aniso);

    // Fresnel rim brightening.
    let fres = pow(1.0 - max(dot(n, v), 0.0), 4.0);
    return col + vec3<f32>(0.5, 0.55, 0.7) * fres * 0.35;
}

// Label side: printed artwork under clear lacquer.
fn label_side(frag: VertexOut, n: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    let art_px = textureSample(art, art_sampler, frag.uv);
    // Unprinted discs, and the hub of printed ones, are bare polycarbonate.
    let printed = u.params.w * art_px.a;
    let base = mix(u.accent.rgb * 0.35, art_px.rgb, printed);

    let ndl = max(dot(n, u.key_dir.xyz), 0.0);
    let ndf = max(dot(n, u.fill_dir.xyz), 0.0);
    let lit = base * (0.32 + 0.72 * ndl + 0.3 * ndf);

    // Clear-coat specular, plus the sheen of the moulding's concentric rings.
    // Circular grooves smear a light's reflection along the groove, so the
    // sheen is a pair of arcs keyed to the half-vector projected into the
    // disc's plane, faded out as the light approaches the normal.
    let r = reflect(-v, n);
    let fres = pow(1.0 - max(dot(n, v), 0.0), 5.0);
    let coat = env_sample(r) * (0.05 + 0.5 * fres);

    let t = normalize(frag.tangent);
    var sheen = 0.0;
    for (var k = 0; k < 2; k++) {
        let l = select(u.fill_dir.xyz, u.key_dir.xyz, k == 0);
        let h = normalize(l + v);
        let hp = h - n * dot(h, n);
        let lean = length(hp);
        if lean < 1e-4 {
            continue;
        }
        let along = dot(t, hp / lean);
        sheen += pow(max(0.0, 1.0 - along * along), 3.0) * lean * max(dot(n, h), 0.0)
            * select(0.035, 0.10, k == 0);
    }

    var col = lit + coat + vec3<f32>(sheen);

    // Inside the printed area the disc is translucent; brighten towards the
    // read side so the hub does not read as a flat hole.
    let hub = 1.0 - smoothstep(u.radii.y - 0.05, u.radii.y + 0.02, frag.r);
    col = mix(col, col + vec3<f32>(0.18, 0.2, 0.26), hub * (1.0 - printed));

    // The metallised lip outside the printed area, drawn from geometry so it
    // is exactly concentric: aluminium under lacquer.
    let lip = smoothstep(u.radii.w - 0.012, u.radii.w + 0.012, frag.r);
    let mirror = vec3<f32>(0.42, 0.44, 0.50) * (0.45 + 0.55 * ndl + 0.25 * ndf)
        + env_sample(r) * vec3<f32>(0.55, 0.57, 0.62)
        + vec3<f32>(0.55, 0.60, 0.75) * fres * 0.45;
    col = mix(col, mirror, lip);

    // The very outer sliver is bare polycarbonate, darker and glassier.
    let clear_edge = smoothstep(u.radii.x * 0.982, u.radii.x * 0.997, frag.r);
    return mix(col, col * 0.45 + env_sample(r) * 0.18, clear_edge * 0.85);
}

// Polycarbonate seen edge-on: bright, slightly tinted, quite reflective.
fn edge(n: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    let r = reflect(-v, n);
    let fres = pow(1.0 - max(dot(n, v), 0.0), 2.0);
    return env_sample(r) * 0.55 + u.accent.rgb * 0.1 + vec3<f32>(0.25, 0.28, 0.35) * fres;
}

@fragment
fn fs_main(frag: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var n = normalize(frag.normal);
    // Edges are seen from inside and out.
    if !front && frag.kind > READ {
        n = -n;
    }
    let v = normalize(u.camera.xyz - frag.world);
    var col: vec3<f32>;
    if frag.kind == LABEL {
        col = label_side(frag, n, v);
    } else if frag.kind == READ {
        col = read_side(frag, n, v);
    } else {
        col = edge(n, v);
    }
    let alpha = u.params.z;
    // Premultiplied, so multisample resolve and compositing agree.
    return vec4<f32>(neutral(col) * alpha, alpha);
}
