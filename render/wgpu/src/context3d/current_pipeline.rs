use naga::valid::{Capabilities, ValidationFlags, Validator};
use naga_agal::{Filter, SamplerOverride, Wrapping};
use ruffle_render::backend::{
    Context3DTextureFilter, Context3DTriangleFace, Context3DVertexBufferFormat, Context3DWrapMode,
    Texture,
};

use wgpu::{
    BindGroupEntry, BindingResource, BufferDescriptor, BufferUsages, FrontFace, TextureView,
};
use wgpu::{Buffer, DepthStencilState, StencilFaceState};
use wgpu::{ColorTargetState, RenderPipelineDescriptor, TextureFormat, VertexState};

use std::cell::Cell;
use std::hash::{Hash, Hasher};
use std::num::NonZeroU64;
use std::rc::Rc;

use crate::context3d::shader_pair::ShaderCompileData;
use crate::context3d::VertexBufferWrapper;
use crate::descriptors::Descriptors;

use super::{ShaderPairAgal, VertexAttributeInfo, MAX_VERTEX_ATTRIBUTES};

const AGAL_NUM_VERTEX_CONSTANTS: u64 = 128;
const AGAL_NUM_FRAGMENT_CONSTANTS: u64 = 28;
pub(super) const AGAL_FLOATS_PER_REGISTER: u64 = 4;

const VERTEX_SHADER_UNIFORMS_BUFFER_SIZE: u64 =
    AGAL_NUM_VERTEX_CONSTANTS * AGAL_FLOATS_PER_REGISTER * std::mem::size_of::<f32>() as u64;
const FRAGMENT_SHADER_UNIFORMS_BUFFER_SIZE: u64 =
    AGAL_NUM_FRAGMENT_CONSTANTS * AGAL_FLOATS_PER_REGISTER * std::mem::size_of::<f32>() as u64;

pub(super) const SAMPLER_REPEAT_LINEAR: u32 = 2;
pub(super) const SAMPLER_REPEAT_NEAREST: u32 = 3;
pub(super) const SAMPLER_CLAMP_LINEAR: u32 = 4;
pub(super) const SAMPLER_CLAMP_NEAREST: u32 = 5;
pub(super) const SAMPLER_CLAMP_U_REPEAT_V_LINEAR: u32 = 6;
pub(super) const SAMPLER_CLAMP_U_REPEAT_V_NEAREST: u32 = 7;
pub(super) const SAMPLER_REPEAT_U_CLAMP_V_LINEAR: u32 = 8;
pub(super) const SAMPLER_REPEAT_U_CLAMP_V_NEAREST: u32 = 9;

pub(super) const TEXTURE_START_BIND_INDEX: u32 = 10;

// The flash Context3D API is similar to OpenGL - it has many methods
// which modify the current state (`setVertexBufferAt`, `setCulling`, etc.)
// These methods can be called at any time.
//
// In WGPU, this state is associated by a `RenderPipeline` object,
// which needs to be rebuilt whenever the state changes.
//
// To match up these APIs, we store the current state in `CurentPipeline`.
// Whenever a state-changing `Context3DCommand` is executed, we mark the `CurrentPipeline`
// as dirty. When a `wgpu::RenderPipeline` is actually needed by `drawTriangles`,
// we build a new `wgpu::RenderPipeline` from the `CurrentPipeline` state (if it's dirty).
//
// The `CurrentPipeline` state (including the compiled `wgpu::RenderPipeline`) is stored
// in `WgpuContext3D`, and is re-used across calls to `present`. Due to lifetime issues,
// we don't actually store the `wgpu::RenderPipeline` in `CurrentPipeline` - it's
// instead stored in `WgpuContext3D`.
pub struct CurrentPipeline {
    shaders: Option<Rc<ShaderPairAgal>>,

    culling: Context3DTriangleFace,

    bound_textures: [Option<BoundTextureData>; 8],

    pub vertex_shader_uniforms: Buffer,
    pub fragment_shader_uniforms: Buffer,

    has_depth_texture: bool,

    color_mask: wgpu::ColorWrites,

    depth_mask: bool,
    pass_compare_mode: wgpu::CompareFunction,

    color_component: wgpu::BlendComponent,
    alpha_component: wgpu::BlendComponent,

