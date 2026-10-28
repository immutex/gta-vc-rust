import bevy_pbr::render::forward_io::VertexOutput;

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var diffuse_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var diffuse_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<uniform> tint: vec4<f32>;

// Vice City's world is prelit: the brightness of a surface is baked into its vertex
// colours, so the texture carries the detail and the vertex colour carries the shade.
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let sampled = textureSample(diffuse_texture, diffuse_sampler, in.uv);

    return vec4<f32>(sampled.rgb * in.color.rgb * tint.rgb, sampled.a * tint.a);
}
