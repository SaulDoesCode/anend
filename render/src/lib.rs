//! The space drawn on wgpu (the checklist's renderer line, queued request #25): what the
//! room holds as plain data (meshes with their textures, lines, labels, the sky and its
//! fog), a camera, and a renderer that draws them into a picture. The desktop shows that
//! picture under its (transparent) webview; the web will draw the same into a WebGPU
//! canvas. No JavaScript is involved.
//!
//! Meshes are unlit, as space.js's were: their vertex colours carry the shading (faces by
//! their direction) and the baked lights, a texture (a recipe's tile, a picture, a text's
//! tile) multiplies them. Passes: the sky (behind everything, no fog), the opaque blocks,
//! see-through ones (zones, ghosts) blended without writing depth, then what's drawn over
//! everything (the gizmo, outlines, ports), and the labels last.

pub mod blocks;
pub mod prims;
pub mod room;
pub mod text;
pub mod tiles;

use std::collections::HashMap;
use std::sync::Arc;

pub use glam;
use glam::{Mat4, Quat, Vec3};
use wgpu::util::DeviceExt;

/// a block as it's drawn (a convenience: it becomes a mesh)
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub at: Vec3,
    pub rot: Quat,
    /// width, height, depth
    pub size: Vec3,
    /// linear RGB, 0–1
    pub colour: [f32; 3],
    pub sphere: bool,
}

/// a straight line in the room (a ray) or a chain of points
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub points: Vec<Vec3>,
    pub colour: [f32; 3],
}

/// a text over the room, placed on the picture as the webview's labels were: its middle
/// at a point of the picture, so many pixels an em
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    /// its middle, in pixels from the picture's top left
    pub x: f32,
    pub y: f32,
    /// pixels an em
    pub px: f32,
    pub text: String,
    /// linear RGBA, premultiplied (alpha: how faded it is with distance)
    pub colour: [f32; 4],
    /// an underline under it, in this colour
    pub under: Option<[f32; 4]>,
    /// a band behind it (a picked text), in this colour
    pub back: Option<[f32; 4]>,
    /// the width it wraps at, in ems
    pub wrap: f32,
    /// an outline round it (one of many selected), in this colour
    pub ring: Option<[f32; 4]>,
}

/// a rectangle over the room (a box being drawn to select), in pixels
#[derive(Clone, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// linear RGBA, premultiplied
    pub fill: [f32; 4],
    pub border: [f32; 4],
}

/// a vertex of a mesh: where (in the mesh's own frame), where in its texture (in tiles: a
/// texture repeats), and its colour (linear RGB, straight alpha)
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vert {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub colour: [f32; 4],
}

/// how a mesh is drawn
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pass {
    /// round the room, behind everything, with no fog
    Sky,
    Opaque,
    /// blended, writing no depth (a zone, a ghost, a wire's glow)
    Glass,
    /// over everything (the gizmo, outlines, ports)
    Over,
}

/// triangles to draw, kept on the GPU while their `id` and `version` stay
#[derive(Clone, Debug)]
pub struct Mesh {
    pub id: u64,
    pub version: u64,
    pub verts: Arc<Vec<Vert>>,
    /// a texture's key in the scene's textures (none: white)
    pub texture: Option<String>,
    pub pass: Pass,
    /// where it is in the room
    pub model: Mat4,
    /// whether back faces are left out
    pub cull: bool,
    /// lines instead of triangles (each two vertices one segment)
    pub lines: bool,
}

/// a picture a mesh repeats (sRGB RGBA), kept on the GPU while its version stays
#[derive(Clone, Debug)]
pub struct Texture {
    pub version: u64,
    pub w: u32,
    pub h: u32,
    pub rgba: Arc<Vec<u8>>,
}

#[derive(Clone, Debug, Default)]
pub struct Scene {
    /// the room's colour (linear RGB)
    pub sky: [f32; 3],
    /// fog from this far to that far, in the room's colour
    pub fog: Option<(f32, f32)>,
    pub blocks: Vec<Block>,
    pub lines: Vec<Line>,
    pub labels: Vec<Label>,
    pub rects: Vec<Rect>,
    pub meshes: Vec<Mesh>,
    pub textures: HashMap<String, Texture>,
    /// what the picture starts as, when not the sky (a see-through thumbnail)
    pub clear: Option<[f32; 4]>,
}

