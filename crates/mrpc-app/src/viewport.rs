//! 3D viewport: orbit camera, CPU scene description and the wgpu callback
//! that renders it inside the egui surface (Blender-style dark viewport).
//!
//! The egui render pass has no depth buffer, so instances are CPU-sorted
//! back-to-front (opaque first, translucent last) — correct enough for a
//! single-car preview and simple to keep in sync with egui.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use egui_wgpu::wgpu;
use egui_wgpu::{CallbackTrait, ScreenDescriptor};
use egui::epaint::PaintCallbackInfo;
use glam::{Mat4, Vec3};

/// Blender-style orbit camera.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub target: Vec3,
}

impl Default for Camera {
    fn default() -> Self {
        Self { yaw: -0.9, pitch: 0.35, dist: 34.0, target: Vec3::new(0.0, 2.0, 0.0) }
    }
}

impl Camera {
    pub fn eye(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.target + Vec3::new(cy * cp, sp, sy * cp) * self.dist
    }

    pub fn view(&self) -> Mat4 {
        glam::camera::rh::view::look_at_mat4(self.eye(), self.target, Vec3::Y)
    }

    pub fn perspective(&self, aspect: f32) -> Mat4 {
        // wgpu uses the Vulkan NDC convention (Y-down, Z in [0, 1]).
        glam::camera::rh::proj::vulkan::perspective(50f32.to_radians(), aspect.max(0.05), 0.1, 400.0)
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw += dx * 0.008;
        self.pitch = (self.pitch + dy * 0.008).clamp(-1.45, 1.45);
    }

    pub fn zoom(&mut self, dy: f32) {
        self.dist = (self.dist * (1.0 + dy * 0.001)).clamp(2.0, 200.0);
    }

    pub fn pan(&mut self, dx: f32, dy: f32) {
        let (_sy, cy) = self.yaw.sin_cos();
        let right = Vec3::new(-(self.yaw.sin()), 0.0, cy);
        let scale = self.dist * 0.0016;
        self.target += right * (-dx * scale) + Vec3::Y * (dy * scale);
    }
}

/// CPU-side vertex (position, normal, uv) matching the shader layout.
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct ViewportVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

impl ViewportVertex {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ViewportVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 12,
                shader_location: 1,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 24,
                shader_location: 2,
            },
        ],
    };
}

/// Per-instance data (model matrix, tint, flags).
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct InstanceData {
    pub model: [[f32; 4]; 4],
    pub tint: [f32; 4],
    /// bit 0 emissive, bit 1 selected, bit 2 textured.
    pub flags: u32,
    pub _pad: [u32; 3],
}

impl InstanceData {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<InstanceData>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 0,
                shader_location: 3,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 16,
                shader_location: 4,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 32,
                shader_location: 5,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 48,
                shader_location: 6,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 64,
                shader_location: 7,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint32,
                offset: 80,
                shader_location: 8,
            },
        ],
    };
}

/// One CPU mesh (a material group of an OBJ part).
#[derive(Debug, Clone)]
pub struct CpuMesh {
    pub vertices: Vec<mrpc_core::obj::Vertex>,
    pub indices: Vec<u32>,
    pub tint: [f32; 3],
    pub alpha: f32,
    pub textured: bool,
    pub emissive: bool,
    pub selected: bool,
    /// Centroid in model units for depth sorting.
    pub centroid: [f32; 3],
}

/// An instance reference: which mesh, where, with what door offset.
#[derive(Debug, Clone, Copy)]
pub struct Instance {
    pub mesh: usize,
    /// Placement + door offset in model units (1/16 block).
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub dx: f64,
    pub dz: f64,
}

impl Instance {
    pub fn new(mesh: usize, x: f64, y: f64, z: f64, dx: f64, dz: f64) -> Instance {
        Instance { mesh, x, y, z, dx, dz }
    }
}

/// Uniform block (view-proj, eye, light).
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    light_dir: [f32; 4],
}

