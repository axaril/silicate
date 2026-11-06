// Vertex shader ///////////////////////////////////////////////////////////////
////////////////////////////////////////////////////////////////////////////////
// Draws one unit quad per canvas tile. The quad corners are generated from the
// vertex index (4-vertex triangle strip) and the tile from the instance index,
// so no vertex, index or instance buffers are needed.

struct CanvasTiling {
    height: u32,
    width: u32,
    cols: u32,
    rows: u32,
    tile_size: u32,
    flipped: u32,
};

@group(0) @binding(0)
var<uniform> canvas: CanvasTiling;

struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(0) coords: vec2f,
    @location(1) @interpolate(flat) silo_index: u32,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    // Strip order: (0,1), (1,1), (0,0), (1,0)
    let corner = vec2f(f32(vertex_index & 1u), f32(1u - (vertex_index >> 1u)));

    // Instances are laid out row-major, matching the silo layout.
    let col = instance_index % canvas.cols;
    let row = instance_index / canvas.cols;

    let tile_coords = vec2f(f32(col), f32(row));
    let canvas_grid = vec2f(f32(canvas.cols), f32(canvas.rows));
    let canvas_dim = vec2f(f32(canvas.width), f32(canvas.height));

    let flipped_horizontally = ((canvas.flipped >> 1) & 1) != 0;
    let flipped_vertically = (canvas.flipped & 1) != 0;

    let scale = canvas_grid * f32(canvas.tile_size) / canvas_dim;
    let pos = (corner + tile_coords) / canvas_grid;
    let normalized_pos = select(pos * scale, 1.0 - pos * scale, vec2(flipped_horizontally, flipped_vertically)) * 2.0 - 1.0;

    var out: VertexOutput;
    out.position = vec4(normalized_pos, 0.0, 1.0);
    out.coords = corner;
    out.silo_index = instance_index;
    return out;
}

// Blening cod ///////////////////////////////////////////////////////////////
////////////////////////////////////////////////////////////////////////////////

