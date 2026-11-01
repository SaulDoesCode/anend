//! The space drawn on wgpu (the checklist's renderer line, queued request #25): what the
//! room holds (blocks, wires, rays, the sky) as plain data, a camera, and a renderer that
//! draws them into a picture. The desktop shows that picture under its (transparent)
//! webview; the web will draw the same into a WebGPU canvas. No JavaScript is involved.
//!
//! This is the first slice: boxes and spheres in their colours, turned and sized, lit by
//! one soft light from above; lines (wires and rays); the sky's colour. Texts, textures,
//! baked lights, bends, the gizmo and picking come in the slices after it.

use glam::{Mat4, Quat, Vec3};
use wgpu::util::DeviceExt;

/// a block as it's drawn
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

/// a straight line in the room (a ray) or a cable (a wire, as a chain of points)
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub points: Vec<Vec3>,
    pub colour: [f32; 3],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    /// the room's colour (linear RGB)
    pub sky: [f32; 3],
    pub blocks: Vec<Block>,
    pub lines: Vec<Line>,
}

/// where the room is seen from
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub at: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    /// vertical field of view, radians
    pub fov: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            at: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            fov: 60f32.to_radians(),
        }
    }
}

impl Camera {
    /// the way it looks (−z at yaw and pitch 0, as three.js's camera)
    pub fn forward(&self) -> Vec3 {
        Quat::from_euler(glam::EulerRot::YXZ, self.yaw, self.pitch, 0.0) * Vec3::NEG_Z
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let rot = Quat::from_euler(glam::EulerRot::YXZ, self.yaw, self.pitch, 0.0);
        let view = glam::camera::rh::view::look_to_mat4(self.at, rot * Vec3::NEG_Z, rot * Vec3::Y);
        glam::camera::rh::proj::directx::perspective(self.fov, aspect.max(1e-3), 0.05, 4000.0) * view
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 3],
    normal: [f32; 3],
    colour: [f32; 3],
}

const SHADER: &str = r#"
struct U { vp: mat4x4<f32>, light: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;

struct Out { @builtin(position) pos: vec4<f32>, @location(0) normal: vec3<f32>, @location(1) colour: vec3<f32> };

@vertex fn vs(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) colour: vec3<f32>) -> Out {
    var o: Out;
    o.pos = u.vp * vec4<f32>(pos, 1.0);
    o.normal = normal;
    o.colour = colour;
    return o;
}

// soft light from above and a little in front; lines (no normal) as they are
@fragment fn fs(i: Out) -> @location(0) vec4<f32> {
    let n = i.normal;
    if (dot(n, n) < 0.01) { return vec4<f32>(i.colour, 1.0); }
    let l = normalize(u.light.xyz);
    let lit = 0.55 + 0.45 * max(dot(normalize(n), l), 0.0);
    return vec4<f32>(i.colour * lit, 1.0);
}
"#;

