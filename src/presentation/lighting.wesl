import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, main_pass_post_lighting_processing},
};
struct SceneLight {
    position_range: vec4<f32>,
    colour: vec4<f32>,
    factors: vec4<f32>,
}
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> compiled: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var<storage, read> lights: array<SceneLight>;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(in, is_front);
    // Sample the same material without baked vertex tint to recover albedo,
    // including black vertices that a dynamic light can illuminate.
    var untinted = in;
    @if(VERTEX_COLORS) untinted.color = vec4<f32>(1.0);
    let albedo = pbr_input_from_standard_material(untinted, is_front).material.base_color;
    var delta = vec3<f32>(0.0);
    let normal = normalize(in.world_normal);
    for (var i = 0u; i < arrayLength(&lights); i += 1u) {
        let light = lights[i];
        let toward = light.position_range.xyz - in.world_position.xyz;
        let distance = length(toward);
        let range = light.position_range.w;
        if distance == 0.0 || distance >= range { continue; }
        let factor = select(light.factors.x, light.factors.y, compiled.x > 0.0);
        delta += light.colour.rgb * factor * (1.0 - distance / range) * max(dot(normal, toward / distance), 0.0);
    }
    pbr.material.base_color = vec4<f32>(max(pbr.material.base_color.rgb + albedo.rgb * delta, vec3<f32>(0.0)), pbr.material.base_color.a);
    pbr.material.base_color = alpha_discard(pbr.material.flags, pbr.material.alpha_cutoff, pbr.material.base_color);
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(pbr, pbr.material.base_color);
    return out;
}
