use lru::LruCache;
use naga_agal::{SamplerOverride, VertexAttributeFormat};
use ruffle_render::backend::ShaderModule;
use std::{
    borrow::Cow,
    cell::{RefCell, RefMut},
    num::NonZeroUsize,
};
use wgpu::SamplerBindingType;

use super::{
    current_pipeline::{
        BoundTextureData, SAMPLER_CLAMP_LINEAR, SAMPLER_CLAMP_NEAREST,
        SAMPLER_CLAMP_U_REPEAT_V_LINEAR, SAMPLER_CLAMP_U_REPEAT_V_NEAREST, SAMPLER_REPEAT_LINEAR,
        SAMPLER_REPEAT_NEAREST, SAMPLER_REPEAT_U_CLAMP_V_LINEAR, SAMPLER_REPEAT_U_CLAMP_V_NEAREST,
        TEXTURE_START_BIND_INDEX,
    },
    MAX_VERTEX_ATTRIBUTES,
};

use crate::descriptors::Descriptors;

pub struct ShaderPairAgal {
    vertex_bytecode: Vec<u8>,
    fragment_bytecode: Vec<u8>,
    // Caches compiled wgpu shader modules. The cache key represents all of the data
    // that we need to pass to `naga_agal::agal_to_naga` to compile a shader.
    compiled: RefCell<LruCache<ShaderCompileData, CompiledShaderProgram>>,
}

impl ShaderModule for ShaderPairAgal {}

pub struct CompiledShaderProgram {
    pub vertex_module: wgpu::ShaderModule,
    pub fragment_module: wgpu::ShaderModule,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

impl ShaderPairAgal {
    pub fn new(vertex_bytecode: Vec<u8>, fragment_bytecode: Vec<u8>) -> Self {
        Self {
            vertex_bytecode,
            fragment_bytecode,
            // TODO - figure out a good size for this cache.
            compiled: RefCell::new(LruCache::new(NonZeroUsize::new(2).unwrap())),
        }
    }

    pub fn compile(
        &self,
        descriptors: &Descriptors,
        data: ShaderCompileData,
    ) -> RefMut<'_, CompiledShaderProgram> {
        let compiled = self.compiled.borrow_mut();
        RefMut::map(compiled, |compiled| {
            // TODO: Figure out a way to avoid the clone when we have a cache hit
            compiled.get_or_insert_mut(data.clone(), || {
                let vertex_naga_module = naga_agal::agal_to_naga(
                    &self.vertex_bytecode,
                    &data.vertex_attributes,
                    &data.sampler_overrides,
                )
                .unwrap();
                let vertex_module =
                    descriptors
                        .device
                        .create_shader_module(wgpu::ShaderModuleDescriptor {
                            label: Some("AGAL vertex shader"),
                            source: wgpu::ShaderSource::Naga(Cow::Owned(vertex_naga_module)),
                        });

                let fragment_naga_module = naga_agal::agal_to_naga(
                    &self.fragment_bytecode,
                    &data.vertex_attributes,
                    &data.sampler_overrides,
                )
                .unwrap();
                let fragment_module =
                    descriptors
                        .device
                        .create_shader_module(wgpu::ShaderModuleDescriptor {
                            label: Some("AGAL fragment shader"),
                            source: wgpu::ShaderSource::Naga(Cow::Owned(fragment_naga_module)),
                        });

                let mut layout_entries = vec![
                    // Vertex shader program constants
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // Fragment shader program constants
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // One sampler per filter/wrapping combination - see BitmapFilters
                    // An AGAL shader can use any of these samplers, so
                    // we need to bind them all.
                    wgpu::BindGroupLayoutEntry {
                        binding: SAMPLER_REPEAT_LINEAR,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: SAMPLER_REPEAT_NEAREST,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: SAMPLER_CLAMP_LINEAR,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: SAMPLER_CLAMP_NEAREST,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: SAMPLER_CLAMP_U_REPEAT_V_LINEAR,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: SAMPLER_CLAMP_U_REPEAT_V_NEAREST,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: SAMPLER_REPEAT_U_CLAMP_V_LINEAR,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: SAMPLER_REPEAT_U_CLAMP_V_NEAREST,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(SamplerBindingType::Filtering),
                        count: None,
                    },
                ];

                for (i, bound_texture) in data.bound_textures.iter().enumerate() {
                    if let Some(bound_texture) = bound_texture {
                        let dimension = if bound_texture.cube {
                            wgpu::TextureViewDimension::Cube
                        } else {
                            wgpu::TextureViewDimension::D2
                        };
                        layout_entries.push(wgpu::BindGroupLayoutEntry {
                            binding: TEXTURE_START_BIND_INDEX + i as u32,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: dimension,
                                multisampled: false,
                            },
                            count: None,
                        });
                    }
                }

                let globals_layout_label = create_debug_label!("Globals bind group layout");
                let bind_group_layout =
                    descriptors
                        .device
                        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                            label: globals_layout_label.as_deref(),
                            entries: &layout_entries,
                        });

                CompiledShaderProgram {
                    vertex_module,
                    fragment_module,
                    bind_group_layout,
                }
            })
        })
    }
}

#[derive(Hash, Eq, PartialEq, Clone)]
pub struct ShaderCompileData {
    pub sampler_overrides: [Option<SamplerOverride>; 8],
    pub vertex_attributes: [Option<VertexAttributeFormat>; MAX_VERTEX_ATTRIBUTES],
    pub bound_textures: [Option<BoundTextureData>; 8],
}