const MESH_SHADER: &str = r#"
struct Globals {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    light_dir: vec4<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var tex_sampler: sampler;
@group(0) @binding(2) var tex: texture_2d<f32>;

struct VertexIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) m0: vec4<f32>,
    @location(4) m1: vec4<f32>,
    @location(5) m2: vec4<f32>,
    @location(6) m3: vec4<f32>,
    @location(7) tint: vec4<f32>,
    @location(8) flags: u32,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tint: vec4<f32>,
    @location(4) @interpolate(flat) flags: u32,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    let model = mat4x4<f32>(in.m0, in.m1, in.m2, in.m3);
    // Model units (1/16 block) → blocks.
    let world_pos = (model * vec4<f32>(in.pos, 1.0)) * (1.0 / 16.0);
    var out: VertexOut;
    out.world = world_pos.xyz;
    out.clip = globals.view_proj * vec4<f32>(world_pos.xyz, 1.0);
    out.normal = normalize((model * vec4<f32>(in.normal, 0.0)).xyz);
    out.uv = in.uv;
    out.tint = in.tint;
    out.flags = in.flags;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    var n = normalize(in.normal);
    if (dot(n, globals.eye.xyz - in.world) < 0.0) {
        n = -n; // double-sided panels
    }
    let l = normalize(-globals.light_dir.xyz);
    let view_dir = normalize(globals.eye.xyz - in.world);
    let half_v = normalize(l + view_dir);

    let emissive = (in.flags & 1u) != 0u;
    let selected = (in.flags & 2u) != 0u;
    let textured = (in.flags & 4u) != 0u;

    var base = in.tint.rgb;
    if (textured) {
        base = base * textureSample(tex, tex_sampler, in.uv).rgb;
    }
    if (selected) {
        // Magenta selection rim, Blender style.
        base = mix(base, vec3<f32>(0.95, 0.45, 0.85), 0.55);
    }

    if (emissive || selected) {
        return vec4<f32>(base, in.tint.a);
    }
    let ambient = 0.42;
    let diffuse = max(dot(n, l), 0.0) * 0.62;
    let specular = pow(max(dot(n, half_v), 0.0), 40.0) * 0.35;
    let lit = base * (ambient + diffuse) + vec3<f32>(specular);
    return vec4<f32>(lit, in.tint.a);
}
"#;

