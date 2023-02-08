// TODO: Module-level docs for buffer API

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};
use ash::vk;
use crate::{Device, Error};

use gpu_allocator::{MemoryLocation, vulkan as vk_alloc};

use anyhow::Result;
use gpu_allocator::vulkan::AllocationScheme;

#[derive(Derivative)]
#[derivative(Debug)]
pub struct Buffer {
    #[derivative(Debug="ignore")]
    device: Arc<Device>,
    #[derivative(Debug="ignore")]
    allocator: Arc<Mutex<vk_alloc::Allocator>>,
    memory: vk_alloc::Allocation,
    pub(crate) pointer: Option<NonNull<c_void>>,
    pub handle: vk::Buffer,
    pub size: vk::DeviceSize,
}

#[derive(Debug)]
pub struct BufferView {
    pub(crate) handle: vk::Buffer,
    pub(crate) pointer: Option<NonNull<c_void>>,
    pub offset: vk::DeviceSize,
    pub size: vk::DeviceSize,
}

impl Buffer {
    pub fn new(device: Arc<Device>, allocator: Arc<Mutex<vk_alloc::Allocator>>, size: vk::DeviceSize, usage: vk::BufferUsageFlags, location: MemoryLocation) -> Result<Self> {
        let handle = unsafe {
            device.create_buffer(&vk::BufferCreateInfo {
                s_type: vk::StructureType::BUFFER_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: vk::BufferCreateFlags::empty(),
                size,
                usage,
                sharing_mode: vk::SharingMode::CONCURRENT,
                queue_family_index_count: device.queue_families.len() as u32,
                p_queue_family_indices: device.queue_families.as_ptr(),
            }, None)?
        };

        let requirements = unsafe { device.get_buffer_memory_requirements(handle) };
        let mut alloc = allocator.lock().or_else(|_| Err(anyhow::Error::from(Error::PoisonError)))?;
        let memory = alloc.allocate(&vk_alloc::AllocationCreateDesc {
            name: "buffer",
            requirements,
            location,
            linear: true,
            allocation_scheme: AllocationScheme::GpuAllocatorManaged,
        })?;

        unsafe { device.bind_buffer_memory(handle, memory.memory(), memory.offset())? };

        Ok(Self {
            device,
            allocator: allocator.clone(),
            pointer: memory.mapped_ptr(),
            memory,
            handle,
            size,
        })
    }

    pub fn new_device_local(device: Arc<Device>, allocator: Arc<Mutex<vk_alloc::Allocator>>, size: vk::DeviceSize, usage: vk::BufferUsageFlags) -> Result<Self> {
        Self::new(device, allocator, size, usage, MemoryLocation::GpuOnly)
    }

    pub fn view(&self, offset: vk::DeviceSize, size: vk::DeviceSize) -> Result<BufferView> {
        return if offset + size >= self.size {
            Err(anyhow::Error::from(Error::BufferViewOutOfRange))
        } else {
            Ok(BufferView {
                handle: self.handle,
                offset,
                pointer: unsafe { self.pointer.map(|p| NonNull::new(p.as_ptr().offset(offset as isize)).unwrap() ) },
                size,
            })
        }
    }

    pub fn view_full(&self) -> BufferView {
        BufferView {
            handle: self.handle,
            pointer: self.pointer,
            offset: 0,
            size: self.size,
        }
    }

    pub fn is_mapped(&self) -> bool {
        self.pointer.is_some()
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        let mut alloc = self.allocator.lock().unwrap();
        let memory = std::mem::take(&mut self.memory);
        alloc.free(memory).unwrap();
        unsafe { self.device.destroy_buffer(self.handle, None); }
    }
}

impl BufferView {
    pub fn mapped_slice<T>(&mut self) -> Result<&mut [T]> {
        if let Some(pointer) = self.pointer {
            Ok(unsafe { std::slice::from_raw_parts_mut(pointer.cast::<T>().as_ptr(),  self.size as usize / std::mem::size_of::<T>()) })
        } else {
            Err(anyhow::Error::from(Error::UnmappableBuffer))
        }
    }
}