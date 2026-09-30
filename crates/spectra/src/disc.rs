//! The disc, drawn with wgpu inside an iced `Shader` widget.
//!
//! It renders into its own multisampled target with a depth buffer - a disc
//! is thin, and its edges alias badly otherwise - then lays the result over
//! whatever the UI drew beneath it. Geometry is built once; each frame costs
//! one small uniform upload and two draw calls, and when nothing moves no
//! frames are drawn at all.

use std::f32::consts::TAU;
use std::sync::Arc;

use glam::{Mat4, Vec3};
use iced::wgpu::{self, util::DeviceExt};
use iced::widget::shader::{self, Viewport};
use iced::{Rectangle, mouse};

use crate::art::{HUB_RATIO, Label};
use crate::motion::Pose;

const DISC_R: f32 = 1.6;
const HOLE_R: f32 = DISC_R * 0.125;
const THICKNESS: f32 = 0.022;
const SEGMENTS: usize = 256;
/// The radius fitted on screen: a little more than the disc, so tilt and
/// perspective never push it past the edge.
const FIT_R: f32 = DISC_R * 1.1;
const FOV_DEGREES: f32 = 32.0;
const SAMPLES: u32 = 4;
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Where the scan stops and the metallised lip begins, as in Rainbow Player.
const ART_OUTER_RATIO: f32 = 0.945;

/// What the widget draws this frame.
pub struct Disc {
    pub pose: Pose,
    pub label: Arc<Label>,
    pub accent: [f32; 3],
    /// Track pitch in nm: 1600 for a CD, 740 for a DVD.
    pub pitch: f32,
}

impl<Message> shader::Program<Message> for Disc {
    type State = ();
    type Primitive = Primitive;

    fn draw(&self, _state: &(), _cursor: mouse::Cursor, _bounds: Rectangle) -> Primitive {
        Primitive {
            pose: self.pose,
            label: self.label.clone(),
            accent: self.accent,
            pitch: self.pitch,
        }
    }
}

#[derive(Debug)]
pub struct Primitive {
    pose: Pose,
    label: Arc<Label>,
    accent: [f32; 3],
    pitch: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    camera: [f32; 4],
    sky: [f32; 4],
    floor: [f32; 4],
    key_dir: [f32; 4],
    key_color: [f32; 4],
    fill_dir: [f32; 4],
    fill_color: [f32; 4],
    accent: [f32; 4],
    params: [f32; 4],
    radii: [f32; 4],
}

/// sRGB hex to linear, the way three.js stores colours.
fn linear(hex: u32) -> Vec3 {
    let channel = |v: u32| {
        let c = (v & 0xff) as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    Vec3::new(channel(hex >> 16), channel(hex >> 8), channel(hex))
}

fn v4(v: Vec3, w: f32) -> [f32; 4] {
    [v.x, v.y, v.z, w]
}

/// The two faces as flat rings, and the outer and inner edges as open
/// cylinders. Kind 0 is the label, 1 the read side, 2 an edge.
fn mesh() -> Vec<Vertex> {
    let mut out = Vec::with_capacity(SEGMENTS * 24);
    let half = THICKNESS / 2.0;
    let at = |i: usize| {
        let a = i as f32 / SEGMENTS as f32 * TAU;
        (a.cos(), a.sin())
    };
    let mut quad =
        |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3], n: [[f32; 3]; 4], kind: u32| {
            for (p, n) in [
                (a, n[0]),
                (b, n[1]),
                (c, n[2]),
                (a, n[0]),
                (c, n[2]),
                (d, n[3]),
            ] {
                out.push(Vertex {
                    position: p,
                    normal: n,
                    kind,
                });
            }
        };
    for i in 0..SEGMENTS {
        let ((c0, s0), (c1, s1)) = (at(i), at(i + 1));
        for (z, nz, kind) in [(half, 1.0, 0), (-half, -1.0, 1)] {
            let n = [0.0, 0.0, nz];
            quad(
                [c0 * HOLE_R, s0 * HOLE_R, z],
                [c0 * DISC_R, s0 * DISC_R, z],
                [c1 * DISC_R, s1 * DISC_R, z],
                [c1 * HOLE_R, s1 * HOLE_R, z],
                [n; 4],
                kind,
            );
        }
        for (r, sign) in [(DISC_R, 1.0), (HOLE_R, -1.0)] {
            let (n0, n1) = ([c0 * sign, s0 * sign, 0.0], [c1 * sign, s1 * sign, 0.0]);
            quad(
                [c0 * r, s0 * r, half],
                [c0 * r, s0 * r, -half],
                [c1 * r, s1 * r, -half],
                [c1 * r, s1 * r, half],
                [n0, n0, n1, n1],
                2,
            );
        }
    }
    out
}