const LINE_SHADER: &str = r#"
struct Globals {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    light_dir: vec4<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    var out: LineOut;
    out.clip = globals.view_proj * vec4<f32>(pos, 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: LineOut) -> @location(0) vec4<f32> {
    return in.color;
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LineVertex {
    pos: [f32; 3],
    color: [f32; 4],
}

/// CPU scene handed to the render callback each frame.
pub struct SceneCpu {
    pub meshes: Vec<CpuMesh>,
    pub instances: Vec<Instance>,
    pub grid: bool,
}

/// The shared, thread-safe viewport state (written by the UI thread, read by
/// the render callback).
pub struct ViewportShared {
    pub camera: Mutex<Camera>,
    pub scene: Mutex<SceneCpu>,
    /// Vehicle texture RGBA8, or `None` for the white fallback.
    pub texture: Mutex<Option<(Vec<u8>, u32, u32)>>,
    pub flip_texture_v: Mutex<bool>,
    /// egui surface format (from eframe's render state).
    pub target_format: wgpu::TextureFormat,
    /// Aspect ratio of the viewport rect (updated each frame).
    pub aspect: Mutex<f32>,
    /// Whether the GPU paint callback has run at least once (startup
    /// diagnostics: proves the render pass was reached).
    first_prepare: AtomicBool,
    gpu: Mutex<Option<GpuScene>>,
}

impl ViewportShared {
    pub fn new(target_format: wgpu::TextureFormat) -> Arc<ViewportShared> {
        Arc::new(ViewportShared {
            camera: Mutex::new(Camera::default()),
            scene: Mutex::new(SceneCpu {
                meshes: Vec::new(),
                instances: Vec::new(),
                grid: true,
            }),
            texture: Mutex::new(None),
            flip_texture_v: Mutex::new(false),
            target_format,
            aspect: Mutex::new(16.0 / 9.0),
            first_prepare: AtomicBool::new(false),
            gpu: Mutex::new(None),
        })
    }

    /// Force GPU resources to rebuild (model or texture changed).
    pub fn invalidate(&self) {
        *self.gpu.lock().unwrap() = None;
    }
}

struct GpuScene {
    mesh_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    uniform_buf: wgpu::Buffer,
    instance_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    mesh_ranges: Vec<(u32, u32)>,
    line_buf: wgpu::Buffer,
    line_count: u32,
    instance_count: usize,
    signature: (usize, usize, usize),
    tex_size: Option<(u32, u32)>,
}

/// The egui paint callback for the 3D viewport.
pub struct ViewportCallback {
    pub shared: Arc<ViewportShared>,
}

impl ViewportCallback {
    pub fn paint(rect: egui::Rect, shared: Arc<ViewportShared>, painter: &egui::Painter) {
        let callback = egui_wgpu::Callback::new_paint_callback(rect, ViewportCallback { shared });
        painter.add(callback);
    }
}

impl CallbackTrait for ViewportCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        _resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if !self
            .shared
            .first_prepare
            .swap(true, Ordering::SeqCst)
        {
            crate::logging::set_stage(crate::logging::Stage::FirstPaint);
            log::info!("viewport: first GPU paint pass (building pipelines)");
        }
        let aspect = *self.shared.aspect.lock().unwrap();
        let camera = *self.shared.camera.lock().unwrap();
        let view_proj = camera.perspective(aspect) * camera.view();
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            eye: [camera.eye().x, camera.eye().y, camera.eye().z, 1.0],
            light_dir: [-0.4, -0.8, -0.35, 0.0],
        };
        let scene_guard = self.shared.scene.lock().unwrap();
        let scene = &*scene_guard;