impl PartialEq for Scene {
    fn eq(&self, o: &Self) -> bool {
        self.sky == o.sky
            && self.blocks == o.blocks
            && self.lines == o.lines
            && self.labels == o.labels
            && self.meshes.len() == o.meshes.len()
    }
}

/// where the room is seen from
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub at: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    /// vertical field of view, radians
    pub fov: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            at: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            fov: 50f32.to_radians(),
        }
    }
}

impl Camera {
    /// its turn: yaw about the world's up, then pitch, then roll (three.js's YXZ order)
    pub fn rot(&self) -> Quat {
        Quat::from_euler(glam::EulerRot::YXZ, self.yaw, self.pitch, self.roll)
    }

    /// the way it looks (−z at yaw and pitch 0, as three.js's camera)
    pub fn forward(&self) -> Vec3 {
        self.rot() * Vec3::NEG_Z
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let rot = self.rot();
        let view = glam::camera::rh::view::look_to_mat4(self.at, rot * Vec3::NEG_Z, rot * Vec3::Y);
        glam::camera::rh::proj::directx::perspective(self.fov, aspect.max(1e-3), 0.05, 4000.0) * view
    }
}

const SHADER: &str = r#"
struct U { vp: mat4x4<f32>, eye: vec4<f32>, fog: vec4<f32>, range: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;
struct M { model: mat4x4<f32>, flags: vec4<f32> };
@group(1) @binding(0) var<uniform> m: M;
@group(2) @binding(0) var tex: texture_2d<f32>;
@group(2) @binding(1) var smp: sampler;

struct Out { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) colour: vec4<f32>, @location(2) world: vec3<f32> };

@vertex fn vs(@location(0) pos: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) colour: vec4<f32>) -> Out {
    var o: Out;
    let w = m.model * vec4<f32>(pos, 1.0);
    o.pos = u.vp * w;
    o.uv = uv;
    o.colour = colour;
    o.world = w.xyz;
    return o;
}

@fragment fn fs(i: Out) -> @location(0) vec4<f32> {
    var c = i.colour * textureSample(tex, smp, i.uv);
    // fog, unless the mesh is out of it (the sky, what's drawn over everything)
    if (u.fog.w > 0.5 && m.flags.x < 0.5) {
        let d = distance(i.world, u.eye.xyz);
        let f = clamp((d - u.range.x) / max(u.range.y - u.range.x, 0.001), 0.0, 1.0);
        c = vec4<f32>(mix(c.rgb, u.fog.rgb, f), c.a);
    }
    return vec4<f32>(c.rgb * c.a, c.a);
}
"#;

const GLYPHS: &str = r#"
@group(0) @binding(0) var atlas: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;

struct Out { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) colour: vec4<f32> };

@vertex fn vs(@location(0) pos: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) colour: vec4<f32>) -> Out {
    var o: Out;
    // already placed on the picture
    o.pos = vec4<f32>(pos, 1.0);
    o.uv = uv;
    o.colour = colour;
    return o;
}

@fragment fn fs(i: Out) -> @location(0) vec4<f32> {
    let a = textureSample(atlas, smp, i.uv).r;
    return i.colour * a;
}
"#;

const ATLAS: u32 = 2048;
const SAMPLES: u32 = 4;
const COLOUR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// a mesh's uniform (its model and flags), at the device's offset alignment
const MODEL_STRIDE: u64 = 256;

/// meshes on the GPU: their version, vertex buffer and vertex count
struct GpuMesh {
    version: u64,
    buf: Option<wgpu::Buffer>,
    count: u32,
}

/// draws scenes into pictures of one size (`resize` for another)
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipes: HashMap<(Pass, bool, bool), wgpu::RenderPipeline>,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    model_layout: wgpu::BindGroupLayout,
    models: wgpu::Buffer,
    models_cap: u64,
    model_bind: wgpu::BindGroup,
    tex_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    white: wgpu::BindGroup,
    textures: HashMap<String, (u64, wgpu::BindGroup)>,
    gpu: HashMap<u64, GpuMesh>,
    width: u32,
    height: u32,
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    target: wgpu::Texture,
    readback: wgpu::Buffer,
    glyphs: wgpu::RenderPipeline,
    atlas: text::Atlas,
    atlas_tex: wgpu::Texture,
    atlas_bind: wgpu::BindGroup,
    layouts: HashMap<(String, u32), text::Layout>,
    pub adapter_name: String,
}

