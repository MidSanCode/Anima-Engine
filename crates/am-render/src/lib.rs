//! `am-render`：基于 `wgpu` 的离屏渲染器。
//!
//! 定位：**引擎负责画，宿主负责显示**。本 crate 永远渲染到自己的离屏纹理，
//! 平台层（Windows / Web / 未来的 macOS·Linux·移动端）负责把这块纹理接到 UI 上。
//!
//! ```text
//! Scene ──► 顶点/索引缓冲 ──► 渲染通道 ──► 离屏 RGBA 纹理 ──► 读回 / 共享纹理
//!                    │
//!                    └─ 遮罩：临时层 + dst_in/dst_out 混合
//! ```
//!
//! 关键约定：
//! * 目标格式固定为 [`TARGET_FORMAT`]（`Rgba8Unorm`），**不做 sRGB 转换**，
//!   与编辑器取色、导出 PNG 的数值保持一致。
//! * 顶点已经是画布空间坐标，唯一变换是「画布 → NDC」的视图矩阵。
//! * 片元输出预乘 alpha，四种混合模式由固定管线实现（见 `shader.wgsl`）。
//! * 读回的像素第 0 行是画面**顶部**（与 PNG 一致），需要 OpenGL 式上下翻转时用
//!   [`texture::DecodedImage::flip_vertical`]。

mod texture;

pub mod pipeline;
pub mod view;

pub use texture::{decode_bytes, decode_file, DecodedImage, TextureError};
pub use view::View;

use am_eval::{DrawableInstance, Scene};
use am_math::Vec2;
use am_model::{BlendType, Id};
use std::collections::BTreeMap;
use wgpu::util::DeviceExt as _;

/// 离屏目标格式。
pub const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// 清屏颜色（完全透明）。
pub const CLEAR_COLOR: wgpu::Color = wgpu::Color::TRANSPARENT;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("没有可用的图形适配器：{0}")]
    NoAdapter(String),
    #[error("创建图形设备失败：{0}")]
    Device(String),
    #[error(transparent)]
    Texture(#[from] TextureError),
    #[error("渲染目标尺寸非法：{0}x{1}")]
    BadSize(u32, u32),
    #[error("回读像素失败")]
    MapFailed,
    #[error("尚未创建渲染目标")]
    NoTarget,
}

// ------------------------------------------------------------------ 顶点与全局量

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
    opacity: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view: [f32; 16],
    resolution: [f32; 4],
}

impl Globals {
    fn new(view: &View, width: u32, height: u32) -> Self {
        let m = view.to_matrix(width, height);
        Self {
            view: [
                m.a, m.b, 0.0, 0.0, //
                m.c, m.d, 0.0, 0.0, //
                0.0, 0.0, 1.0, 0.0, //
                m.tx, m.ty, 0.0, 1.0,
            ],
            resolution: [width as f32, height as f32, 0.0, 0.0],
        }
    }
}

// ------------------------------------------------------------------ 资源

struct GpuTexture {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

struct Attachment {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

/// 一帧里已经准备好 GPU 缓冲的绘制对象。
struct Prepared<'a> {
    drawable: &'a DrawableInstance,
    texture: &'a GpuTexture,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
}

/// 渲染器。
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter_info: String,
    format: wgpu::TextureFormat,
    globals: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    layouts: pipeline::Layouts,
    pipelines: pipeline::Pipelines,
    textures: Vec<Option<GpuTexture>>,
    target: Option<Attachment>,
    layer: Option<Attachment>,
    layer_bg: Option<wgpu::BindGroup>,
    mask: Option<Attachment>,
    mask_bg: Option<wgpu::BindGroup>,
    max_texture_size: u32,
}

impl Renderer {
    /// 创建离屏渲染器（不依赖窗口系统，可在无 GUI 环境下使用）。
    pub fn new() -> Result<Self, RenderError> {
        Self::with_size(1024, 1024)
    }

    /// 创建指定尺寸的离屏渲染器。
    pub fn with_size(width: u32, height: u32) -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(
            wgpu::InstanceDescriptor::new_without_display_handle().with_env(),
        );
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .map_err(|e| RenderError::NoAdapter(e.to_string()))?;

