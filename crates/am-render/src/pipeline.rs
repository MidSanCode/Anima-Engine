//! 渲染管线：四种混合模式 + 遮罩裁剪 + 层合成。
//!
//! 全部管线共用同一套着色器与绑定布局（group 0 = 全局 uniform，group 1 = 纹理 + 采样器），
//! 差异只在**混合状态**，因此切换模式不需要重建绑定。

use am_model::BlendType;
use wgpu::{
    BindGroupLayout, BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingType, BlendComponent,
    BlendState, BlendOperation, ColorTargetState, ColorWrites, Device, FragmentState,
    MultisampleState, PipelineCompilationOptions, PipelineLayout, PipelineLayoutDescriptor,
    PrimitiveState, RenderPipeline, RenderPipelineDescriptor, ShaderStages, TextureFormat,
    TextureSampleType, TextureViewDimension, VertexBufferLayout, VertexState, VertexStepMode,
};

const SHADER: &str = include_str!("shader.wgsl");

/// 顶点：位置（画布空间）+ 纹理坐标 + 不透明度。
pub const VERTEX_STRIDE: u64 = 20;

const VERTEX_ATTRS: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32];

/// 混合模式下标（数组索引）。
pub fn blend_index(blend: BlendType) -> usize {
    match blend {
        BlendType::Normal => 0,
        BlendType::Multiply => 1,
        BlendType::Screen => 2,
        BlendType::Additive => 3,
    }
}

/// 绑定组布局。
pub struct Layouts {
    pub globals: BindGroupLayout,
    pub texture: BindGroupLayout,
    pub pipeline: PipelineLayout,
}

impl Layouts {
    pub fn new(device: &Device) -> Self {
        let globals = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("anima-globals-layout"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("anima-texture-layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("anima-pipeline-layout"),
            bind_group_layouts: &[Some(&globals), Some(&texture)],
            immediate_size: 0,
        });
        Self { globals, texture, pipeline }
    }
}

/// 全部渲染管线。
pub struct Pipelines {
    /// 按 [`blend_index`] 排列：normal / multiply / screen / additive。
    pub blend: [RenderPipeline; 4],
    /// 遮罩裁剪：`layer *= mask_alpha`（全屏三角形，采样遮罩纹理）。
    pub mask_apply_in: RenderPipeline,
    /// 反向遮罩：`layer *= (1 - mask_alpha)`。
    pub mask_apply_out: RenderPipeline,
    /// 把临时层合成回主目标。
    pub composite: RenderPipeline,
}

impl Pipelines {
    pub fn new(device: &Device, layouts: &Layouts, format: TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("anima-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let mesh = |label: &str, blend: BlendState| {
            make_pipeline(device, layouts, &shader, label, "vs_main", "fs_main", blend, format, true)
        };

        let blend = [
            mesh("anima-normal", normal_blend()),
            mesh("anima-multiply", multiply_blend()),
            mesh("anima-screen", screen_blend()),
            mesh("anima-additive", additive_blend()),
        ];

        // 遮罩应用与层合成都是全屏三角形（采样纹理），只是混合状态不同
        let fullscreen = |label: &str, blend: BlendState| {
            make_pipeline(
                device,
                layouts,
                &shader,
                label,
                "vs_fullscreen",
                "fs_composite",
                blend,
                format,
                false,
            )
        };
        let mask_apply_in = fullscreen("anima-mask-in", mask_in_blend());
        let mask_apply_out = fullscreen("anima-mask-out", mask_out_blend());
        let composite = fullscreen("anima-composite", normal_blend());

        Self { blend, mask_apply_in, mask_apply_out, composite }
    }

    pub fn blend_for(&self, blend: BlendType) -> &RenderPipeline {
        &self.blend[blend_index(blend)]
    }
}