fn padded(width: u32) -> u32 {
    (width * 4).div_ceil(256) * 256
}

impl Renderer {
    pub fn new(width: u32, height: u32) -> Result<Renderer, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .map_err(|e| format!("no GPU adapter: {e}"))?;
        let adapter_name = adapter.get_info().name;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("anend space"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| format!("GPU device: {e}"))?;
        let uniform_entry = |dynamic: bool| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("space"),
            entries: &[uniform_entry(false)],
        });
        let model_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("models"),
            entries: &[wgpu::BindGroupLayoutEntry {
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(80),
                },
                ..uniform_entry(true)
            }],
        });
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("space uniform"),
            size: 112,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("space"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 4,
            ..Default::default()
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("space"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let playout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("space"),
            bind_group_layouts: &[Some(&layout), Some(&model_layout), Some(&tex_layout)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4];
        let mut pipes = HashMap::new();
        for pass in [Pass::Sky, Pass::Opaque, Pass::Glass, Pass::Over] {
            for cull in [false, true] {
                for lines in [false, true] {
                    let blend = (pass != Pass::Opaque).then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
                    let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("space"),
                        layout: Some(&playout),
                        vertex: wgpu::VertexState {
                            module: &module,
                            entry_point: Some("vs"),
                            compilation_options: Default::default(),
                            buffers: &[Some(wgpu::VertexBufferLayout {
                                array_stride: std::mem::size_of::<Vert>() as u64,
                                step_mode: wgpu::VertexStepMode::Vertex,
                                attributes: &attrs,
                            })],
                        },
                        primitive: wgpu::PrimitiveState {
                            topology: if lines {
                                wgpu::PrimitiveTopology::LineList
                            } else {
                                wgpu::PrimitiveTopology::TriangleList
                            },
                            cull_mode: (cull && !lines).then_some(if pass == Pass::Sky {
                                wgpu::Face::Front
                            } else {
                                wgpu::Face::Back
                            }),
                            ..Default::default()
                        },
                        depth_stencil: Some(wgpu::DepthStencilState {
                            format: DEPTH,
                            depth_write_enabled: Some(pass == Pass::Opaque),
                            depth_compare: Some(if matches!(pass, Pass::Over | Pass::Sky) {
                                wgpu::CompareFunction::Always
                            } else {
                                wgpu::CompareFunction::Less
                            }),
                            stencil: Default::default(),
                            bias: Default::default(),
                        }),
                        multisample: wgpu::MultisampleState {
                            count: SAMPLES,
                            ..Default::default()
                        },
                        fragment: Some(wgpu::FragmentState {
                            module: &module,
                            entry_point: Some("fs"),
                            compilation_options: Default::default(),
                            targets: &[Some(wgpu::ColorTargetState {
                                format: COLOUR,
                                blend,
                                write_mask: wgpu::ColorWrites::ALL,
                            })],
                        }),
                        multiview_mask: None,
                        cache: None,
                    });
                    pipes.insert((pass, cull, lines), p);
                }
            }
        }
        let models_cap = 256;
        let (models, model_bind) = Self::model_buffer(&device, &model_layout, models_cap);
        let white = Self::texture_bind(&device, &queue, &tex_layout, &sampler, 1, 1, &[255, 255, 255, 255]);
        let (glyphs, atlas_tex, atlas_bind) = Self::glyph_pipeline(&device);
        let mut atlas = text::Atlas::new(ATLAS);
        // a solid texel for underlines
        for i in [0, 1, ATLAS as usize, ATLAS as usize + 1] {
            atlas.pixels[i] = 255;
        }
        let (msaa, depth, target, readback) = Self::targets(&device, width, height);
        Ok(Renderer {
            device,
            queue,
            pipes,
            uniform,
            bind,
            model_layout,
            models,
            models_cap,
            model_bind,
            tex_layout,
            sampler,
            white,
            textures: HashMap::new(),
            gpu: HashMap::new(),
            width: width.max(1),
            height: height.max(1),
            msaa,
            depth,
            target,
            readback,
            glyphs,
            atlas,
            atlas_tex,
            atlas_bind,
            layouts: HashMap::new(),
            adapter_name,
        })
    }

    fn model_buffer(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        cap: u64,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("models"),
            size: cap * MODEL_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("models"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buf,
                    offset: 0,
                    size: wgpu::BufferSize::new(80),
                }),
            }],
        });
        (buf, bind)
    }

    fn texture_bind(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        w: u32,
        h: u32,
        rgba: &[u8],
    ) -> wgpu::BindGroup {
        let (w, h) = (w.max(1), h.max(1));
        let mips = (w.min(h) as f32).log2().floor() as u32 + 1;
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tile"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // the mip levels, halved on the CPU (box filtered)
        let mut level = rgba.to_vec();
        let (mut lw, mut lh) = (w, h);
        for mip in 0..mips {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: mip,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &level,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(lw * 4),
                    rows_per_image: Some(lh),
                },
                wgpu::Extent3d {
                    width: lw,
                    height: lh,
                    depth_or_array_layers: 1,
                },
            );
            if mip + 1 == mips {
                break;
            }
            let (nw, nh) = ((lw / 2).max(1), (lh / 2).max(1));
            let mut next = vec![0u8; (nw * nh * 4) as usize];
            for y in 0..nh {
                for x in 0..nw {
                    for c in 0..4 {
                        let px = |xx: u32, yy: u32| {
                            level[(((yy.min(lh - 1)) * lw + xx.min(lw - 1)) * 4 + c) as usize] as u32
                        };
                        let s =
                            px(2 * x, 2 * y) + px(2 * x + 1, 2 * y) + px(2 * x, 2 * y + 1) + px(2 * x + 1, 2 * y + 1);
                        next[((y * nw + x) * 4 + c) as usize] = (s / 4) as u8;
                    }
                }
            }
            level = next;
            (lw, lh) = (nw, nh);
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tile"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&tex.create_view(&Default::default())),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    /// the labels' pipeline: glyphs from the atlas, blended over the room
    fn glyph_pipeline(device: &wgpu::Device) -> (wgpu::RenderPipeline, wgpu::Texture, wgpu::BindGroup) {
        let gmod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glyphs"),
            source: wgpu::ShaderSource::Wgsl(GLYPHS.into()),
        });
        let tlayout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("atlas"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let atlas_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas"),
            size: wgpu::Extent3d {
                width: ATLAS,
                height: ATLAS,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let atlas_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas"),
            layout: &tlayout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&atlas_tex.create_view(&Default::default())),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let glayout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("glyphs"),
            bind_group_layouts: &[Some(&tlayout)],
            immediate_size: 0,
        });
        let gattrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4];
        let glyphs = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("glyphs"),
            layout: Some(&glayout),
            vertex: wgpu::VertexState {
                module: &gmod,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vert>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &gattrs,
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: SAMPLES,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &gmod,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOUR,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        (glyphs, atlas_tex, atlas_bind)
    }

    fn targets(
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> (wgpu::TextureView, wgpu::TextureView, wgpu::Texture, wgpu::Buffer) {
        let size = wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        };
        let tex = |format, samples, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("space"),
                size,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let msaa = tex(COLOUR, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default());
        let depth = tex(DEPTH, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default());
        let target = tex(
            COLOUR,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("space readback"),
            size: (padded(width.max(1)) * height.max(1)) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        (msaa, depth, target, readback)
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) == (self.width, self.height) {
            return;
        }
        let (msaa, depth, target, readback) = Self::targets(&self.device, width, height);
        (self.msaa, self.depth, self.target, self.readback) = (msaa, depth, target, readback);
        (self.width, self.height) = (width, height);
    }

    /// a label's layout (kept while it's asked for)
    pub fn layout(&mut self, text: &str, wrap: f32) -> &text::Layout {
        let k = (text.to_string(), (wrap * 16.0) as u32);
        if self.layouts.len() > 20_000 {
            self.layouts.clear();
        }
        self.layouts.entry(k).or_insert_with(|| text::layout(text, wrap))
    }

    /// the labels as glyph quads on the picture, in their order (the farthest first)
    fn label_quads(&mut self, scene: &Scene) -> Vec<Vert> {
        let (w, h) = (self.width as f32, self.height as f32);
        let mut out = Vec::new();
        let white = 0.5 / ATLAS as f32;
        for l in &scene.labels {
            let lay = self.layout(&l.text, l.wrap).clone();
            let s = l.px;
            let (ox, oy) = (l.x - lay.w * s / 2.0, l.y - lay.h * s / 2.0);
            let quad = |out: &mut Vec<Vert>, x0: f32, y0: f32, x1: f32, y1: f32, uv: [f32; 4], c: [f32; 4]| {
                let p = |x: f32, y: f32| [(ox + x * s) / w * 2.0 - 1.0, 1.0 - (oy + y * s) / h * 2.0, 0.0];
                let v = |x: f32, y: f32, u: f32, t: f32| Vert {
                    pos: p(x, y),
                    uv: [u, t],
                    colour: c,
                };
                let (a, b, cc, d) = (
                    v(x0, y0, uv[0], uv[1]),
                    v(x1, y0, uv[2], uv[1]),
                    v(x1, y1, uv[2], uv[3]),
                    v(x0, y1, uv[0], uv[3]),
                );
                out.extend_from_slice(&[a, b, cc, a, cc, d]);
            };
            if let Some(c) = l.back {
                quad(&mut out, 0.0, 0.0, lay.w, lay.h, [white, white, white, white], c);
            }
            for g in &lay.glyphs {
                if let Some(cell) = self.atlas.cell(g.ch) {
                    quad(
                        &mut out,
                        g.x + cell.at[0],
                        g.y + cell.at[1],
                        g.x + cell.at[2],
                        g.y + cell.at[3],
                        cell.uv,
                        l.colour,
                    );
                }
            }
            if let Some(c) = l.under {
                // the underline: .12em at the label's foot
                quad(
                    &mut out,
                    0.0,
                    lay.h - 0.12,
                    lay.w,
                    lay.h,
                    [white, white, white, white],
                    c,
                );
            }
            if let Some(c) = l.ring {
                // 2px wide, 2px out from the label (as `outline-offset`)
                let (o, t) = (2.0 / s, 2.0 / s);
                let wh = [white; 4];
                let (x0, y0, x1, y1) = (-o - t, -o - t, lay.w + o + t, lay.h + o + t);
                quad(&mut out, x0, y0, x1, y0 + t, wh, c);
                quad(&mut out, x0, y1 - t, x1, y1, wh, c);
                quad(&mut out, x0, y0 + t, x0 + t, y1 - t, wh, c);
                quad(&mut out, x1 - t, y0 + t, x1, y1 - t, wh, c);
            }
        }
        for r in &scene.rects {
            let p = |x: f32, y: f32| [x / w * 2.0 - 1.0, 1.0 - y / h * 2.0, 0.0];
            let mut q = |x0: f32, y0: f32, x1: f32, y1: f32, c: [f32; 4]| {
                let v = |x: f32, y: f32| Vert {
                    pos: p(x, y),
                    uv: [white, white],
                    colour: c,
                };
                let (a, b, cc, d) = (v(x0, y0), v(x1, y0), v(x1, y1), v(x0, y1));
                out.extend_from_slice(&[a, b, cc, a, cc, d]);
            };
            let (x1, y1) = (r.x + r.w, r.y + r.h);
            q(r.x, r.y, x1, y1, r.fill);
            q(r.x, r.y, x1, r.y + 1.0, r.border);
            q(r.x, y1 - 1.0, x1, y1, r.border);
            q(r.x, r.y + 1.0, r.x + 1.0, y1 - 1.0, r.border);
            q(x1 - 1.0, r.y + 1.0, x1, y1 - 1.0, r.border);
        }
        out
    }

    /// the scene as a picture: RGBA, row by row from the top, `width × height × 4` bytes
    pub fn render(&mut self, scene: &Scene, camera: &Camera) -> Vec<u8> {
        // the convenience shapes as meshes
        let mut meshes: Vec<Mesh> = scene.meshes.clone();
        let extra = convenience(scene);
        meshes.extend(extra);
        // textures, uploaded when new or changed
        for (k, t) in &scene.textures {
            if self.textures.get(k).is_none_or(|(v, _)| *v != t.version) {
                let b = Self::texture_bind(
                    &self.device,
                    &self.queue,
                    &self.tex_layout,
                    &self.sampler,
                    t.w,
                    t.h,
                    &t.rgba,
                );
                self.textures.insert(k.clone(), (t.version, b));
            }
        }
        // meshes, uploaded when new or changed; the rest let go
        let mut seen = std::collections::HashSet::new();
        for m in &meshes {
            seen.insert(m.id);
            if self.gpu.get(&m.id).is_none_or(|g| g.version != m.version) {
                let buf = (!m.verts.is_empty()).then(|| {
                    self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("mesh"),
                        contents: bytemuck::cast_slice(&m.verts),
                        usage: wgpu::BufferUsages::VERTEX,
                    })
                });
                self.gpu.insert(
                    m.id,
                    GpuMesh {
                        version: m.version,
                        buf,
                        count: m.verts.len() as u32,
                    },
                );
            }
        }
        self.gpu.retain(|id, _| seen.contains(id));
        // each mesh's model and flags
        let need = meshes.len().max(1) as u64;
        if need > self.models_cap {
            self.models_cap = need.next_power_of_two();
            let (b, g) = Self::model_buffer(&self.device, &self.model_layout, self.models_cap);
            (self.models, self.model_bind) = (b, g);
        }
        let mut mdata = vec![0u8; (need * MODEL_STRIDE) as usize];
        for (i, m) in meshes.iter().enumerate() {
            let mut u = [0f32; 20];
            u[..16].copy_from_slice(&m.model.to_cols_array());
            u[16] = if matches!(m.pass, Pass::Sky | Pass::Over) {
                1.0
            } else {
                0.0
            };
            let at = i * MODEL_STRIDE as usize;
            mdata[at..at + 80].copy_from_slice(bytemuck::cast_slice(&u));
        }
        self.queue.write_buffer(&self.models, 0, &mdata);
        let glyphs = self.label_quads(scene);
        if self.atlas.dirty {
            self.atlas.dirty = false;
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &self.atlas.pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ATLAS),
                    rows_per_image: Some(ATLAS),
                },
                wgpu::Extent3d {
                    width: ATLAS,
                    height: ATLAS,
                    depth_or_array_layers: 1,
                },
            );
        }
        let vp = camera.view_proj(self.width as f32 / self.height as f32);
        let mut u = [0f32; 28];
        u[..16].copy_from_slice(&vp.to_cols_array());
        u[16..19].copy_from_slice(&camera.at.to_array());
        u[20..23].copy_from_slice(&scene.sky);
        if let Some((near, far)) = scene.fog {
            u[23] = 1.0;
            u[24] = near;
            u[25] = far;
        }
        self.queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&u));
        let gb = (!glyphs.is_empty()).then(|| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("space glyphs"),
                contents: bytemuck::cast_slice(&glyphs),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        // the order: sky, opaque, see-through (the farthest first), over everything
        let rank = |p: Pass| match p {
            Pass::Sky => 0,
            Pass::Opaque => 1,
            Pass::Glass => 2,
            Pass::Over => 3,
        };
        let mut order: Vec<usize> = (0..meshes.len()).collect();
        order.sort_by(|&a, &b| {
            let (ma, mb) = (&meshes[a], &meshes[b]);
            rank(ma.pass).cmp(&rank(mb.pass)).then_with(|| {
                if ma.pass == Pass::Glass {
                    let da = ma.model.w_axis.truncate().distance_squared(camera.at);
                    let db = mb.model.w_axis.truncate().distance_squared(camera.at);
                    db.total_cmp(&da)
                } else {
                    std::cmp::Ordering::Equal
                }
            })
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let view = self.target.create_view(&Default::default());
            let [r, g, b, alpha] = scene.clear.unwrap_or([scene.sky[0], scene.sky[1], scene.sky[2], 1.0]);
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("space"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.msaa,
                    depth_slice: None,
                    resolve_target: Some(&view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: r as f64,
                            g: g as f64,
                            b: b as f64,
                            a: alpha as f64,
                        }),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind, &[]);
            for i in order {
                let m = &meshes[i];
                let Some(g) = self.gpu.get(&m.id) else { continue };
                let Some(buf) = &g.buf else { continue };
                let Some(p) = self.pipes.get(&(m.pass, m.cull, m.lines)) else {
                    continue;
                };
                pass.set_pipeline(p);
                pass.set_bind_group(1, &self.model_bind, &[(i as u64 * MODEL_STRIDE) as u32]);
                let tex = m
                    .texture
                    .as_ref()
                    .and_then(|k| self.textures.get(k))
                    .map(|t| &t.1)
                    .unwrap_or(&self.white);
                pass.set_bind_group(2, tex, &[]);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..g.count, 0..1);
            }
            if let Some(b) = &gb {
                pass.set_pipeline(&self.glyphs);
                pass.set_bind_group(0, &self.atlas_bind, &[]);
                pass.set_vertex_buffer(0, b.slice(..));
                pass.draw(0..glyphs.len() as u32, 0..1);
            }
        }
        let row = padded(self.width);
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([enc.finish()]);
        let slice = self.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("reading the picture back"));
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU poll");
        let mut out = Vec::with_capacity((self.width * self.height * 4) as usize);
        {
            let data = slice.get_mapped_range().expect("the picture, mapped");
            for y in 0..self.height {
                let at = (y * row) as usize;
                out.extend_from_slice(&data[at..at + (self.width * 4) as usize]);
            }
        }
        self.readback.unmap();
        out
    }
}

