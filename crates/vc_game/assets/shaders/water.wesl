import bevy_pbr::render::forward_io::VertexOutput;

struct WaterParams {
    time: f32,
    deep: vec4<f32>,
    shallow: vec4<f32>,
    sky: vec4<f32>,
    sun: vec4<f32>,
};

@group(3) @binding(0) var ripples: texture_2d<f32>;
@group(3) @binding(1) var ripple_sampler: sampler;
@group(3) @binding(2) var<uniform> params: WaterParams;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let drift_a = params.time * vec2<f32>(0.013, 0.008);
    let drift_b = params.time * vec2<f32>(-0.019, 0.015);
    let a = textureSample(ripples, ripple_sampler, in.uv * 37.0 + drift_a).rgb;
    let b = textureSample(ripples, ripple_sampler, in.uv * 61.0 + drift_b).rgb;
    let slope = (a - b) * 1.1;
    let crest = clamp(length(slope), 0.0, 1.0);
    let pattern = mix(a, b, 0.5);
    let body = mix(params.deep.rgb * 0.85, params.shallow.rgb * 0.75, pattern.r * 0.65);
    let colour = mix(body, params.shallow.rgb, crest * 0.25)
        + params.sky.rgb * (crest * 0.10 + 0.12)
        + vec3<f32>(params.sun.w * crest * 0.18);

    return vec4<f32>(colour, 1.0);
}