    sample_count: u32,

    target_format: TextureFormat,

    dirty: Cell<bool>,

    sampler_override: [Option<SamplerOverride>; 8],
}

#[derive(Clone)]
pub struct BoundTextureData {
    /// This is used to allow us to remove a bound texture when
    /// it's used with `setRenderToTexture`. The actual shader binding
    /// uses `view`
    pub id: Rc<dyn Texture>,
    pub view: Rc<TextureView>,
    pub cube: bool,
}

impl Hash for BoundTextureData {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // We can't hash 'view', but we can hash the pointer of the 'Rc<dyn Texture>',
        // which is unique to the TextureView
        let BoundTextureData { id, cube, view: _ } = self;
        (Rc::as_ptr(id) as *const ()).hash(state);
        cube.hash(state);
    }
}

impl PartialEq for BoundTextureData {
    fn eq(&self, other: &Self) -> bool {
        let BoundTextureData { id, cube, view: _ } = self;
        let BoundTextureData {
            id: other_id,
            cube: other_cube,
            view: _,
        } = other;
        std::ptr::eq(
            Rc::as_ptr(id) as *const (),
            Rc::as_ptr(other_id) as *const (),
        ) && cube == other_cube
    }
}
impl Eq for BoundTextureData {}

impl CurrentPipeline {
    pub fn new(descriptors: &Descriptors) -> Self {
        let vertex_shader_uniforms = descriptors.device.create_buffer(&BufferDescriptor {
            label: Some("Vertex shader uniforms"),
            size: VERTEX_SHADER_UNIFORMS_BUFFER_SIZE,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let fragment_shader_uniforms = descriptors.device.create_buffer(&BufferDescriptor {
            label: Some("Fragment shader uniforms"),
            size: FRAGMENT_SHADER_UNIFORMS_BUFFER_SIZE,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        CurrentPipeline {
            shaders: None,
            bound_textures: std::array::from_fn(|_| None),
            vertex_shader_uniforms,
            fragment_shader_uniforms,
            dirty: Cell::new(true),
            culling: Context3DTriangleFace::None,

            has_depth_texture: false,

            color_mask: wgpu::ColorWrites::ALL,

            depth_mask: true,
            pass_compare_mode: wgpu::CompareFunction::LessEqual,
            color_component: wgpu::BlendComponent::REPLACE,
            alpha_component: wgpu::BlendComponent::REPLACE,
            sample_count: 1,

            target_format: TextureFormat::Rgba8Unorm,

            sampler_override: [None; 8],
        }
    }
    pub fn set_shaders(&mut self, shaders: Option<Rc<ShaderPairAgal>>) {
        self.dirty.set(true);
        self.shaders = shaders;
    }

    pub fn update_texture_at(&mut self, index: usize, texture: Option<BoundTextureData>) {
        // FIXME - determine if the texture actually changed
        self.dirty.set(true);
        self.bound_textures[index] = texture;
    }

    pub fn remove_texture(&mut self, texture: &Rc<dyn Texture>) {
        for i in 0..self.bound_textures.len() {
            if let Some(bound_texture) = &self.bound_textures[i] {
                // Ignore the vtable pointer
                if std::ptr::eq(
                    Rc::as_ptr(&bound_texture.id) as *const (),
                    Rc::as_ptr(texture) as *const (),
                ) {
                    self.update_texture_at(i, None);
                }
            }
        }
    }

    pub fn update_vertex_buffer_at(&mut self, _index: usize) {
        // FIXME - check if it's the same, so we can skip rebuilding the pipeline
        self.dirty.set(true);
    }

    pub fn update_color_mask(&mut self, color_mask: wgpu::ColorWrites) {
        if self.color_mask != color_mask {
            self.dirty.set(true);
        }
        self.color_mask = color_mask;
    }

    pub fn update_depth(&mut self, depth_mask: bool, pass_compare_mode: wgpu::CompareFunction) {
        if self.depth_mask != depth_mask || self.pass_compare_mode != pass_compare_mode {
            self.dirty.set(true);
        }
        self.depth_mask = depth_mask;
        self.pass_compare_mode = pass_compare_mode;
    }

    pub fn update_has_depth_texture(&mut self, has_depth_texture: bool) {
        if self.has_depth_texture != has_depth_texture {
            self.dirty.set(true);
            self.has_depth_texture = has_depth_texture;
        }
    }

    pub fn update_sample_count(&mut self, sample_count: u32) {
        if self.sample_count != sample_count {
            self.dirty.set(true);
            self.sample_count = sample_count;
        }
    }

    pub fn update_target_format(&mut self, format: TextureFormat) {
        if self.target_format != format {
            self.dirty.set(true);
            self.target_format = format;
        }
    }

    /// If the pipeline is dirty, recompiles it and returns `Some(freshly_compiled_pipeline`)
    /// Otherwise, returns `None`.
    pub fn rebuild_pipeline(
        &self,
        descriptors: &Descriptors,
        vertex_attributes: &[Option<VertexAttributeInfo>; MAX_VERTEX_ATTRIBUTES],
    ) -> Option<(wgpu::RenderPipeline, wgpu::BindGroup)> {
        if !self.dirty.get() {
            return None;
        }

        self.dirty.set(false);

        let bind_group_label = create_debug_label!("Bind group");

        let mut bind_group_entries = vec![
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &self.vertex_shader_uniforms,
                    offset: 0,
                    size: Some(NonZeroU64::new(VERTEX_SHADER_UNIFORMS_BUFFER_SIZE).unwrap()),
                }),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &self.fragment_shader_uniforms,
                    offset: 0,
                    size: Some(NonZeroU64::new(FRAGMENT_SHADER_UNIFORMS_BUFFER_SIZE).unwrap()),
                }),
            },
            BindGroupEntry {
                binding: SAMPLER_REPEAT_LINEAR,
                resource: BindingResource::Sampler(&descriptors.bitmap_samplers.repeat_linear),
            },
            BindGroupEntry {
                binding: SAMPLER_REPEAT_NEAREST,
                resource: BindingResource::Sampler(&descriptors.bitmap_samplers.repeat_nearest),
            },
            BindGroupEntry {
                binding: SAMPLER_CLAMP_LINEAR,
                resource: BindingResource::Sampler(&descriptors.bitmap_samplers.clamp_linear),
            },
            BindGroupEntry {
                binding: SAMPLER_CLAMP_NEAREST,
                resource: BindingResource::Sampler(&descriptors.bitmap_samplers.clamp_nearest),
            },
            BindGroupEntry {
                binding: SAMPLER_CLAMP_U_REPEAT_V_LINEAR,
                resource: BindingResource::Sampler(
                    &descriptors.bitmap_samplers.clamp_u_repeat_v_linear,
                ),
            },
            BindGroupEntry {
                binding: SAMPLER_CLAMP_U_REPEAT_V_NEAREST,
                resource: BindingResource::Sampler(
                    &descriptors.bitmap_samplers.clamp_u_repeat_v_nearest,
                ),
            },
            BindGroupEntry {
                binding: SAMPLER_REPEAT_U_CLAMP_V_LINEAR,
                resource: BindingResource::Sampler(
                    &descriptors.bitmap_samplers.repeat_u_clamp_v_linear,
                ),
            },
            BindGroupEntry {
                binding: SAMPLER_REPEAT_U_CLAMP_V_NEAREST,
                resource: BindingResource::Sampler(
                    &descriptors.bitmap_samplers.repeat_u_clamp_v_nearest,
                ),
            },
        ];

