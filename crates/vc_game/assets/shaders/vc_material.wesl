import bevy_pbr::render::forward_io::VertexOutput;

// Group constants::MATERIAL_BIND_GROUP is the material bind group; view and mesh take
// the others. It is a shader definition rather than a fixed number, so it is imported
// through `constants` and cannot drift if Bevy renumbers again.
struct VcUniforms {
    tint: vec4<f32>,
    cutout: f32,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var diffuse_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var diffuse_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<uniform> uniforms: VcUniforms;

// Vice City's world is prelit: the brightness of a surface is baked into its vertex
// colours, so the texture carries the detail and the vertex colour carries the shade.
// Both were authored in gamma space and are multiplied there, exactly as the original
// fixed-function pipeline did.
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let sampled = textureSample(diffuse_texture, diffuse_sampler, in.uv);
    let colour = vec4<f32>(
        sampled.rgb * in.color.rgb * uniforms.tint.rgb,
        sampled.a * uniforms.tint.a,
    );

    // Bevy only discards alpha inside its own fragment path, so a custom material has
    // to do it itself; otherwise every cut-out texture renders as a solid card.
    if uniforms.cutout > 0.5 && colour.a < 0.5 {
        discard;
    }

    return colour;
}
