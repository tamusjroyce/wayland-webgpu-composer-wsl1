// Presents the shared framebuffer texture on a fullscreen triangle, scaled to fit the
// window while preserving aspect ratio (letterboxed).

struct Uniforms {
    win: vec2<f32>,
    tex: vec2<f32>,
};

@group(0) @binding(0) var frame_tex: texture_2d<f32>;
@group(0) @binding(1) var frame_sampler: sampler;
@group(0) @binding(2) var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let xy = corners[vid];
    var out: VsOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    // Clip space [-1,1] -> UV [0,1], with Y flipped so the framebuffer is upright.
    out.uv = vec2<f32>((xy.x + 1.0) * 0.5, (1.0 - xy.y) * 0.5);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let win_aspect = u.win.x / u.win.y;
    let tex_aspect = u.tex.x / u.tex.y;
    var scale = vec2<f32>(1.0, 1.0);
    if (win_aspect > tex_aspect) {
        scale.x = win_aspect / tex_aspect;
    } else {
        scale.y = tex_aspect / win_aspect;
    }
    let uv = (in.uv - vec2<f32>(0.5, 0.5)) * scale + vec2<f32>(0.5, 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        return vec4<f32>(0.02, 0.02, 0.02, 1.0);
    }
    return textureSample(frame_tex, frame_sampler, uv);
}