        for (i, bound_texture) in self.bound_textures.iter().enumerate() {
            if let Some(bound_texture) = bound_texture {
                bind_group_entries.push(BindGroupEntry {
                    binding: TEXTURE_START_BIND_INDEX + i as u32,
                    resource: BindingResource::TextureView(&bound_texture.view),
                });
            }
        }

        let agal_attributes = vertex_attributes.clone().map(|attr| {
            attr.map(|attr| match attr.format {
                Context3DVertexBufferFormat::Float4 => naga_agal::VertexAttributeFormat::Float4,
                Context3DVertexBufferFormat::Float3 => naga_agal::VertexAttributeFormat::Float3,
                Context3DVertexBufferFormat::Float2 => naga_agal::VertexAttributeFormat::Float2,
                Context3DVertexBufferFormat::Float1 => naga_agal::VertexAttributeFormat::Float1,
                Context3DVertexBufferFormat::Bytes4 => naga_agal::VertexAttributeFormat::Bytes4,
            })
        });

        let compiled_shaders = self.shaders.as_ref().expect("Missing shaders!").compile(
            descriptors,
            ShaderCompileData {
                vertex_attributes: agal_attributes,
                sampler_overrides: self.sampler_override,
                bound_textures: self.bound_textures.clone(),
            },
        );