        let info = adapter.get_info();
        let limits = adapter.limits();
        let max_texture_size = limits.max_texture_dimension_2d;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("anima-device"),
            required_features: wgpu::Features::empty(),
            required_limits: limits,
            ..Default::default()
        }))
        .map_err(|e| RenderError::Device(e.to_string()))?;

        let layouts = pipeline::Layouts::new(&device);
        let pipelines = pipeline::Pipelines::new(&device, &layouts, TARGET_FORMAT);

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("anima-globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("anima-globals-bg"),
            layout: &layouts.globals,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("anima-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let mut renderer = Self {
            device,
            queue,
            adapter_info: format!("{} ({:?})", info.name, info.backend),
            format: TARGET_FORMAT,
            globals,
            globals_bg,
            sampler,
            layouts,
            pipelines,
            textures: Vec::new(),
            target: None,
            layer: None,
            layer_bg: None,
            mask: None,
            mask_bg: None,
            max_texture_size,
        };
        renderer.resize(width, height)?;
        Ok(renderer)
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    pub fn adapter_info(&self) -> &str {
        &self.adapter_info
    }

    pub fn max_texture_size(&self) -> u32 {
        self.max_texture_size
    }

    pub fn size(&self) -> (u32, u32) {
        self.target.as_ref().map(|t| (t.width, t.height)).unwrap_or((0, 0))
    }

    /// 重建离屏目标（尺寸变化时调用）。
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), RenderError> {
        if width == 0 || height == 0 {
            return Err(RenderError::BadSize(width, height));
        }
        let make = |label: &str, usage: wgpu::TextureUsages| {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            (texture, view)
        };

        let (texture, view) = make(
            "anima-target",
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        self.target = Some(Attachment { texture, view, width, height });

        let (texture, view) = make(
            "anima-layer",
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let layer_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("anima-layer-bg"),
            layout: &self.layouts.texture,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.layer = Some(Attachment { texture, view, width, height });
        self.layer_bg = Some(layer_bg);

        let (texture, view) = make(
            "anima-mask",
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let mask_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("anima-mask-bg"),
            layout: &self.layouts.texture,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.mask = Some(Attachment { texture, view, width, height });
        self.mask_bg = Some(mask_bg);
        Ok(())
    }

    /// 离屏目标的纹理视图（平台层共享纹理时用）。
    pub fn target_view(&self) -> Option<&wgpu::TextureView> {
        self.target.as_ref().map(|t| &t.view)
    }

    pub fn target_texture(&self) -> Option<&wgpu::Texture> {
        self.target.as_ref().map(|t| &t.texture)
    }

    // -------------------------------------------------------------- 纹理管理

    /// 上传一张纹理到指定下标（下标对应 `Model::textures`）。
    pub fn set_texture(&mut self, index: u32, image: &DecodedImage) -> Result<(), RenderError> {
        if image.width > self.max_texture_size || image.height > self.max_texture_size {
            return Err(TextureError::TooLarge {
                width: image.width,
                height: image.height,
                max: self.max_texture_size,
            }
            .into());
        }
        let size = wgpu::Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("anima-atlas"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &image.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(image.width * 4),
                rows_per_image: Some(image.height),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("anima-atlas-bg"),
            layout: &self.layouts.texture,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        let idx = index as usize;
        if self.textures.len() <= idx {
            self.textures.resize_with(idx + 1, || None);
        }
        self.textures[idx] = Some(GpuTexture {
            texture,
            bind_group,
            width: image.width,
            height: image.height,
        });
        Ok(())
    }

    /// 解码并上传（文件路径）。
    pub fn load_texture_file(
        &mut self,
        index: u32,
        path: impl AsRef<std::path::Path>,
    ) -> Result<(), RenderError> {
        let image = decode_file(path)?;
        self.set_texture(index, &image)
    }

    /// 解码并上传（内存字节）。
    pub fn load_texture_bytes(&mut self, index: u32, bytes: &[u8]) -> Result<(), RenderError> {
        let image = decode_bytes(bytes)?;
        self.set_texture(index, &image)
    }

    /// 丢弃全部纹理。
    pub fn clear_textures(&mut self) {
        self.textures.clear();
    }

    pub fn texture_count(&self) -> usize {
        self.textures.iter().filter(|t| t.is_some()).count()
    }

    pub fn texture_size(&self, index: u32) -> Option<(u32, u32)> {
        self.textures
            .get(index as usize)
            .and_then(|t| t.as_ref())
            .map(|t| (t.width, t.height))
    }

    // -------------------------------------------------------------- 渲染

    /// 渲染一帧到离屏目标。
    pub fn render(&mut self, scene: &Scene, view: &View) -> Result<(), RenderError> {
        if self.target.is_none() || self.layer.is_none() {
            return Err(RenderError::NoTarget);
        }
        let (width, height) = self.size();
        let globals = Globals::new(view, width, height);
        self.queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));

        let prepared = self.prepare(scene);
        let target_view = self.target.as_ref().unwrap().view.clone();
        let layer_view = self.layer.as_ref().unwrap().view.clone();
        let layer_bg = self.layer_bg.clone().unwrap();
        let mask_view = self.mask.as_ref().unwrap().view.clone();
        let mask_bg = self.mask_bg.clone().unwrap();

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("anima-frame") });

        let mut first = true;
        let mut mask_cache: BTreeMap<Id, Vec<Vec<usize>>> = BTreeMap::new();

        for item in prepared.iter() {
            // 不可见的绘制对象仍然可能作为遮罩来源，因此只在这里跳过
            if !item.drawable.visible || item.drawable.opacity <= 1e-4 {
                continue;
            }
            if item.drawable.masks.is_empty() {
                let load = if first { wgpu::LoadOp::Clear(CLEAR_COLOR) } else { wgpu::LoadOp::Load };
                let mut pass = begin_pass(&mut encoder, &target_view, load, "anima-main");
                draw_prepared(
                    &mut pass,
                    &self.pipelines.blend_for(item.drawable.blend),
                    &self.globals_bg,
                    item,
                );
                first = false;
                continue;
            }

            // 1) 绘制对象 → 临时层
            {
                let mut pass = begin_pass(
                    &mut encoder,
                    &layer_view,
                    wgpu::LoadOp::Clear(CLEAR_COLOR),
                    "anima-layer-draw",
                );
                draw_prepared(
                    &mut pass,
                    &self.pipelines.blend_for(item.drawable.blend),
                    &self.globals_bg,
                    item,
                );
            }

            // 2) 逐条遮罩裁剪临时层
            //
            // 注意：不能直接把遮罩几何画到层上用 dst_in —— 未被遮罩几何覆盖的像素
            // 根本不会执行片元着色器，因此不会被清零。正确做法是先把遮罩画进
            // 独立的遮罩纹理（清空后绘制），再用**全屏三角形**把它的 alpha 乘到层上。
            let groups: Vec<Vec<usize>> = mask_cache
                .entry(item.drawable.node.clone())
                .or_insert_with(|| mask_sources(scene, &prepared, &item.drawable.masks))
                .clone();
            let apply_pipeline = if item.drawable.inverted_mask {
                &self.pipelines.mask_apply_out
            } else {
                &self.pipelines.mask_apply_in
            };
            for group in groups.iter().filter(|g| !g.is_empty()) {
                {
                    let mut pass = begin_pass(
                        &mut encoder,
                        &mask_view,
                        wgpu::LoadOp::Clear(CLEAR_COLOR),
                        "anima-mask-draw",
                    );
                    for idx in group {
                        if let Some(src) = prepared.get(*idx) {
                            draw_prepared(
                                &mut pass,
                                &self.pipelines.blend_for(src.drawable.blend),
                                &self.globals_bg,
                                src,
                            );
                        }
                    }
                }
                let mut pass =
                    begin_pass(&mut encoder, &layer_view, wgpu::LoadOp::Load, "anima-mask-apply");
                pass.set_pipeline(apply_pipeline);
                pass.set_bind_group(0, &self.globals_bg, &[]);
                pass.set_bind_group(1, &mask_bg, &[]);
                pass.draw(0..3, 0..1);
            }

            // 3) 临时层 → 主目标
            let load = if first { wgpu::LoadOp::Clear(CLEAR_COLOR) } else { wgpu::LoadOp::Load };
            let mut pass = begin_pass(&mut encoder, &target_view, load, "anima-composite");
            pass.set_pipeline(&self.pipelines.composite);
            pass.set_bind_group(0, &self.globals_bg, &[]);
            pass.set_bind_group(1, &layer_bg, &[]);
            pass.draw(0..3, 0..1);
            first = false;
        }

        if first {
            // 本帧没有任何可见内容：至少要把目标清空，避免残留上一帧
            let _pass = begin_pass(
                &mut encoder,
                &target_view,
                wgpu::LoadOp::Clear(CLEAR_COLOR),
                "anima-clear",
            );
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        Ok(())
    }

    /// 渲染一帧并回读 RGBA8 像素（第 0 行是画面顶部）。
    pub fn render_to_pixels(&mut self, scene: &Scene, view: &View) -> Result<Vec<u8>, RenderError> {
        self.render(scene, view)?;
        self.read_pixels()
    }

    /// 渲染一帧并回读为 [`DecodedImage`]。
    pub fn render_to_image(
        &mut self,
        scene: &Scene,
        view: &View,
    ) -> Result<DecodedImage, RenderError> {
        let pixels = self.render_to_pixels(scene, view)?;
        let (width, height) = self.size();
        DecodedImage::new(width, height, pixels).map_err(Into::into)
    }

    /// 回读当前离屏目标。
    pub fn read_pixels(&self) -> Result<Vec<u8>, RenderError> {
        let target = self.target.as_ref().ok_or(RenderError::NoTarget)?;
        let (width, height) = (target.width, target.height);
        let unpadded = width * 4;
        let bytes_per_row = unpadded.div_ceil(256) * 256;

        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("anima-readback"),
            size: (bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("anima-readback") });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit(std::iter::once(encoder.finish()));

        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        rx.recv().map_err(|_| RenderError::MapFailed)?.map_err(|_| RenderError::MapFailed)?;

        let data = slice.get_mapped_range().map_err(|_| RenderError::MapFailed)?;
        let mut out = Vec::with_capacity((unpadded * height) as usize);
        for row in 0..height {
            let start = (row * bytes_per_row) as usize;
            out.extend_from_slice(&data[start..start + unpadded as usize]);
        }
        drop(data);
        buffer.unmap();
        Ok(out)
    }

    // -------------------------------------------------------------- 内部

    fn prepare<'a>(&'a self, scene: &'a Scene) -> Vec<Prepared<'a>> {
        let mut out = Vec::new();
        for drawable in &scene.drawables {
            // 注意：这里**不过滤可见性**，隐藏的绘制对象仍可作为遮罩来源
            if drawable.texture.is_none()
                || drawable.vertices.len() < 3
                || drawable.indices.len() < 3
            {
                continue;
            }
            let Some(index) = drawable.texture else { continue };
            let Some(Some(texture)) = self.textures.get(index as usize) else {
                continue;
            };
            let vertices = build_vertices(drawable);
            if vertices.len() < 3 || drawable.indices.len() < 3 {
                continue;
            }
            let vb = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("anima-vertices"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            let ib = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("anima-indices"),
                    contents: bytemuck::cast_slice(&drawable.indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
            out.push(Prepared {
                drawable,
                texture,
                vertices: vb,
                indices: ib,
                index_count: drawable.indices.len() as u32,
            });
        }
        out
    }
}

fn begin_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    view: &'a wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
    label: &'a str,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
        })],
        ..Default::default()
    })
}