/// a block's faces shaded by their direction, as space.js's (the top brightest)
pub fn shade(n: Vec3) -> f32 {
    0.86 + 0.14 * n.y + 0.08 * n.z.abs()
}

/// a unit box's (or sphere's) triangles in a colour, shaded by direction
pub fn unit_shape(sphere: bool, colour: [f32; 4]) -> Vec<Vert> {
    let mut out = Vec::new();
    let mut push = |p: Vec3, n: Vec3, uv: [f32; 2]| {
        let s = shade(n);
        out.push(Vert {
            pos: p.to_array(),
            uv,
            colour: [colour[0] * s, colour[1] * s, colour[2] * s, colour[3]],
        });
    };
    if sphere {
        let (rings, segs) = (16, 24);
        let at = |i: u32, j: u32| {
            let th = std::f32::consts::PI * i as f32 / rings as f32;
            let ph = std::f32::consts::TAU * j as f32 / segs as f32;
            Vec3::new(th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin())
        };
        for i in 0..rings {
            for j in 0..segs {
                let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1));
                for v in [a, d, b, b, d, c] {
                    push(v * 0.5, v, [0.0, 0.0]);
                }
            }
        }
    } else {
        for (n, u, v) in [
            (Vec3::X, Vec3::Y, Vec3::Z),
            (Vec3::NEG_X, Vec3::Z, Vec3::Y),
            (Vec3::Y, Vec3::Z, Vec3::X),
            (Vec3::NEG_Y, Vec3::X, Vec3::Z),
            (Vec3::Z, Vec3::X, Vec3::Y),
            (Vec3::NEG_Z, Vec3::Y, Vec3::X),
        ] {
            let c = n * 0.5;
            let (a, b, c2, d) = (
                c - u * 0.5 - v * 0.5,
                c + u * 0.5 - v * 0.5,
                c + u * 0.5 + v * 0.5,
                c - u * 0.5 + v * 0.5,
            );
            for (p, uv) in [
                (a, [0.0, 1.0]),
                (b, [1.0, 1.0]),
                (c2, [1.0, 0.0]),
                (a, [0.0, 1.0]),
                (c2, [1.0, 0.0]),
                (d, [0.0, 0.0]),
            ] {
                push(p, n, uv);
            }
        }
    }
    out
}