        let pipeline_layout_label = create_debug_label!("Pipeline layout");
        let pipeline_layout =
            descriptors
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: pipeline_layout_label.as_deref(),
                    bind_group_layouts: &[&compiled_shaders.bind_group_layout],
                    push_constant_ranges: &[],
                });

        let bind_group = descriptors
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: bind_group_label.as_deref(),
                layout: &compiled_shaders.bind_group_layout,
                entries: &bind_group_entries,
            });

        struct BufferData {
            buffer: Rc<VertexBufferWrapper>,
            attrs: Vec<wgpu::VertexAttribute>,
            total_size: usize,
        }

        // The user can call Context3D.setVertexBufferAt with a a mixture of vertex buffers.
        // We need to create one 'BufferData' struct for each distinct vertex buffer
        // across all of the calls to 'setVertexBufferAt'. The 'BufferData' keeps track
        // of all of the bound indices associated with that buffer.
        let mut index_per_buffer: Vec<BufferData> = Vec::new();

        for (i, attr) in vertex_attributes.iter().enumerate() {
            if let Some(attr) = attr {
                let (format, entry_size_bytes) = match attr.format {
                    Context3DVertexBufferFormat::Float4 => (
                        wgpu::VertexFormat::Float32x4,
                        4 * std::mem::size_of::<f32>(),
                    ),
                    Context3DVertexBufferFormat::Float3 => (
                        wgpu::VertexFormat::Float32x3,
                        3 * std::mem::size_of::<f32>(),
                    ),
                    Context3DVertexBufferFormat::Float2 => (
                        wgpu::VertexFormat::Float32x2,
                        2 * std::mem::size_of::<f32>(),
                    ),
                    Context3DVertexBufferFormat::Float1 => {
                        (wgpu::VertexFormat::Float32, std::mem::size_of::<f32>())
                    }
                    // AGAL shaders always work with floating-point values, so
                    // we use Unorm8x4 to convert the bytes to floats in the range
                    // [0, 1].
                    Context3DVertexBufferFormat::Bytes4 => (wgpu::VertexFormat::Unorm8x4, 4),
                };

                let buffer_data = index_per_buffer
                    .iter_mut()
                    .find(|data| Rc::ptr_eq(&data.buffer, &attr.buffer));

                let buffer_data = if let Some(buffer_data) = buffer_data {
                    buffer_data
                } else {
                    index_per_buffer.push(BufferData {
                        buffer: attr.buffer.clone(),
                        attrs: Vec::new(),
                        total_size: 0,
                    });
                    index_per_buffer.last_mut().unwrap()
                };

                // FIXME - assert that this matches up with the AS3-supplied offset
                buffer_data.total_size += entry_size_bytes;
                buffer_data.attrs.push(wgpu::VertexAttribute {
                    format,
                    offset: attr.offset_in_32bit_units * 4,
                    shader_location: i as u32,
                })
            }
        }

        let cull_mode = match self.culling {
            Context3DTriangleFace::Back => Some(wgpu::Face::Back),
            Context3DTriangleFace::Front => Some(wgpu::Face::Front),
            Context3DTriangleFace::FrontAndBack => {
                tracing::error!("FrontAndBack culling not supported!");
                None
            }
            Context3DTriangleFace::None => None,
        };

        let depth_stencil = if self.has_depth_texture {
            Some(DepthStencilState {
                format: TextureFormat::Depth24PlusStencil8,
                depth_write_enabled: self.depth_mask,
                depth_compare: self.pass_compare_mode,
                // FIXME - implement this
                stencil: wgpu::StencilState {
                    front: StencilFaceState::IGNORE,
                    back: StencilFaceState::IGNORE,
                    read_mask: !0,
                    write_mask: !0,
                },
                bias: Default::default(),
            })
        } else {
            None
        };

        let wgpu_vertex_buffers = index_per_buffer
            .iter()
            .map(|data| {
                // This value is set when Context3D.createVertexBuffer is called.
                // We may not all of the data associated with a single vertex
                // (e.g. we might have 8 floats per vertex, but only
                // call setVertexBufferAt once to bind the first 4 floats per vertex.
                // However, the total size of the bindings can be at most the total
                // amount of data per vertex. Verify that here
                let data_bytes_per_vertex = (data.buffer.data_32_per_vertex * 4) as u64;
                if data.total_size > data_bytes_per_vertex as usize {
                    panic!("Total size of bound vertex attributes {:?} exceeds data_bytes_per_vertex {:?}", data.total_size,
                    data_bytes_per_vertex);
                }

                let attrs = &data.attrs;
                wgpu::VertexBufferLayout {
                    array_stride: data_bytes_per_vertex,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: attrs,
                }
            })
            .collect::<Vec<_>>();

        let compiled = descriptors
            .device
            .create_render_pipeline(&RenderPipelineDescriptor {
                label: create_debug_label!("RenderPipeline").as_deref(),
                layout: Some(&pipeline_layout),
                vertex: VertexState {
                    module: &compiled_shaders.vertex_module,
                    entry_point: naga_agal::SHADER_ENTRY_POINT,
                    buffers: &wgpu_vertex_buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &compiled_shaders.fragment_module,
                    entry_point: naga_agal::SHADER_ENTRY_POINT,
                    targets: &[Some(ColorTargetState {
                        format: self.target_format,
                        blend: Some(wgpu::BlendState {
                            color: self.color_component,
                            alpha: self.alpha_component,
                        }),
                        write_mask: self.color_mask,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    // Stage3d appears to use clockwise winding:
                    // https://stackoverflow.com/questions/8677498/stage3d-culling-confusion
                    front_face: FrontFace::Cw,
                    cull_mode,
                    ..Default::default()
                },
                depth_stencil,
                multisample: wgpu::MultisampleState {
                    count: self.sample_count,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                multiview: Default::default(),
            });
        Some((compiled, bind_group))
    }

    pub fn set_culling(&mut self, face: Context3DTriangleFace) {
        self.culling = face;
        self.dirty.set(true);
    }

    pub fn update_blend_factors(
        &mut self,
        color_component: wgpu::BlendComponent,
        alpha_component: wgpu::BlendComponent,
    ) {
        if color_component != self.color_component || alpha_component != self.alpha_component {
            self.color_component = color_component;
            self.alpha_component = alpha_component;
            self.dirty.set(true);
        }
    }

    pub(crate) fn update_sampler_state_at(
        &mut self,
        sampler: usize,
        wrap: ruffle_render::backend::Context3DWrapMode,
        filter: ruffle_render::backend::Context3DTextureFilter,
    ) {
        let sampler_override = SamplerOverride {
            wrapping: match wrap {
                Context3DWrapMode::Clamp => Wrapping::Clamp,
                Context3DWrapMode::Repeat => Wrapping::Repeat,
                Context3DWrapMode::ClampURepeatV => Wrapping::ClampURepeatV,
                Context3DWrapMode::RepeatUClampV => Wrapping::RepeatUClampV,
            },
            filter: match filter {
                Context3DTextureFilter::Linear => Filter::Linear,
                Context3DTextureFilter::Nearest => Filter::Nearest,
                _ => unimplemented!(),
            },
            // FIXME - implement this
            mipmap: naga_agal::Mipmap::Disable,
        };
        if self.sampler_override[sampler] != Some(sampler_override) {
            self.dirty.set(true);
            self.sampler_override[sampler] = Some(sampler_override);
        }
    }
}

// This is useful for debugging shader issues
#[allow(dead_code)]
fn to_wgsl(module: &naga::Module) -> String {
    let mut out = String::new();

    let mut validator = Validator::new(ValidationFlags::all(), Capabilities::all());
    let module_info = validator
        .validate(module)
        .unwrap_or_else(|e| panic!("Validation failed: {:#?}", e));

    let mut writer =
        naga::back::wgsl::Writer::new(&mut out, naga::back::wgsl::WriterFlags::EXPLICIT_TYPES);

    writer.write(module, &module_info).expect("Writing failed");
    out
}
