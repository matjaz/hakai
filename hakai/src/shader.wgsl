// One damage-layer tile: a textured quad, generated procedurally (no vertex buffer) from
// `vertex_index` — six vertices, two triangles, in the tile's own 0..1 quad space.
//
// `tile.ndc_origin`/`tile.ndc_size` carry this tile's placement already converted to clip
// space on the Rust side (see `main.rs`), because that placement is static between
// resizes — only the texture's pixel *contents* change per frame, uploaded separately via
// `write_texture`, not through this uniform.

struct Tile {
    ndc_origin: vec2<f32>,
    ndc_size: vec2<f32>,
};

@group(0) @binding(0) var<uniform> tile: Tile;
@group(0) @binding(1) var tile_texture: texture_2d<f32>;
@group(0) @binding(2) var tile_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let corner = corners[vertex_index];

    var out: VertexOutput;
    // `tile.ndc_size.y` is negative — see the comment in `main.rs` where it's computed —
    // which is what makes `corner.y == 0` (this tile's top edge, in our y-down scene
    // convention) land at the *larger* NDC y (wgpu's clip space is y-up), without any
    // flip needed here in the shader itself.
    out.position = vec4<f32>(tile.ndc_origin + corner * tile.ndc_size, 0.0, 1.0);
    out.uv = corner;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(tile_texture, tile_sampler, in.uv);
}

// A rotatable sprite (currently: termites). Unlike `Tile` above, this doesn't carry an
// origin/size/angle for the shader to place — it carries the sprite's four corners,
// *already rotated and converted to clip space*, computed on the Rust side. That's
// deliberate: clip space's x and y generally have different physical scale (a screen's
// width and height differ), so rotating there — or rotating in this quad's local 0..1
// space and only then scaling anisotropically into clip space — would shear the sprite
// unless the shader corrected for the screen's aspect ratio itself. Rotating in pixel
// space instead (where x and y really are the same unit) sidesteps needing the shader to
// know or correct for that at all — see `rotated_sprite_ndc` in `main.rs`.

struct RotatedSprite {
    top_left: vec2<f32>,
    top_right: vec2<f32>,
    bottom_left: vec2<f32>,
    bottom_right: vec2<f32>,
    // A per-instance opacity multiplier — everything but a standing flame just passes 1.0
    // here (termites, shells, droplets). A flame needs it for its own dying-down fade
    // (`alpha = t > 0.8 ? max(0, (1 - t) / 0.2) : 1` in the Swift original) — a per-frame
    // number that has nothing to do with this quad's placement, so it doesn't belong in
    // the corners above.
    alpha: f32,
};

@group(0) @binding(0) var<uniform> sprite: RotatedSprite;
@group(0) @binding(1) var sprite_texture: texture_2d<f32>;
@group(0) @binding(2) var sprite_sampler: sampler;

@vertex
fn vs_sprite(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var corners = array<vec2<f32>, 4>(sprite.top_left, sprite.top_right, sprite.bottom_left, sprite.bottom_right);
    var uvs = array<vec2<f32>, 4>(vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 1.0));
    // Two triangles over the four corners [TL, TR, BL, BR]: (TL,TR,BL) and (TR,BR,BL).
    var indices = array<u32, 6>(0u, 1u, 2u, 1u, 3u, 2u);
    let idx = indices[vertex_index];

    var out: VertexOutput;
    out.position = vec4<f32>(corners[idx], 0.0, 1.0);
    out.uv = uvs[idx];
    return out;
}

@fragment
fn fs_sprite(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(sprite_texture, sprite_sampler, in.uv);
    return vec4<f32>(color.rgb, color.a * sprite.alpha);
}