// HSL Blending Modes //////////////////////////////////////////////////////////
// [PDF Blend Modes: Addendum]
// [KHR_blend_equation_advanced]
fn rgb2hsv(c: vec3f) -> vec3f {
    let K = vec4f(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    let p = mix(vec4f(c.bg, K.wz), vec4f(c.gb, K.xy), step(c.b, c.g));
    let q = mix(vec4f(p.xyw, c.r), vec4f(c.r, p.yzx), step(p.x, c.r));

    let d = q.x - min(q.w, q.y);
    let e = 1.0e-10;
    return vec3f(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

fn hsv2rgb(c: vec3f) -> vec3f {
    let K = vec4f(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    let p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
    return c.z * mix(K.xxx, saturate(p - K.xxx), c.y);
}

fn lum(c: vec3f) -> f32 {
    return dot(c, vec3(0.3, 0.59, 0.11));
}

fn clip_color(c: vec3f) -> vec3f {
    let l = lum(c);
    let n = min(min(c.r, c.g), c.b);
    let x = max(max(c.r, c.g), c.b);
    var z = c;
    if (n < 0.0) {
        z = l + (((c - l) * l) / (l - n));
    }
    if (x > 1.0) {
        z = l + (((z - l) * (1.0 - l)) / (x - l));
    }
    return saturate(z);
}

fn set_lum(c: vec3f, l: f32) -> vec3f {
    let d = l - lum(c);
    return clip_color(c + d);
}

fn sat(c: vec3f) -> f32 {
    let n = min(min(c.r, c.g), c.b);
    let x = max(max(c.r, c.g), c.b);
    return x - n;
}

fn set_sat(cb: vec3f, s: f32) -> vec3f {
    let mb = min(min(cb.r, cb.g), cb.b);
    let sb = sat(cb);
    // Equivalent (modulo rounding errors) to setting the
    // smallest (R,G,B) component to 0, the largest to <ssat>,
    // and interpolating the "middle" component based on its
    // original value relative to the smallest/largest.
    return select(vec3(0.0), (cb - mb) * s / sb, sb > 0.0);
}

fn color(b: vec3f, s: vec3f) -> vec3f {
    return set_lum(s.rgb, lum(b.rgb));
}

fn luminosity(b: vec3f, s: vec3f) -> vec3f {
    return set_lum(b.rgb, lum(s.rgb));
}

fn hue(b: vec3f, s: vec3f) -> vec3f {
    return set_lum(set_sat(s.rgb, sat(b.rgb)), lum(b.rgb));
}

fn saturation(b: vec3f, s: vec3f) -> vec3f {
    return set_lum(set_sat(b.rgb, sat(s.rgb)), lum(b.rgb));
}

// RGB Blending Modes //////////////////////////////////////////////////////////
// [PDF Blend Modes: Addendum]
// [https://photoblogstop.com/photoshop/photoshop-blend-modes-explained]

fn normal(b: vec3f, s: vec3f) -> vec3f {
    return s;
}

fn multiply(b: vec3f, s: vec3f) -> vec3f {
    return s * b;
}

fn divide(b: vec3f, s: vec3f) -> vec3f {
    return b / s;
}

fn screen(b: vec3f, s: vec3f) -> vec3f {
    return s + b - s * b;
}

fn add(b: vec3f, s: vec3f) -> vec3f {
    return s + b;
}

fn hard_light(b: vec3f, s: vec3f) -> vec3f {
    return mix(
        screen(b, 2.0 * s - 1.0),
        multiply(b, s * 2.0),
        step(s, vec3(0.5))
    );
}

fn overlay(b: vec3f, s: vec3f) -> vec3f {
    return hard_light(s, b);
}

fn darken(b: vec3f, s: vec3f) -> vec3f {
    return min(s, b);
}

fn lighten(b: vec3f, s: vec3f) -> vec3f {
    return max(s, b);
}

fn difference(b: vec3f, s: vec3f) -> vec3f {
    return abs(b - s);
}

fn subtract(b: vec3f, s: vec3f) -> vec3f {
    return b - s;
}

fn linear_burn(b: vec3f, s: vec3f) -> vec3f {
    return max(b + s - 1.0, vec3(0.0));
}

fn linear_dodge(b: vec3f, s: vec3f) -> vec3f {
    return min(b + s, vec3(1.0));
}

fn linear_light(b: vec3f, s: vec3f) -> vec3f {
    return mix(
        linear_dodge(b, 2.0 * (s - 0.5)),
        linear_burn(b, 2.0 * s),
        step(s, vec3(0.5))
    );
}

fn exclusion(b: vec3f, s: vec3f) -> vec3f {
    return b + s - 2.0 * b * s;
}

fn color_dodge(b: vec3f, s: vec3f) -> vec3f {
    return mix(
        vec3(1.0),
        min(vec3(1.0), b / (1.0 - s)),
        step(s, vec3(1.0))
    );
}

fn color_burn(b: vec3f, s: vec3f) -> vec3f {
    return mix(
        1.0 - min(vec3(1.0), (1.0 - b) / s),
        vec3(0.0),
        step(s, vec3(0.0))
    );
}

fn soft_light(b: vec3f, s: vec3f) -> vec3f {
    return mix(
        sqrt(b) * (2.0 * s - 1.0) + 2.0 * b * (1.0 - s),
        2.0 * b * s + b * b * (1.0 - 2.0 * s),
        step(s, vec3(0.5))
    );
}

fn vivid_light(b: vec3f, s: vec3f) -> vec3f {
    return mix(
        color_dodge(b, 2.0 * (s - 0.5)),
        color_burn(b, 2.0 * s),
        step(s, vec3(0.5))
    );
}

fn hard_mix(b: vec3f, s: vec3f) -> vec3f {
    return mix(
        vec3(1.0),
        vec3(0.0),
        step(vivid_light(b, s), vec3(0.5))
    );
}

fn pin_light(b: vec3f, s: vec3f) -> vec3f {
    return mix(
        lighten(b, 2.0 * (s - 0.5)),
        darken(b, 2.0 * s),
        step(s, vec3(0.5))
    );
}

fn lighter_color(b: vec3f, s: vec3f) -> vec3f {
    return select(b, s, lum(b) < lum(s));
}

fn darker_color(b: vec3f, s: vec3f) -> vec3f {
    return select(b, s, lum(b) > lum(s));
}

// Fragment shader /////////////////////////////////////////////////////////////
////////////////////////////////////////////////////////////////////////////////

struct LayerData {
    opacity: f32,
    blend: u32,
    flags: u32,
};

const LayerFlags_CLIPPED: u32 = 1;

fn layer_clipped(flags: u32) -> bool {
    return (flags & LayerFlags_CLIPPED) != 0;
}

// Location of a tile in the atlas, resolved on the CPU.
struct AtlasRef {
    // Texel origin packed as x | y << 16.
    origin: u32,
    layer: u32,
};

// Hidden layers, hidden masks and clipped chunks without a base are culled on
// the CPU, so every chunk here contributes and HAS_MASK means "apply the mask".
const ChunkFlags_HAS_MASK: u32 = 1;

struct ChunkData {
    color: AtlasRef,
    mask: AtlasRef,
    clip: AtlasRef,
    layer_index: u32,
    flags: u32,
};

struct SiloData {
    start: u32,
    end: u32,
}

@group(1) @binding(0)
var atlas_texture: texture_2d_array<f32>;
@group(1) @binding(1)
var<storage, read> chunks: array<ChunkData>;
@group(1) @binding(2)
var<storage, read> layers: array<LayerData>;
@group(1) @binding(3)
var<storage, read> silos: array<SiloData>;
@group(1) @binding(4)
var<uniform> bg_in: vec4f;

fn load_atlas(tile: AtlasRef, px: vec2u) -> vec4f {
    let origin = vec2u(tile.origin & 0xFFFFu, tile.origin >> 16u);
    return textureLoad(atlas_texture, origin + px, tile.layer, 0);
}

// Premultiplied -> straight color.
fn unpremultiply(c: vec4f) -> vec3f {
    return select(saturate(c.rgb / c.a), vec3f(0.0), c.a == 0.0);
}

// Blend straight colors according to the layer's mode.
fn blend_rgb(mode: u32, b: vec3f, s: vec3f) -> vec3f {
    switch (mode) {
        case 1u: { return multiply(b, s); }
        case 2u: { return screen(b, s); }
        case 3u: { return add(b, s); }
        case 4u: { return lighten(b, s); }
        case 5u: { return exclusion(b, s); }
        case 6u: { return difference(b, s); }
        case 7u: { return subtract(b, s); }
        case 8u: { return linear_burn(b, s); }
        case 9u: { return color_dodge(b, s); }
        case 10u: { return color_burn(b, s); }
        case 11u: { return overlay(b, s); }
        case 12u: { return hard_light(b, s); }
        case 13u: { return color(b, s); }
        case 14u: { return luminosity(b, s); }
        case 15u: { return hue(b, s); }
        case 16u: { return saturation(b, s); }
        case 17u: { return soft_light(b, s); }
        case 19u: { return darken(b, s); }
        case 20u: { return hard_mix(b, s); }
        case 21u: { return vivid_light(b, s); }
        case 22u: { return linear_light(b, s); }
        case 23u: { return pin_light(b, s); }
        case 24u: { return lighter_color(b, s); }
        case 25u: { return darker_color(b, s); }
        case 26u: { return divide(b, s); }
        default: { return normal(b, s); }
    }
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4f {
    // Accumulated premultiplied color.
    var dst = bg_in;

    let silo = silos[in.silo_index];
    let px = min(vec2u(in.coords * f32(canvas.tile_size)), vec2u(canvas.tile_size - 1u));

    for (var i: u32 = silo.start; i < silo.end; i++) {
        // `chunk` and `layer` are the same for every pixel in the tile, so
        // branches on them don't diverge.
        let chunk = chunks[i];
        let layer = layers[chunk.layer_index];

        // Premultiplied layer color.
        var fg = load_atlas(chunk.color, px);
        if (fg.a == 0.0) {
            continue;
        }
        if (layer_clipped(layer.flags)) {
            fg *= load_atlas(chunk.clip, px).a;
        }
        if ((chunk.flags & ChunkFlags_HAS_MASK) != 0u) {
            fg *= load_atlas(chunk.mask, px).a;
        }

        // Premultiplied source with layer opacity applied.
        let src = fg * layer.opacity;

        if (layer.blend == 0u) {
            // Normal: plain premultiplied "over".
            dst = src + dst * (1.0 - src.a);
            continue;
        }

        // Separable blend (PDF spec) in premultiplied form:
        //   Cs'(1 - ab) + Cb'(1 - as) + as * ab * B(cb, cs)
        let mixed = saturate(blend_rgb(layer.blend, unpremultiply(dst), unpremultiply(fg)));
        dst = saturate(vec4(
            src.rgb * (1.0 - dst.a) + dst.rgb * (1.0 - src.a) + src.a * dst.a * mixed,
            src.a + dst.a - src.a * dst.a
        ));
    }
    return dst;
}