#[allow(clippy::too_many_arguments)]
fn make_pipeline(
    device: &Device,
    layouts: &Layouts,
    shader: &wgpu::ShaderModule,
    label: &str,
    vs_entry: &str,
    fs_entry: &str,
    blend: BlendState,
    format: TextureFormat,
    vertex_buffer: bool,
) -> RenderPipeline {
    let buffers = [Some(VertexBufferLayout {
        array_stride: VERTEX_STRIDE,
        step_mode: VertexStepMode::Vertex,
        attributes: &VERTEX_ATTRS,
    })];
    let buffers: &[Option<VertexBufferLayout>] = if vertex_buffer { &buffers } else { &[] };

    device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layouts.pipeline),
        vertex: VertexState {
            module: shader,
            entry_point: Some(vs_entry),
            compilation_options: PipelineCompilationOptions::default(),
            buffers,
        },
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: MultisampleState::default(),
        fragment: Some(FragmentState {
            module: shader,
            entry_point: Some(fs_entry),
            compilation_options: PipelineCompilationOptions::default(),
            targets: &[Some(ColorTargetState {
                format,
                blend: Some(blend),
                write_mask: ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn component(src_factor: wgpu::BlendFactor, dst_factor: wgpu::BlendFactor) -> BlendComponent {
    BlendComponent { src_factor, dst_factor, operation: BlendOperation::Add }
}

/// 预乘 alpha 的常规混合。
pub fn normal_blend() -> BlendState {
    BlendState {
        color: component(wgpu::BlendFactor::One, wgpu::BlendFactor::OneMinusSrcAlpha),
        alpha: component(wgpu::BlendFactor::One, wgpu::BlendFactor::OneMinusSrcAlpha),
    }
}

/// `out = src * dst`
pub fn multiply_blend() -> BlendState {
    BlendState {
        color: component(wgpu::BlendFactor::Dst, wgpu::BlendFactor::Zero),
        alpha: component(wgpu::BlendFactor::One, wgpu::BlendFactor::OneMinusSrcAlpha),
    }
}

/// `out = src + dst * (1 - src)`
pub fn screen_blend() -> BlendState {
    BlendState {
        color: component(wgpu::BlendFactor::One, wgpu::BlendFactor::OneMinusSrc),
        alpha: component(wgpu::BlendFactor::One, wgpu::BlendFactor::OneMinusSrcAlpha),
    }
}

/// `out = src + dst`
pub fn additive_blend() -> BlendState {
    BlendState {
        color: component(wgpu::BlendFactor::One, wgpu::BlendFactor::One),
        alpha: component(wgpu::BlendFactor::One, wgpu::BlendFactor::One),
    }
}

fn mask_in_blend() -> BlendState {
    BlendState {
        color: component(wgpu::BlendFactor::Zero, wgpu::BlendFactor::SrcAlpha),
        alpha: component(wgpu::BlendFactor::Zero, wgpu::BlendFactor::SrcAlpha),
    }
}

fn mask_out_blend() -> BlendState {
    BlendState {
        color: component(wgpu::BlendFactor::Zero, wgpu::BlendFactor::OneMinusSrcAlpha),
        alpha: component(wgpu::BlendFactor::Zero, wgpu::BlendFactor::OneMinusSrcAlpha),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_indices_are_stable() {
        assert_eq!(blend_index(BlendType::Normal), 0);
        assert_eq!(blend_index(BlendType::Multiply), 1);
        assert_eq!(blend_index(BlendType::Screen), 2);
        assert_eq!(blend_index(BlendType::Additive), 3);
    }

    #[test]
    fn blend_states_match_documented_formulas() {
        // multiply: src_factor = Dst → out = src*dst
        let m = multiply_blend();
        assert_eq!(m.color.src_factor, wgpu::BlendFactor::Dst);
        assert_eq!(m.color.dst_factor, wgpu::BlendFactor::Zero);
        // screen: out = src + dst*(1-src)
        let s = screen_blend();
        assert_eq!(s.color.dst_factor, wgpu::BlendFactor::OneMinusSrc);
        // additive
        let a = additive_blend();
        assert_eq!(a.color.dst_factor, wgpu::BlendFactor::One);
    }

    #[test]
    fn vertex_stride_matches_shader_attributes() {
        assert_eq!(VERTEX_STRIDE, 20);
        assert_eq!(VERTEX_ATTRS.len(), 3);
        assert_eq!(VERTEX_ATTRS[0].offset, 0);
        assert_eq!(VERTEX_ATTRS[1].offset, 8);
        assert_eq!(VERTEX_ATTRS[2].offset, 16);
    }
}