        // Rebuild GPU buffers when the CPU scene signature changed.
        let signature = (
            scene.meshes.len(),
            scene.instances.len(),
            scene.meshes.iter().map(|m| m.vertices.len() + m.indices.len()).sum::<usize>(),
        );
        {
            let mut gpu = self.shared.gpu.lock().unwrap();
            let needs_rebuild = gpu.as_ref().map(|g| g.signature) != Some(signature)
                || self.shared.texture.lock().unwrap().as_ref().map(|t| (t.1, t.2))
                    != gpu.as_ref().and_then(|g| g.tex_size);
            if needs_rebuild {
                *gpu = None;
                *gpu = Some(build_gpu_scene(device, queue, &self.shared, &globals));
            }
            if let Some(g) = gpu.as_mut() {
                queue.write_buffer(&g.uniform_buf, 0, bytemuck::bytes_of(&globals));

                // Depth-sort instances: opaque first, then back-to-front.
                let eye = camera.eye();
                let mut order: Vec<usize> = (0..scene.instances.len()).collect();
                order.sort_by(|&a, &b| {
                    let ia = &scene.instances[a];
                    let ib = &scene.instances[b];
                    let ma = &scene.meshes[ia.mesh];
                    let mb = &scene.meshes[ib.mesh];
                    let key = |inst: &Instance, m: &CpuMesh| -> (u32, f32) {
                        let p = Vec3::new(
                            (inst.x + inst.dx) as f32 / 16.0 + m.centroid[0] / 16.0,
                            inst.y as f32 / 16.0 + m.centroid[1] / 16.0,
                            (inst.z + inst.dz) as f32 / 16.0 + m.centroid[2] / 16.0,
                        );
                        ((m.alpha < 0.999) as u32, (p - eye).length_squared())
                    };
                    let ka = key(ia, ma);
                    let kb = key(ib, mb);
                    ka.0.cmp(&kb.0).then(ka.1.partial_cmp(&kb.1).unwrap_or(std::cmp::Ordering::Equal))
                });

                let mut instance_data: Vec<InstanceData> = Vec::with_capacity(order.len());
                for &i in &order {
                    let inst = &scene.instances[i];
                    let mesh = &scene.meshes[inst.mesh];
                    let model = Mat4::from_translation(Vec3::new(
                        (inst.x + inst.dx) as f32,
                        inst.y as f32,
                        (inst.z + inst.dz) as f32,
                    ));
                    let mut tint =
                        [mesh.tint[0], mesh.tint[1], mesh.tint[2], mesh.alpha.clamp(0.05, 1.0)];
                    if mesh.selected {
                        tint = [0.95, 0.45, 0.85, 1.0];
                    }
                    let flags = (mesh.emissive as u32)
                        | ((mesh.selected as u32) << 1)
                        | ((mesh.textured as u32) << 2);
                    instance_data.push(InstanceData {
                        model: model.to_cols_array_2d(),
                        tint,
                        flags,
                        _pad: [0; 3],
                    });
                }
                if !instance_data.is_empty() {
                    queue.write_buffer(&g.instance_buf, 0, bytemuck::cast_slice(&instance_data));
                }
                g.instance_count = instance_data.len();
            }
        }
        Vec::new()
    }

    fn paint(
        &self,
        info: PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        _resources: &egui_wgpu::CallbackResources,
    ) {
        let gpu = self.shared.gpu.lock().unwrap();
        let Some(g) = gpu.as_ref() else { return };
        if g.instance_count > 0 {
            render_pass.set_pipeline(&g.mesh_pipeline);
            render_pass.set_bind_group(0, &g.bind_group, &[]);
            render_pass.set_vertex_buffer(0, g.vertex_buf.slice(..));
            render_pass.set_vertex_buffer(1, g.instance_buf.slice(..));
            render_pass.set_index_buffer(g.index_buf.slice(..), wgpu::IndexFormat::Uint32);
            for (offset, count) in &g.mesh_ranges {
                render_pass.draw_indexed(*offset..(*offset + *count), 0, 0..g.instance_count as u32);
            }
        }
        if self.shared.scene.lock().unwrap().grid && g.line_count > 0 {
            render_pass.set_pipeline(&g.line_pipeline);
            render_pass.set_bind_group(0, &g.bind_group, &[]);
            render_pass.set_vertex_buffer(0, g.line_buf.slice(..));
            render_pass.draw(0..g.line_count, 0..1);
        }
        let _ = info;
    }
}

/// Extra state kept alongside the pipeline for rebuild detection.