fn draw_prepared<'a>(
    pass: &mut wgpu::RenderPass<'a>,
    pipeline: &'a wgpu::RenderPipeline,
    globals: &'a wgpu::BindGroup,
    item: &'a Prepared<'a>,
) {
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, globals, &[]);
    pass.set_bind_group(1, &item.texture.bind_group, &[]);
    pass.set_vertex_buffer(0, item.vertices.slice(..));
    pass.set_index_buffer(item.indices.slice(..), wgpu::IndexFormat::Uint32);
    pass.draw_indexed(0..item.index_count, 0, 0..1);
}

/// 把绘制对象变成 GPU 顶点（画布空间坐标 + 纹理坐标）。
fn build_vertices(drawable: &DrawableInstance) -> Vec<Vertex> {
    let count = drawable.vertices.len();
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let p = drawable.vertices[i];
        let mut uv = drawable.uvs.get(i).copied().unwrap_or(Vec2::ZERO);
        if let Some(rect) = drawable.uv_rect {
            uv = rect.min + Vec2::new(uv.x * rect.width(), uv.y * rect.height());
        }
        out.push(Vertex { pos: [p.x, p.y], uv: [uv.x, uv.y], opacity: drawable.opacity });
    }
    out
}

/// 解析遮罩来源：每个遮罩节点对应一组「该节点自身及其所有后代」的绘制对象。
fn mask_sources(scene: &Scene, prepared: &[Prepared<'_>], masks: &[Id]) -> Vec<Vec<usize>> {
    let mut out = Vec::with_capacity(masks.len());
    for mask in masks {
        let mut group = Vec::new();
        for (i, item) in prepared.iter().enumerate() {
            if is_descendant_or_self(scene, &item.drawable.node, mask) && !group.contains(&i) {
                group.push(i);
            }
        }
        out.push(group);
    }
    out
}

fn is_descendant_or_self(scene: &Scene, node: &str, ancestor: &str) -> bool {
    let mut current = Some(node.to_string());
    let mut guard = 0;
    while let Some(id) = current {
        guard += 1;
        if guard > scene.nodes.len() + 1 {
            return false;
        }
        if id == ancestor {
            return true;
        }
        current = scene.nodes.get(&id).and_then(|v| v.parent.clone());
    }
    false
}

/// 混合模式 → 管线（供外部查询）。
pub fn blend_index(blend: BlendType) -> usize {
    pipeline::blend_index(blend)
}
