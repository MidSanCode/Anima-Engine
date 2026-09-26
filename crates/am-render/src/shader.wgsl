// Anima 引擎离屏渲染着色器。
//
// 设计要点：
//   * 顶点已经是画布空间坐标，唯一的变换是「画布 → NDC」的视图矩阵（uniform）。
//   * 片元输出**预乘 alpha**，四种混合模式（normal / multiply / screen / additive）
//     全部由固定管线的混合状态实现，不需要分支。
//   * 遮罩用「层 + dst_in / dst_out」实现：先把绘制对象画进临时层，再用遮罩几何体
//     以 dst_in（或 dst_out）混合裁剪该层，最后把层合成回主目标。

struct Globals {
    view: mat4x4<f32>,
    resolution: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var src_tex: texture_2d<f32>;
@group(1) @binding(1) var src_samp: sampler;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    // 绘制对象的不透明度（已乘上层级不透明度），逐顶点携带以避免逐对象 uniform
    @location(2) opacity: f32,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) opacity: f32,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip = globals.view * vec4<f32>(in.pos, 0.0, 1.0);
    out.uv = in.uv;
    out.opacity = in.opacity;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(src_tex, src_samp, in.uv);
    // 预乘 alpha：混合状态直接工作在预乘色彩空间
    let a = texel.a * in.opacity;
    return vec4<f32>(texel.rgb * a, a);
}

// 全屏三角形（遮罩层合成）：不依赖顶点缓冲
@vertex
fn vs_fullscreen(@builtin(vertex_index) idx: u32) -> VsOut {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );
    var out: VsOut;
    let p = corners[idx];
    out.clip = vec4<f32>(p, 0.0, 1.0);
    // NDC → UV：纹理第 0 行对应 NDC y = +1
    out.uv = vec2<f32>((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
    out.opacity = 1.0;
    return out;
}

@fragment
fn fs_composite(in: VsOut) -> @location(0) vec4<f32> {
    // 层里存的已经是预乘结果，直接输出
    return textureSample(src_tex, src_samp, in.uv);
}