const SAMPLES: u32 = 4;
const COLOUR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// draws scenes into pictures of one size (`resize` for another)
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    tris: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    width: u32,
    height: u32,
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    target: wgpu::Texture,
    /// where pictures are read back
    readback: wgpu::Buffer,
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
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("space"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("space"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("space uniform"),
            size: 80,
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
        let playout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("space"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3];
        let pipeline = |topology: wgpu::PrimitiveTopology, cull: Option<wgpu::Face>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("space"),
                layout: Some(&playout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attrs,
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    cull_mode: cull,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
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
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let tris = pipeline(wgpu::PrimitiveTopology::TriangleList, Some(wgpu::Face::Back));
        let lines = pipeline(wgpu::PrimitiveTopology::LineList, None);
        let (msaa, depth, target, readback) = Self::targets(&device, width, height);
        Ok(Renderer {
            device,
            queue,
            tris,
            lines,
            uniform,
            bind,
            width,
            height,
            msaa,
            depth,
            target,
            readback,
            adapter_name,
        })
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
        if (width, height) == (self.width, self.height) {
            return;
        }
        let (msaa, depth, target, readback) = Self::targets(&self.device, width, height);
        (self.msaa, self.depth, self.target, self.readback) = (msaa, depth, target, readback);
        (self.width, self.height) = (width.max(1), height.max(1));
    }

    /// the scene as a picture: RGBA, row by row from the top, `width × height × 4` bytes
    pub fn render(&self, scene: &Scene, camera: &Camera) -> Vec<u8> {
        let (tris, lines) = meshes(scene);
        let vp = camera.view_proj(self.width as f32 / self.height as f32);
        let mut u = [0f32; 20];
        u[..16].copy_from_slice(&vp.to_cols_array());
        u[16..19].copy_from_slice(&[0.35, 1.0, 0.55]);
        self.queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&u));
        let buf = |v: &[Vertex]| {
            (!v.is_empty()).then(|| {
                self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("space vertices"),
                    contents: bytemuck::cast_slice(v),
                    usage: wgpu::BufferUsages::VERTEX,
                })
            })
        };
        let (tb, lb) = (buf(&tris), buf(&lines));
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let view = self.target.create_view(&Default::default());
            let [r, g, b] = scene.sky;
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
                            a: 1.0,
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
            if let Some(b) = &tb {
                pass.set_pipeline(&self.tris);
                pass.set_vertex_buffer(0, b.slice(..));
                pass.draw(0..tris.len() as u32, 0..1);
            }
            if let Some(b) = &lb {
                pass.set_pipeline(&self.lines);
                pass.set_vertex_buffer(0, b.slice(..));
                pass.draw(0..lines.len() as u32, 0..1);
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

/// the scene's triangles and line segments
fn meshes(scene: &Scene) -> (Vec<Vertex>, Vec<Vertex>) {
    let mut tris = Vec::new();
    for b in &scene.blocks {
        let m = Mat4::from_scale_rotation_translation(b.size, b.rot, b.at);
        let push = |tris: &mut Vec<Vertex>, p: Vec3, n: Vec3| {
            tris.push(Vertex {
                pos: m.transform_point3(p).to_array(),
                normal: (b.rot * n).normalize_or_zero().to_array(),
                colour: b.colour,
            })
        };
        if b.sphere {
            // a UV sphere of radius ½ in the unit box
            let (rings, segs) = (16, 24);
            let at = |i: u32, j: u32| {
                let th = std::f32::consts::PI * i as f32 / rings as f32;
                let ph = std::f32::consts::TAU * j as f32 / segs as f32;
                Vec3::new(th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin())
            };
            for i in 0..rings {
                for j in 0..segs {
                    let (a, b2, c, d) = (at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1));
                    for v in [a, d, b2, b2, d, c] {
                        push(&mut tris, v * 0.5, v);
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
                let (a, b2, c2, d) = (
                    c - u * 0.5 - v * 0.5,
                    c + u * 0.5 - v * 0.5,
                    c + u * 0.5 + v * 0.5,
                    c - u * 0.5 + v * 0.5,
                );
                for p in [a, b2, c2, a, c2, d] {
                    push(&mut tris, p, n);
                }
            }
        }
    }
    let mut lines = Vec::new();
    for l in &scene.lines {
        for w in l.points.windows(2) {
            for p in w {
                lines.push(Vertex {
                    pos: p.to_array(),
                    normal: [0.0; 3],
                    colour: l.colour,
                });
            }
        }
    }
    (tris, lines)
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
        let Ok(r) = Renderer::new(160, 120) else {
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
        };
        let img = r.render(&scene, &Camera::default());
        assert_eq!(img.len(), 160 * 120 * 4);
        let mid = px(&img, 160, 80, 60);
        assert!(mid[0] > 150 && mid[1] < 80 && mid[2] < 80, "the box: {mid:?}");
        let corner = px(&img, 160, 2, 2);
        assert!(corner[0] > 200 && corner[2] > 150, "the sky: {corner:?}");
        // the line, below the box
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
    }
}