struct Targets {
    size: (u32, u32),
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolve: wgpu::TextureView,
    composite: wgpu::BindGroup,
}

pub struct Pipeline {
    disc: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    vertex_count: u32,
    uniforms: wgpu::Buffer,
    options: wgpu::Buffer,
    disc_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    art_sampler: wgpu::Sampler,
    plain_sampler: wgpu::Sampler,
    bind: Option<wgpu::BindGroup>,
    label_id: u64,
    targets: Option<Targets>,
    /// Where the widget is, in physical pixels, as of the last prepare.
    bounds: (f32, f32, f32, f32),
}

impl shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let disc_module = device.create_shader_module(wgpu::include_wgsl!("disc.wgsl"));
        let composite_module = device.create_shader_module(wgpu::include_wgsl!("composite.wgsl"));

        let disc_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("disc"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(1),
                sampler_entry(2),
            ],
        });
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("disc composite"),
            entries: &[
                texture_entry(0),
                sampler_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let disc = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("disc"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("disc"),
                bind_group_layouts: &[&disc_layout],
                push_constant_ranges: &[],
            })),
            vertex: wgpu::VertexState {
                module: &disc_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Uint32],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &disc_module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: SAMPLES, ..Default::default() },
            multiview: None,
            cache: None,
        });

        let composite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("disc composite"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("disc composite"),
                    bind_group_layouts: &[&composite_layout],
                    push_constant_ranges: &[],
                }),
            ),
            vertex: wgpu::VertexState {
                module: &composite_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &composite_module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let vertices = mesh();
        let options = [if format.is_srgb() { 0.0f32 } else { 1.0 }, 0.0, 0.0, 0.0];
        Self {
            disc,
            composite,
            vertex_count: vertices.len() as u32,
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("disc mesh"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            uniforms: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("disc uniforms"),
                size: std::mem::size_of::<Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            options: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("disc composite options"),
                contents: bytemuck::cast_slice(&options),
                usage: wgpu::BufferUsages::UNIFORM,
            }),
            disc_layout,
            composite_layout,
            art_sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("label"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                anisotropy_clamp: 8,
                ..Default::default()
            }),
            plain_sampler: device.create_sampler(&wgpu::SamplerDescriptor::default()),
            bind: None,
            label_id: 0,
            targets: None,
            bounds: (0.0, 0.0, 0.0, 0.0),
        }
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

impl Pipeline {
    fn upload_label(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, label: &Label) {
        let base = &label.levels[0];
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("label"),
            size: wgpu::Extent3d {
                width: base.width(),
                height: base.height(),
                depth_or_array_layers: 1,
            },
            mip_level_count: label.levels.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, image) in label.levels.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                image.as_raw(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * image.width()),
                    rows_per_image: Some(image.height()),
                },
                wgpu::Extent3d {
                    width: image.width(),
                    height: image.height(),
                    depth_or_array_layers: 1,
                },
            );
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("disc"),
            layout: &self.disc_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.art_sampler),
                },
            ],
        }));
        self.label_id = label.id;
    }

    fn ensure_targets(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let texture = |label, format, samples, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size.0,
                        height: size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let attachment = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let resolve = texture(
            "disc resolve",
            COLOR_FORMAT,
            1,
            attachment | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let composite = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("disc composite"),
            layout: &self.composite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&resolve),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.plain_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.options.as_entire_binding(),
                },
            ],
        });
        self.targets = Some(Targets {
            size,
            color: texture("disc color", COLOR_FORMAT, SAMPLES, attachment),
            depth: texture("disc depth", DEPTH_FORMAT, SAMPLES, attachment),
            resolve,
            composite,
        });
    }
}