/// the scene's convenience blocks and lines as meshes (ids from the top of the range)
fn convenience(scene: &Scene) -> Vec<Mesh> {
    let mut out = Vec::new();
    for (i, b) in scene.blocks.iter().enumerate() {
        let verts = unit_shape(b.sphere, [b.colour[0], b.colour[1], b.colour[2], 1.0]);
        let h = (b.sphere as u64)
            | ((b.colour[0].to_bits() as u64) << 1)
                ^ ((b.colour[1].to_bits() as u64) << 17)
                ^ ((b.colour[2].to_bits() as u64) << 33);
        out.push(Mesh {
            id: u64::MAX - i as u64,
            version: h,
            verts: Arc::new(verts),
            texture: None,
            pass: Pass::Opaque,
            model: Mat4::from_scale_rotation_translation(b.size, b.rot, b.at),
            cull: true,
            lines: false,
        });
    }
    let mut lv = Vec::new();
    for l in &scene.lines {
        for w in l.points.windows(2) {
            for p in w {
                lv.push(Vert {
                    pos: p.to_array(),
                    uv: [0.0, 0.0],
                    colour: [l.colour[0], l.colour[1], l.colour[2], 1.0],
                });
            }
        }
    }
    if !lv.is_empty() {
        let v = lv.iter().fold(0u64, |h, x| {
            h.wrapping_mul(31)
                .wrapping_add(x.pos[0].to_bits() as u64 ^ (x.pos[1].to_bits() as u64) << 20)
        });
        out.push(Mesh {
            id: u64::MAX - 1_000_000,
            version: v,
            verts: Arc::new(lv),
            texture: None,
            pass: Pass::Opaque,
            model: Mat4::IDENTITY,
            cull: false,
            lines: true,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(img: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * w + x) * 4) as usize;
        [img[i], img[i + 1], img[i + 2]]
    }

    #[test]
    fn a_red_box_on_a_pale_sky() {
        let Ok(mut r) = Renderer::new(160, 120) else {
            eprintln!("no GPU here: skipped");
            return;
        };
        let scene = Scene {
            sky: [0.8, 0.7, 0.55],
            blocks: vec![Block {
                at: Vec3::new(0.0, 0.0, -5.0),
                rot: Quat::IDENTITY,
                size: Vec3::splat(1.5),
                colour: [0.9, 0.05, 0.05],
                sphere: false,
            }],
            lines: vec![Line {
                points: vec![Vec3::new(-3.0, -1.0, -6.0), Vec3::new(3.0, -1.0, -6.0)],
                colour: [0.0, 0.0, 1.0],
            }],
            ..Default::default()
        };
        let img = r.render(&scene, &Camera::default());
        assert_eq!(img.len(), 160 * 120 * 4);
        let mid = px(&img, 160, 80, 60);
        assert!(mid[0] > 150 && mid[1] < 80 && mid[2] < 80, "the box: {mid:?}");
        let corner = px(&img, 160, 2, 2);
        assert!(corner[0] > 200 && corner[2] > 150, "the sky: {corner:?}");
        let below: Vec<[u8; 3]> = (60..120).map(|y| px(&img, 160, 40, y)).collect();
        assert!(below.iter().any(|p| p[2] > 200 && p[0] < 160), "the line: {below:?}");
        // a sphere is round: its corners are sky
        let ball = Scene {
            blocks: vec![Block {
                sphere: true,
                ..scene.blocks[0].clone()
            }],
            ..scene.clone()
        };
        let img = r.render(&ball, &Camera::default());
        assert!(px(&img, 160, 80, 60)[0] > 150);
        let edge = px(&img, 160, 80 - 22, 60 - 22);
        assert!(edge[0] > 200 && edge[2] > 120, "the sphere's corner: {edge:?}");
        // a label, drawn over the room in its colour
        let words = Scene {
            labels: vec![Label {
                x: 80.0,
                y: 60.0,
                px: 24.0,
                text: "HH".into(),
                colour: [0.0, 0.6, 0.0, 1.0],
                under: None,
                back: None,
                wrap: 30.0,
                ring: None,
            }],
            ..scene.clone()
        };
        let img = r.render(&words, &Camera::default());
        let greens = (0..160 * 120)
            .filter(|i| {
                let p = &img[i * 4..i * 4 + 3];
                p[1] > 150 && p[0] < 100
            })
            .count();
        assert!(greens > 20, "the label's green pixels: {greens}");
        // a textured mesh: a two-colour tile, repeated; fog fades what's far
        let tile: Vec<u8> = (0..4 * 4)
            .flat_map(|i| {
                if i % 2 == 0 {
                    [255, 255, 255, 255]
                } else {
                    [0, 0, 255, 255]
                }
            })
            .collect();
        let mut textures = HashMap::new();
        textures.insert(
            "t".to_string(),
            Texture {
                version: 1,
                w: 4,
                h: 4,
                rgba: Arc::new(tile),
            },
        );
        let tex = Scene {
            meshes: vec![Mesh {
                id: 1,
                version: 1,
                verts: Arc::new(unit_shape(false, [1.0, 1.0, 1.0, 1.0])),
                texture: Some("t".into()),
                pass: Pass::Opaque,
                model: Mat4::from_scale_rotation_translation(
                    Vec3::splat(1.5),
                    Quat::IDENTITY,
                    Vec3::new(0.0, 0.0, -5.0),
                ),
                cull: true,
                lines: false,
            }],
            textures,
            ..Default::default()
        };
        let img = r.render(&tex, &Camera::default());
        let blues = (40..80)
            .filter(|x| {
                let p = px(&img, 160, *x, 60);
                p[2] > 150 && p[0] < 120
            })
            .count();
        assert!(blues > 0, "the tile's blue");
        let fogged = Scene {
            fog: Some((0.0, 4.0)),
            ..tex.clone()
        };
        let img = r.render(&fogged, &Camera::default());
        let mid = px(&img, 160, 80, 60);
        assert!(mid[0] < 20 && mid[2] < 20, "all fog (the room's black): {mid:?}");
    }
}