fn build_gpu_scene(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    shared: &ViewportShared,
    globals: &Globals,
) -> GpuScene {
    use wgpu::util::DeviceExt;

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("mrpc.viewport.shader"),
        source: wgpu::ShaderSource::Wgsl(MESH_SHADER.into()),
    });
    let line_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("mrpc.grid.shader"),
        source: wgpu::ShaderSource::Wgsl(LINE_SHADER.into()),
    });

    let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("mrpc.globals"),
        contents: bytemuck::bytes_of(globals),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    // Texture: vehicle texture or 1×1 white fallback.
    let texture_data = shared.texture.lock().unwrap().clone();
    let (tex_view, tex_size) = match &texture_data {
        Some((data, w, h)) => {
            let extent = wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 };
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("mrpc.vehicle_tex"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                wgpu::wgt::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * *w),
                    rows_per_image: Some(*h),
                },
                extent,
            );
            (tex.create_view(&wgpu::TextureViewDescriptor::default()), Some((*w, *h)))
        }
        None => {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("mrpc.white_tex"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                wgpu::wgt::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &[255, 255, 255, 255],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            );
            (tex.create_view(&wgpu::TextureViewDescriptor::default()), None)
        }
    };

    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("mrpc.bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("mrpc.bg"),
        layout: &bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&tex_view) },
        ],
    });

    // Merged vertex/index buffers.
    let mut all_vertices: Vec<ViewportVertex> = Vec::new();
    let mut all_indices: Vec<u32> = Vec::new();
    let mut mesh_ranges: Vec<(u32, u32)> = Vec::new();
    let flip_v = *shared.flip_texture_v.lock().unwrap();
    let scene_guard = shared.scene.lock().unwrap();
    for mesh in &scene_guard.meshes {
        let base = all_vertices.len() as u32;
        for v in &mesh.vertices {
            all_vertices.push(ViewportVertex {
                pos: v.pos,
                normal: v.normal,
                uv: [v.uv[0], if flip_v { 1.0 - v.uv[1] } else { v.uv[1] }],
            });
        }
        let start = all_indices.len() as u32;
        for idx in &mesh.indices {
            all_indices.push(base + *idx);
        }
        mesh_ranges.push((start, mesh.indices.len() as u32));
    }

    let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("mrpc.vertices"),
        contents: bytemuck::cast_slice(&all_vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let index_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("mrpc.indices"),
        contents: bytemuck::cast_slice(&all_indices),
        usage: wgpu::BufferUsages::INDEX,
    });

    let instance_capacity = 8192;
    let instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mrpc.instances"),
        size: (instance_capacity * std::mem::size_of::<InstanceData>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // Grid lines.
    let mut lines: Vec<LineVertex> = Vec::new();
    let major = [0.35, 0.35, 0.38, 1.0];
    let minor = [0.24, 0.24, 0.27, 1.0];
    let axis_x = [0.75, 0.35, 0.35, 1.0];
    let axis_z = [0.35, 0.55, 0.8, 1.0];
    for i in -14..=14i32 {
        let c = if i % 4 == 0 { major } else { minor };
        lines.push(LineVertex { pos: [i as f32, 0.0, -13.0], color: c });
        lines.push(LineVertex { pos: [i as f32, 0.0, 13.0], color: c });
    }
    for j in -13..=13i32 {
        let c = if j % 4 == 0 { major } else { minor };
        lines.push(LineVertex { pos: [-14.0, 0.0, j as f32], color: c });
        lines.push(LineVertex { pos: [14.0, 0.0, j as f32], color: c });
    }
    lines.push(LineVertex { pos: [0.0, 0.02, 0.0], color: axis_x });
    lines.push(LineVertex { pos: [12.0, 0.02, 0.0], color: axis_x });
    lines.push(LineVertex { pos: [0.0, 0.02, 0.0], color: axis_z });
    lines.push(LineVertex { pos: [0.0, 0.02, 12.0], color: axis_z });
    let line_count = lines.len() as u32;
    let line_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("mrpc.grid"),
        contents: bytemuck::cast_slice(&lines),
        usage: wgpu::BufferUsages::VERTEX,
    });

    let target = shared.target_format;
    let mesh_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("mrpc.mesh_pipeline"),
        layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mrpc.mesh_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        })),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(ViewportVertex::LAYOUT), Some(InstanceData::LAYOUT)],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let line_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("mrpc.line_layout"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let line_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("mrpc.line_pipeline"),
        layout: Some(&line_layout),
        vertex: wgpu::VertexState {
            module: &line_shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<LineVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: 12,
                        shader_location: 1,
                    },
                ],
            })],
        },
        fragment: Some(wgpu::FragmentState {
            module: &line_shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::LineList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let signature = (
        scene_guard.meshes.len(),
        scene_guard.instances.len(),
        scene_guard.meshes.iter().map(|m| m.vertices.len() + m.indices.len()).sum::<usize>(),
    );
    let instance_count = scene_guard.instances.len();
    drop(scene_guard);

    GpuScene {
        mesh_pipeline,
        line_pipeline,
        uniform_buf,
        instance_buf,
        bind_group,
        vertex_buf,
        index_buf,
        mesh_ranges,
        line_buf,
        line_count,
        instance_count,
        signature,
        tex_size,
    }
}
