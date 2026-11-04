//! The samplers materials read their pictures with, in groups of their own.
//!
//! A card keeps a sampler slot for every sampler binding of every bind group
//! alive, however few samplers there really are, and DirectX 12 has 2048 of
//! them for everything. So no kept material binds a sampler itself. It names
//! one of these groups, and there's one for each different way of reading its
//! pictures, which is a handful however many models are kept.

use std::collections::HashMap;
use std::sync::Mutex;

/// How one picture is read: its wrap each way and its filter.
pub(super) type Reading = (wgpu::AddressMode, wgpu::AddressMode, wgpu::FilterMode);

/// The reading of a slot with no picture of its own.
pub(super) const PLAIN: Reading = (
    wgpu::AddressMode::Repeat,
    wgpu::AddressMode::Repeat,
    wgpu::FilterMode::Linear,
);

/// One sampler for each reading, one group for each set of five.
pub(super) struct Samplers {
    pub(super) layout: wgpu::BindGroupLayout,
    /// Read the picture behind glass.
    behind: wgpu::Sampler,
    each: Mutex<HashMap<Reading, wgpu::Sampler>>,
    groups: Mutex<HashMap<[Reading; 5], wgpu::BindGroup>>,
}

fn sampler(device: &wgpu::Device, (u, v, filter): Reading) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("mesh-sampler"),
        address_mode_u: u,
        address_mode_v: v,
        mag_filter: filter,
        min_filter: filter,
        mipmap_filter: wgpu::FilterMode::Linear,
        // Sharper at a glancing angle. The card only takes it with every
        // filter linear.
        anisotropy_clamp: if filter == wgpu::FilterMode::Linear {
            16
        } else {
            1
        },
        ..Default::default()
    })
}

impl Samplers {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let entries: Vec<_> = (0..6)
            .map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            })
            .collect();
        Self {
            layout: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("mesh-samplers"),
                entries: &entries,
            }),
            behind: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("mesh-behind"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            each: Mutex::new(HashMap::new()),
            groups: Mutex::new(HashMap::new()),
        }
    }

    /// The group that reads a material's five pictures as `readings` say,
    /// made the first time it's asked for.
    pub(super) fn group(&self, device: &wgpu::Device, readings: [Reading; 5]) -> wgpu::BindGroup {
        let make = || {
            let samplers = readings.map(|r| match self.each.lock() {
                Ok(mut each) => each.entry(r).or_insert_with(|| sampler(device, r)).clone(),
                Err(_) => sampler(device, r),
            });
            let mut entries: Vec<_> = (0u32..)
                .zip(&samplers)
                .map(|(binding, s)| wgpu::BindGroupEntry {
                    binding,
                    resource: wgpu::BindingResource::Sampler(s),
                })
                .collect();
            entries.push(wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(&self.behind),
            });
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mesh-samplers"),
                layout: &self.layout,
                entries: &entries,
            })
        };
        let kept = self
            .groups
            .lock()
            .ok()
            .and_then(|g| g.get(&readings).cloned());
        kept.unwrap_or_else(|| {
            let group = make();
            if let Ok(mut groups) = self.groups.lock() {
                groups.insert(readings, group.clone());
            }
            group
        })
    }
}