impl shader::Primitive for Primitive {
    type Pipeline = Pipeline;

    fn prepare(
        &self,
        pipeline: &mut Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        let scale = viewport.scale_factor();
        let (x, y) = (bounds.x * scale, bounds.y * scale);
        let (w, h) = (
            (bounds.width * scale).round().max(1.0),
            (bounds.height * scale).round().max(1.0),
        );
        pipeline.bounds = (x, y, w, h);
        pipeline.ensure_targets(device, (w as u32, h as u32));
        if pipeline.label_id != self.label.id {
            pipeline.upload_label(device, queue, &self.label);
        }

        // Fit the disc to the widget with a margin, looking straight down -z.
        let pad = (w.min(h) * 0.06).max(20.0 * scale);
        let radius_px = (w.min(h) / 2.0 - pad).max(1.0);
        let tan_half = (FOV_DEGREES.to_radians() / 2.0).tan();
        let camera = Vec3::new(0.0, 0.0, FIT_R * (h / 2.0) / (radius_px * tan_half));
        let projection = glam::camera::rh::proj::directx::perspective(
            FOV_DEGREES.to_radians(),
            w / h,
            0.1,
            100.0,
        );
        let view = glam::camera::rh::view::look_at_mat4(camera, Vec3::ZERO, Vec3::Y);

        let pose = self.pose;
        let scale_by = 0.92 + 0.08 * pose.presence;
        let model = Mat4::from_rotation_x(pose.tilt)
            * Mat4::from_rotation_y(pose.turn)
            * Mat4::from_rotation_z(pose.spin)
            * Mat4::from_scale(Vec3::splat(scale_by));

        // The room takes a little of the artwork's colour, so the disc sits
        // in its own light. Used as-is, as Rainbow Player's three.js did.
        let accent = Vec3::from(self.accent);
        let sky = linear(0x2a3350).lerp(accent, 0.28);
        let uniforms = Uniforms {
            view_proj: (projection * view).to_cols_array_2d(),
            model: model.to_cols_array_2d(),
            camera: v4(camera, 1.0),
            sky: v4(sky, 1.0),
            floor: v4(linear(0x05060a), 1.0),
            key_dir: v4(Vec3::new(-0.45, 0.85, 0.75).normalize(), 0.0),
            key_color: v4(Vec3::ONE, 1.0),
            fill_dir: v4(Vec3::new(0.9, -0.3, 0.5).normalize(), 0.0),
            fill_color: v4(linear(0x6f8cff), 1.0),
            accent: v4(accent, 1.0),
            params: [self.pitch, 1.0, pose.presence, 1.0],
            radii: [
                DISC_R,
                DISC_R * HUB_RATIO,
                DISC_R * 0.97,
                DISC_R * ART_OUTER_RATIO,
            ],
        };
        queue.write_buffer(&pipeline.uniforms, 0, bytemuck::bytes_of(&uniforms));
    }

    fn render(
        &self,
        pipeline: &Pipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        let (Some(targets), Some(bind)) = (&pipeline.targets, &pipeline.bind) else {
            return;
        };
        if clip_bounds.width == 0 || clip_bounds.height == 0 {
            return;
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("disc"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.color,
                    depth_slice: None,
                    resolve_target: Some(&targets.resolve),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&pipeline.disc);
            pass.set_bind_group(0, bind, &[]);
            pass.set_vertex_buffer(0, pipeline.vertices.slice(..));
            pass.draw(0..pipeline.vertex_count, 0..1);
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("disc composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        let (x, y, w, h) = pipeline.bounds;
        pass.set_viewport(x, y, w, h, 0.0, 1.0);
        pass.set_scissor_rect(
            clip_bounds.x,
            clip_bounds.y,
            clip_bounds.width,
            clip_bounds.height,
        );
        pass.set_pipeline(&pipeline.composite);
        pass.set_bind_group(0, &targets.composite, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn mesh_is_whole_triangles() {
        let mesh = super::mesh();
        assert_eq!(mesh.len() % 3, 0);
        assert_eq!(mesh.len(), super::SEGMENTS * 4 * 6);
    }
}
